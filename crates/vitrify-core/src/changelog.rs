//! Change entries (schema §12): the `CH-###` headings of an SDD's changelog
//! — `changelog.md` in a bundle, the `## Changelog` section of a single
//! file. Read-only: an entry is written by its author, and what vitrify
//! holds against it is the front matter's `version` and `updated`.

use crate::CoreError;
use crate::model::{self, SddHandle, Shape};

/// One change entry, as its heading states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `CH-###`.
    pub id: String,
    pub number: u64,
    /// The entry's date: the last `(YYYY-MM-DD)` of the heading.
    pub date: Option<String>,
    /// The version the entry bumps the SDD from: the version just before
    /// the heading's last `→`, when one is written there.
    pub from: Option<String>,
    /// The version the entry bumps the SDD to: what follows the heading's
    /// last `→`.
    pub to: Option<String>,
}

/// What a change entry's version transition bumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn as_str(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

/// The last `(YYYY-MM-DD)` in `line`.
fn last_date(line: &str) -> Option<String> {
    let mut found = None;
    let mut from = 0;
    while let Some(rel) = line[from..].find('(') {
        let at = from + rel + 1;
        if let Some(date) = line.get(at..at + 10)
            && is_iso_date(date)
            && line[at + 10..].starts_with(')')
        {
            found = Some(date.to_string());
        }
        from = at;
    }
    found
}

/// `line` split around its last `→` (or `->`).
fn around_arrow(line: &str) -> Option<(&str, &str)> {
    match (line.rfind('→'), line.rfind("->")) {
        (Some(a), Some(b)) if b > a => Some((&line[..b], &line[b + 2..])),
        (Some(a), _) => Some((&line[..a], &line[a + '→'.len_utf8()..])),
        (None, Some(b)) => Some((&line[..b], &line[b + 2..])),
        (None, None) => None,
    }
}

/// What follows the last `→` (or `->`) of `line`, up to the next blank.
fn target_version(line: &str) -> Option<String> {
    let (_, after) = around_arrow(line)?;
    after
        .split_whitespace()
        .next()
        .map(bare)
        .filter(|v| !v.is_empty())
}

/// The version just before the last `→` (or `->`) of `line` — `None` when
/// what stands there is not a version (`— → 0.1.0`).
fn source_version(line: &str) -> Option<String> {
    let (before, _) = around_arrow(line)?;
    before
        .split_whitespace()
        .next_back()
        .map(bare)
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// A version's numeric `major.minor.patch`, missing parts read as 0.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut out = [0u64; 3];
    let mut parts = core.split('.');
    for slot in &mut out {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None => break,
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// What going from `from` to `to` bumps: the first of major, minor, patch
/// that differs. `None` when the two are the same version or either is not
/// one.
pub fn bump(from: &str, to: &str) -> Option<Bump> {
    let (a, b) = (numbers(&bare(from))?, numbers(&bare(to))?);
    if a[0] != b[0] {
        Some(Bump::Major)
    } else if a[1] != b[1] {
        Some(Bump::Minor)
    } else if a[2] != b[2] {
        Some(Bump::Patch)
    } else {
        None
    }
}

/// The last change entry — the one with the highest id, which is the one
/// the front matter's `version` and `updated` are held to.
pub fn last(entries: &[Entry]) -> Option<&Entry> {
    entries.iter().max_by_key(|e| e.number)
}

/// The version `entry` bumps from: the one its heading writes, else the
/// version the entry before it (by id) went to.
pub fn from_version(entries: &[Entry], entry: &Entry) -> Option<String> {
    entry.from.clone().or_else(|| {
        entries
            .iter()
            .filter(|e| e.number < entry.number)
            .max_by_key(|e| e.number)
            .and_then(|e| e.to.clone())
    })
}

/// A version as written in passing — `` `v1.4.0`. `` — down to `1.4.0`.
fn bare(version: &str) -> String {
    let v = version
        .trim()
        .trim_matches(['`', '*', '.', ',', ';', ')', '(']);
    v.strip_prefix(['v', 'V'])
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(v)
        .to_string()
}

/// The change entries of a changelog's text, in document order: every
/// heading (outside fenced code) whose title begins `CH-<digits>`.
pub fn parse(text: &str) -> Vec<Entry> {
    headings(text).into_iter().map(|(_, e)| e).collect()
}

/// [`parse`], each entry with its heading's `#` count.
fn headings(text: &str) -> Vec<(usize, Entry)> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for line in crate::newline::split_universal(text) {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        if !(1..=6).contains(&hashes) || !line[hashes..].starts_with(char::is_whitespace) {
            continue;
        }
        let title = line[hashes..].trim();
        let Some(rest) = title.strip_prefix("CH-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let ends = rest[digits.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        let Ok(number) = digits.parse::<u64>() else {
            continue;
        };
        if !ends {
            continue;
        }
        out.push((
            hashes,
            Entry {
                id: format!("CH-{digits}"),
                number,
                date: last_date(title),
                from: source_version(title),
                to: target_version(title),
            },
        ));
    }
    out
}

/// The change entries a single-file SDD keeps where none is read: each
/// `CH-###` heading outside its `## Changelog` section — at `##`, such a
/// heading ends that section — as `(id, its heading's # count)`, in
/// document order. Empty for a bundle, whose entries are `changelog.md`'s.
pub fn misplaced(sdd: &SddHandle) -> Result<Vec<(String, usize)>, CoreError> {
    if sdd.shape != Shape::Single {
        return Ok(Vec::new());
    }
    Ok(misplaced_in(&model::read_text(&sdd.file)?))
}

/// [`misplaced`], over a single-file SDD's text.
pub fn misplaced_in(text: &str) -> Vec<(String, usize)> {
    let outside = match model::section_span(text, "Changelog") {
        Some((s, e)) => format!("{}{}", &text[..s], &text[e..]),
        None => text.to_string(),
    };
    headings(&outside)
        .into_iter()
        .map(|(hashes, e)| (e.id, hashes))
        .collect()
}

/// Why a [`misplaced`] entry is not read, and what puts it right:
/// `(cause, remediation)`, for lint L10 and `vitrify change apply` alike.
pub fn misplaced_finding(id: &str, hashes: usize) -> (String, String) {
    let cause = if hashes <= 2 {
        format!(
            "a change entry outside `## Changelog` is not read: `{} {id}` ends that section",
            "#".repeat(hashes)
        )
    } else {
        "a change entry outside `## Changelog` is not read".to_string()
    };
    (
        cause,
        format!("write it as `### {id} — …` under `## Changelog`"),
    )
}

/// The change entries of an SDD; empty when it keeps no changelog.
pub fn entries(sdd: &SddHandle) -> Result<Vec<Entry>, CoreError> {
    let text = match sdd.shape {
        Shape::Bundle => {
            let path = sdd.root.join("changelog.md");
            if !path.is_file() {
                return Ok(Vec::new());
            }
            model::read_text(&path)?
        }
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            model::section(&text, "Changelog").to_string()
        }
    };
    Ok(parse(&text))
}

/// Where `version` and `updated` disagree with the changelog, and where the
/// changelog disagrees with itself: `(subject, cause)` pairs. An SDD
/// without change entries has nothing to be held against.
pub fn findings(entries: &[Entry], version: &str, updated: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in entries.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        if after.number == before.number {
            out.push((
                after.id.clone(),
                "appears twice — change entry ids are never reused".to_string(),
            ));
        } else if after.number < before.number {
            out.push((
                after.id.clone(),
                format!(
                    "follows {} — change entries are appended in id order, newest last",
                    before.id
                ),
            ));
        }
    }
    let Some(last) = last(entries) else {
        return out;
    };
    if let Some(to) = &last.to
        && *to != bare(version)
    {
        out.push((
            "version".to_string(),
            format!(
                "front matter says {}, the last change entry {} goes to {to} — every version bump has its change entry, and every entry its bump",
                if version.trim().is_empty() {
                    "nothing"
                } else {
                    version.trim()
                },
                last.id
            ),
        ));
    }
    if let Some(date) = &last.date
        && date != updated.trim()
    {
        out.push((
            "updated".to_string(),
            format!(
                "front matter says {}, the last change entry {} is dated {date}",
                if updated.trim().is_empty() {
                    "nothing"
                } else {
                    updated.trim()
                },
                last.id
            ),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_of_either_depth_carry_id_date_and_version() {
        let text = "# Changelog\n\n## CH-001 — Initial authoring (2026-10-28) — 0.1.0 → 1.0.0\n\n- Added: U-001\n\n### CH-002 — Pins synced from the 2026-11-05 re-pins — 1.0.0 → 1.0.1\n\n### CH-003 — A view (2026-11-01), not a missing one (2026-11-15) — → 2.0.0\n";
        let got = parse(text);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].id, "CH-001");
        assert_eq!(got[0].date.as_deref(), Some("2026-10-28"));
        assert_eq!(got[0].to.as_deref(), Some("1.0.0"));
        // a date in the title's prose is not the entry's date
        assert_eq!(got[1].date, None);
        assert_eq!(got[1].to.as_deref(), Some("1.0.1"));
        // the last parenthesized date is the entry's
        assert_eq!(got[2].date.as_deref(), Some("2026-11-15"));
        assert_eq!(got[2].to.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn placeholders_and_fenced_examples_are_not_entries_to_judge() {
        // the shipped template: a placeholder date, a real target version
        let template = "# Changelog\n\n### CH-001 — Initial authoring ([YYYY-MM-DD]) — → 0.1.0\n";
        let got = parse(template);
        assert_eq!(got[0].date, None);
        assert_eq!(got[0].to.as_deref(), Some("0.1.0"));
        assert!(findings(&got, "0.1.0", "[YYYY-MM-DD]").is_empty());
        // an entry quoted inside a code fence is prose
        let quoted = "```\n## CH-009 — example (2026-01-01) — 1.0.0 → 9.0.0\n```\n";
        assert!(parse(quoted).is_empty());
        // CH-12x is not an id
        assert!(parse("## CH-12x — no\n").is_empty());
    }

    #[test]
    fn version_updated_and_order_are_held_to_the_last_entry() {
        let entries = parse(
            "## CH-001 — a (2026-10-01) — 0.1.0 → 1.0.0\n## CH-002 — b (2026-10-05) — 1.0.0 → 1.1.0\n",
        );
        assert!(findings(&entries, "1.1.0", "2026-10-05").is_empty());
        let got = findings(&entries, "1.0.0", "2026-10-01");
        let subjects: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(subjects, ["version", "updated"]);
        assert!(got[0].1.contains("CH-002 goes to 1.1.0"), "{}", got[0].1);

        // out of order, and reused
        let entries = parse(
            "## CH-002 — b (2026-10-05) — → 1.1.0\n## CH-001 — a (2026-10-01) — → 1.0.0\n## CH-001 — again — → 1.0.0\n",
        );
        let got = findings(&entries, "1.1.0", "2026-10-05");
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].1.contains("follows CH-002"));
        assert!(got[1].1.contains("appears twice"));

        // no entries: nothing to hold the front matter against
        assert!(findings(&[], "9.9.9", "").is_empty());
        // a version is a version, however it is dressed
        let entries = parse("## CH-001 — a (2026-10-01) — v0.1.0 → `v1.0.0`.\n");
        assert_eq!(entries[0].to.as_deref(), Some("1.0.0"));
        assert!(findings(&entries, "1.0.0", "2026-10-01").is_empty());
        assert!(findings(&entries, "v1.0.0", "2026-10-01").is_empty());
        assert_eq!(findings(&entries, "1.0.1", "2026-10-01").len(), 1);
    }

    #[test]
    fn a_single_files_entries_outside_its_changelog_are_misplaced() {
        // under `## Changelog`, at any depth below it: read, not misplaced
        let read = "# s\n\n## Changelog\n\n### CH-001 — a (2026-10-01) — → 1.0.0\n\n#### CH-002 — b — → 1.1.0\n\n## Journal\n";
        assert!(misplaced_in(read).is_empty());
        // a `##` entry ends the section it was meant for; one elsewhere is
        // as unread; a quoted one is prose
        let text = "# s\n\n## Changelog\n\n### CH-001 — a (2026-10-01) — → 1.0.0\n\n## CH-002 — b (2026-11-18) — 1.0.0 → 2.0.0\n\n- Added: U-002\n\n## Decisions & Open Questions\n\n### CH-003 — c — → 2.1.0\n\n```\n## CH-009 — example — → 9.0.0\n```\n";
        assert_eq!(
            misplaced_in(text),
            [("CH-002".to_string(), 2), ("CH-003".to_string(), 3)]
        );
        // with no `## Changelog` at all, every entry is unread
        assert_eq!(
            misplaced_in("# s\n\n## CH-001 — a — → 1.0.0\n"),
            [("CH-001".to_string(), 2)]
        );
        let (cause, fix) = misplaced_finding("CH-002", 2);
        assert_eq!(
            cause,
            "a change entry outside `## Changelog` is not read: `## CH-002` ends that section"
        );
        assert_eq!(fix, "write it as `### CH-002 — …` under `## Changelog`");
        assert_eq!(
            misplaced_finding("CH-003", 3).0,
            "a change entry outside `## Changelog` is not read"
        );
    }

    #[test]
    fn the_last_entry_carries_its_transition_and_bump() {
        let entries = parse(
            "## CH-001 — a (2026-10-01) — → 1.0.0\n## CH-002 — b (2026-10-05) — `v1.0.0` → 1.1.0\n## CH-003 — c (2026-10-06) — → 1.1.1\n",
        );
        assert_eq!(entries[0].from, None);
        assert_eq!(entries[1].from.as_deref(), Some("1.0.0"));
        let last = last(&entries).unwrap();
        assert_eq!(last.id, "CH-003");
        // a heading without its source version reads the one the entry before it went to
        assert_eq!(from_version(&entries, last).as_deref(), Some("1.1.0"));
        assert_eq!(from_version(&entries, &entries[0]), None);
        assert_eq!(bump("1.1.0", "1.1.1"), Some(Bump::Patch));
        assert_eq!(bump("1.2.0", "1.3.0"), Some(Bump::Minor));
        assert_eq!(bump("0.1.0", "1.0.0"), Some(Bump::Major));
        assert_eq!(bump("1.2", "1.2.1"), Some(Bump::Patch));
        assert_eq!(bump("1.0.0", "1.0.0"), None);
        assert_eq!(bump("one", "1.0.0"), None);
    }
}
