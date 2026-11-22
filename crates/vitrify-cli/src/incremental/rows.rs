//! Which rows a gate, a re-check or a preflight runs.
//!
//! Every selection is a list of Verification rows — each distinct command
//! once, in the order its first unit lists it — and, where the §5 Test
//! command belongs, that command after them:
//!
//! * **an integration gate** runs the merged units' rows, then the rows of
//!   the SDD's dirty Verified units — every dependent of the merged units is
//!   among them, since their inputs changed — then the §5 Test command, the
//!   whole-suite backstop. Rows of clean units are not run: their recorded
//!   checks still hold. An SDD **without** a §5 Test command has no backstop
//!   for coupling its declarations miss, so its gate runs every Verified
//!   unit's rows, as a full re-run;
//! * **a preflight** runs the rows of dirty units only — what replaces
//!   re-running every Verified unit's table at the start of a run;
//! * **a re-check batch** runs the dirty units' rows, then the §5 Test
//!   command.
//!
//! The selections are pure: the caller reads the tables ([`tables`]) and
//! the dirty set (`super::dirty`), and runs what comes back.

use vitrify_core::CoreError;
use vitrify_core::model::{self, Instance};

use crate::gate;

/// One unit's Verification rows, as a selection reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitTable {
    pub unit: String,
    /// The unit's status is Verified.
    pub verified: bool,
    /// Its Verification table's commands, in table order.
    pub rows: Vec<String>,
}

/// Every unit's table of `inst`, in the SDD's unit order.
pub fn tables(inst: &Instance) -> Result<Vec<UnitTable>, CoreError> {
    inst.units
        .iter()
        .map(|u| {
            Ok(UnitTable {
                unit: u.unit.clone(),
                verified: u.meta.get_str_or("status", "Draft") == "Verified",
                rows: gate::verification_table(inst, u)?,
            })
        })
        .collect()
}

/// The SDD's §5 Commands: the first `### Commands` subsection of the core
/// file's `## Technology Decisions` section, up to the next heading of level
/// 3 or less; each the command between the first pair of backticks on its
/// `- Name:` line. A `Commands` heading anywhere else — at another level,
/// under another section, in a unit region of a single-file SDD, inside a
/// fenced block — is not §5, and nothing under it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commands {
    pub bootstrap: Option<String>,
    pub build: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
}

impl Commands {
    /// Read from the core file's text.
    pub fn parse(core_text: &str) -> Commands {
        let mut out = Commands::default();
        for line in commands_lines(core_text) {
            let Some(rest) = line.trim_start().strip_prefix("- ") else {
                continue;
            };
            let Some((name, value)) = rest.split_once(':') else {
                continue;
            };
            let cmd = value
                .split('`')
                .nth(1)
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            match name.trim().to_lowercase().as_str() {
                "bootstrap" => out.bootstrap = cmd,
                "build" => out.build = cmd,
                "test" => out.test = cmd,
                "lint" => out.lint = cmd,
                _ => {}
            }
        }
        out
    }

    /// Read from `inst`'s core file.
    pub fn of(inst: &Instance) -> Result<Commands, CoreError> {
        Ok(Commands::parse(&model::read_text(&inst.file)?))
    }
}

/// A Markdown ATX heading's level and text; `None` for any other line.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim_end()))
}

/// The lines of the §5 Commands subsection: under the first `## Technology
/// Decisions`, the first `### Commands`, up to the next heading of level 3
/// or less. Headings inside fenced blocks are not headings.
fn commands_lines(core_text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    // 0: before §5; 1: in §5; 2: in its Commands; 3: past them
    let mut at = 0;
    for line in core_text.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            if at == 2 {
                out.push(line);
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            if at == 2 {
                out.push(line);
            }
            continue;
        }
        if let Some((level, text)) = heading(line) {
            at = match at {
                0 if level == 2 && text == "Technology Decisions" => 1,
                1 if level <= 2 => 3,
                1 if level == 3 && text == "Commands" => 2,
                2 if level <= 3 => 3,
                other => other,
            };
            if at == 3 {
                break;
            }
            continue;
        }
        if at == 2 {
            out.push(line);
        }
    }
    out
}

/// What to run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The units whose rows are selected, in the order they were taken.
    pub units: Vec<String>,
    /// Their rows, each distinct command once, in first-listed order.
    pub rows: Vec<String>,
    /// The §5 Test command, run after the rows; `None` when the selection
    /// has none.
    pub test: Option<String>,
}

impl Selection {
    /// Take `unit`'s rows, unless it is already taken or has no table.
    fn take(&mut self, tables: &[UnitTable], unit: &str) {
        if self.units.iter().any(|u| u == unit) {
            return;
        }
        let Some(t) = tables.iter().find(|t| t.unit == unit) else {
            return;
        };
        self.units.push(t.unit.clone());
        for row in &t.rows {
            if !self.rows.contains(row) {
                self.rows.push(row.clone());
            }
        }
    }

    /// Whether nothing at all is selected.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.test.is_none()
    }
}

/// Whether `unit` is Verified in `tables`.
fn is_verified(tables: &[UnitTable], unit: &str) -> bool {
    tables.iter().any(|t| t.unit == unit && t.verified)
}

/// An integration gate: the `merged` units' rows; then, with a §5 Test
/// command, the rows of the `dirty` units still Verified, and the command;
/// without one, the rows of every Verified unit (in the SDD's order).
pub fn for_gate(
    tables: &[UnitTable],
    merged: &[String],
    dirty: &[String],
    test: Option<&str>,
) -> Selection {
    let mut sel = Selection::default();
    for unit in merged {
        sel.take(tables, unit);
    }
    match test {
        Some(test) => {
            for unit in dirty {
                if is_verified(tables, unit) {
                    sel.take(tables, unit);
                }
            }
            sel.test = Some(test.to_string());
        }
        None => {
            for t in tables.iter().filter(|t| t.verified) {
                sel.take(tables, &t.unit);
            }
        }
    }
    sel
}

/// A preflight: the rows of the `dirty` units still Verified; no Test
/// command.
pub fn for_preflight(tables: &[UnitTable], dirty: &[String]) -> Selection {
    let mut sel = Selection::default();
    for unit in dirty {
        if is_verified(tables, unit) {
            sel.take(tables, unit);
        }
    }
    sel
}

/// A re-check batch: the rows of the `dirty` units still Verified, then
/// the §5 Test command when the SDD has one.
pub fn for_recheck(tables: &[UnitTable], dirty: &[String], test: Option<&str>) -> Selection {
    let mut sel = for_preflight(tables, dirty);
    sel.test = test.map(str::to_string);
    sel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(unit: &str, verified: bool, rows: &[&str]) -> UnitTable {
        UnitTable {
            unit: unit.into(),
            verified,
            rows: rows.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// U-001 a foundation, U-002 and U-003 Verified, U-004 Ready; U-003
    /// shares a row with U-001.
    fn sdd() -> Vec<UnitTable> {
        vec![
            t("U-001", false, &["test -f a", "check shared"]),
            t("U-002", true, &["test -f b"]),
            t("U-003", true, &["check shared", "test -f c"]),
            t("U-004", false, &["test -f d"]),
        ]
    }

    #[test]
    fn a_gate_runs_the_merged_and_the_dirty_rows_then_the_suite() {
        let sel = for_gate(
            &sdd(),
            &ids(&["U-001"]),
            &ids(&["U-003", "U-004"]),
            Some("make test"),
        );
        assert_eq!(sel.units, ids(&["U-001", "U-003"]));
        // each distinct command once, in first-listed order
        assert_eq!(sel.rows, ids(&["test -f a", "check shared", "test -f c"]));
        assert_eq!(sel.test.as_deref(), Some("make test"));
    }

    #[test]
    fn a_gate_with_nothing_dirty_runs_the_merged_rows_and_the_suite() {
        let sel = for_gate(&sdd(), &ids(&["U-004"]), &[], Some("make test"));
        assert_eq!(sel.units, ids(&["U-004"]));
        assert_eq!(sel.rows, ids(&["test -f d"]));
        assert_eq!(sel.test.as_deref(), Some("make test"));
    }

    #[test]
    fn without_a_test_command_a_gate_runs_every_verified_unit() {
        // nothing dirty, and still the full re-run: no backstop
        let sel = for_gate(&sdd(), &ids(&["U-001"]), &[], None);
        assert_eq!(sel.units, ids(&["U-001", "U-002", "U-003"]));
        assert_eq!(
            sel.rows,
            ids(&["test -f a", "check shared", "test -f b", "test -f c"])
        );
        assert_eq!(sel.test, None);
    }

    #[test]
    fn a_merged_unit_listed_dirty_too_is_taken_once() {
        let sel = for_gate(
            &sdd(),
            &ids(&["U-002"]),
            &ids(&["U-002", "U-003"]),
            Some("t"),
        );
        assert_eq!(sel.units, ids(&["U-002", "U-003"]));
        assert_eq!(sel.rows, ids(&["test -f b", "check shared", "test -f c"]));
    }

    #[test]
    fn a_preflight_runs_dirty_verified_rows_only() {
        let sel = for_preflight(&sdd(), &ids(&["U-003", "U-004", "U-009"]));
        assert_eq!(sel.units, ids(&["U-003"]));
        assert_eq!(sel.rows, ids(&["check shared", "test -f c"]));
        assert_eq!(sel.test, None);
        assert!(for_preflight(&sdd(), &[]).is_empty());
    }

    #[test]
    fn a_recheck_runs_dirty_rows_then_the_suite() {
        let sel = for_recheck(&sdd(), &ids(&["U-003", "U-002"]), Some("t"));
        // in the order the units were given
        assert_eq!(sel.units, ids(&["U-003", "U-002"]));
        assert_eq!(sel.rows, ids(&["check shared", "test -f c", "test -f b"]));
        assert_eq!(sel.test.as_deref(), Some("t"));
        let none = for_recheck(&sdd(), &[], Some("t"));
        assert!(none.rows.is_empty());
        assert!(!none.is_empty());
        assert!(for_recheck(&sdd(), &[], None).is_empty());
    }

    #[test]
    fn commands_parse_the_commands_section() {
        let text = "# x\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `npm ci`\n- Build:     ``\n- Test: `npm test` (whole suite)\n\n## Next\n\n- Lint: `nope`\n";
        let c = Commands::parse(text);
        assert_eq!(c.bootstrap.as_deref(), Some("npm ci"));
        assert_eq!(c.build, None);
        assert_eq!(c.test.as_deref(), Some("npm test"));
        assert_eq!(c.lint, None);
    }

    /// A single-file SDD whose unit region has a `#### Commands` heading of
    /// its own, after the real §5 Commands.
    const SINGLE: &str = "# s\n\n## Ground Rules\n\n### Commands\n\n- Lint: `ground`\n\n## Technology Decisions\n\nT.\n\n### Tools\n\n- Build: `tools`\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `make`\n- Test:      `make test`\n\n```sh\n# Commands\n- Lint: `fenced`\n```\n\n#### Notes\n\n- Lint: `cargo clippy`\n\n### Commands\n\n- Build: `second`\n\n## Work Units\n\n### U-001 — a\n\n#### Commands\n\n- Build: `rm -rf /tmp/x`\n\n# Commands\n\n- Test: `evil`\n";

    #[test]
    fn a_commands_heading_outside_section_5_is_not_read() {
        let c = Commands::parse(SINGLE);
        // the unit region's heading, the top-level one, the one under
        // Ground Rules, and the fenced one run nothing
        assert_eq!(c.build.as_deref(), Some("make"));
        assert_eq!(c.test.as_deref(), Some("make test"));
        assert_eq!(c.bootstrap.as_deref(), Some("true"));
        // a level-4 heading inside the subsection does not end it
        assert_eq!(c.lint.as_deref(), Some("cargo clippy"));
    }

    #[test]
    fn the_first_commands_under_technology_decisions_wins() {
        // the second `### Commands` under §5 is ignored
        assert_eq!(Commands::parse(SINGLE).build.as_deref(), Some("make"));
        // no Technology Decisions section: no commands
        assert_eq!(
            Commands::parse("# x\n\n### Commands\n\n- Build: `make`\n"),
            Commands::default()
        );
        // a Commands heading at level 2 under nothing is not §5 either
        assert_eq!(
            Commands::parse("## Technology Decisions\n\n## Commands\n\n- Build: `make`\n"),
            Commands::default()
        );
    }

    #[test]
    fn every_fixture_and_the_example_parse_their_commands() {
        // every core file of the repository's fixtures with a `### Commands`
        // heading is a well-formed SDD: its §5 Commands are all read
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|e| e == "md") {
                    out.push(p);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let mut files = Vec::new();
        walk(&root, &mut files);
        let mut seen = 0;
        for f in files {
            let text = model::read_text(&f).unwrap();
            if !text.lines().any(|l| l == "### Commands") {
                continue;
            }
            seen += 1;
            let c = Commands::parse(&text);
            assert!(
                c.bootstrap.is_some() && c.build.is_some() && c.test.is_some() && c.lint.is_some(),
                "{}: {c:?}",
                f.display()
            );
        }
        assert!(seen > 20, "{seen} fixtures");
        let calc = model::read_text(&root.join("example/specs/calc/sdd.md")).unwrap();
        assert_eq!(
            Commands::parse(&calc).test.as_deref(),
            Some("python3 -m unittest discover -s tests -t .")
        );
    }
}
