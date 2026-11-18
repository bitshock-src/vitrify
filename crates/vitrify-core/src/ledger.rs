//! Ledger derivation — held by the regression goldens (`fixtures/golden/`).
//! The ledger is derived, never hand-edited; bundle shape writes
//! `ledger.md` wholesale, single shape rewrites only the `## State Ledger`
//! section.

use std::path::PathBuf;

use crate::CoreError;
use crate::model::{self, Instance, Shape, Unit, is_empty_marker, stamp_version};

pub const STATUS_ORDER: [&str; 7] = [
    "Verified", "Built", "Claimed", "Ready", "Stale", "Draft", "Retired",
];

/// `Progress: <V>/<total> Verified · <n> <Status> · …` — statuses in the
/// fixed order, zero-count omitted after the first term; total excludes
/// Retired (unknown statuses count toward the total).
pub fn progress_line(units: &[Unit]) -> String {
    let mut counts: Vec<(String, i64)> = STATUS_ORDER.iter().map(|s| (s.to_string(), 0)).collect();
    for u in units {
        let status = u.meta.get_str_or("status", "Draft").to_string();
        match counts.iter_mut().find(|(s, _)| *s == status) {
            Some(slot) => slot.1 += 1,
            None => counts.push((status, 1)),
        }
    }
    let count_of = |name: &str| {
        counts
            .iter()
            .find(|(s, _)| s == name)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    let total: i64 = counts
        .iter()
        .filter(|(s, _)| s != "Retired")
        .map(|(_, n)| n)
        .sum();
    let mut parts = vec![format!("{}/{} Verified", count_of("Verified"), total)];
    for s in &STATUS_ORDER[1..] {
        let n = count_of(s);
        if n > 0 {
            parts.push(format!("{n} {s}"));
        }
    }
    format!("Progress: {}", parts.join(" · "))
}

const HEADS: [&str; 6] = ["Unit", "Status", "Group", "Built @", "Verified @", "Note"];

/// Render the full ledger body for `today` (`%Y-%m-%d`).
pub fn render(inst: &Instance, today: &str) -> String {
    let mut rows: Vec<[String; 6]> = Vec::new();
    let mut sorted_units: Vec<&Unit> = inst.units.iter().collect();
    sorted_units.sort_by(|a, b| a.unit.cmp(&b.unit));
    for u in sorted_units {
        let m = &u.meta;
        let status = m.get_str_or("status", "Draft").to_string();
        let note = match status.as_str() {
            "Claimed" => {
                let exp = m.get_str_or("claim_expires", "").trim().to_string();
                if is_empty_marker(&exp) {
                    "lease".to_string()
                } else {
                    format!("lease to {exp}")
                }
            }
            "Built" => "awaiting gate".to_string(),
            _ => String::new(),
        };
        let group = {
            let g = m.get_str_or("parallel_group", "").trim().to_string();
            if g.is_empty() { "—".to_string() } else { g }
        };
        rows.push([
            u.unit.clone(),
            status,
            group,
            stamp_version(m.get_str_or("built", "")),
            stamp_version(m.get_str_or("verified", "")),
            note,
        ]);
    }
    let widths: Vec<usize> = (0..6)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain(std::iter::once(HEADS[i].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let fmt = |cells: &[String]| -> String {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(c, w)| {
                let pad = w.saturating_sub(c.chars().count());
                format!("{}{}", c, " ".repeat(pad))
            })
            .collect();
        format!("| {} |", padded.join(" | "))
    };
    let heads: Vec<String> = HEADS.iter().map(|h| h.to_string()).collect();
    let mut lines = vec![
        format!(
            "# Ledger — {} @ SDD {} (derived {})",
            inst.meta.get_str_or("sdd", "?"),
            inst.meta.get_str_or("version", "?"),
            today
        ),
        String::new(),
        progress_line(&inst.units),
        String::new(),
        fmt(&heads),
        format!(
            "| {} |",
            widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ];
    for r in &rows {
        lines.push(fmt(r));
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// The file the ledger lives in and the text a regeneration leaves there:
/// a bundle's whole `ledger.md`; a single file's whole text, its
/// `## State Ledger` section replaced.
fn planned(inst: &Instance, today: &str) -> Result<(PathBuf, String), CoreError> {
    let body = render(inst, today);
    match inst.shape {
        Shape::Bundle => Ok((inst.root.join("ledger.md"), body)),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            let section = format!("## State Ledger\n\n{body}");
            let new = match model::section_span(&text, "State Ledger") {
                Some((s, e)) => format!("{}{}\n{}", &text[..s], section, &text[e..]),
                None => format!("{}\n\n{}", text.trim_end_matches('\n'), section),
            };
            Ok((inst.file.clone(), new))
        }
    }
}

/// Regenerate the ledger on disk (the `regen` verb). Returns the written
/// path and the stderr note.
pub fn regen(inst: &Instance, today: &str) -> Result<(PathBuf, String), CoreError> {
    let (out, text) = planned(inst, today)?;
    model::write_text(&out, &text)?;
    let note = match inst.shape {
        Shape::Bundle => format!("regenerated {}", out.display()),
        Shape::Single => format!("regenerated ledger section in {}", out.display()),
    };
    Ok((out, note))
}

/// What comparing the ledger on disk with a fresh derivation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The ledger on disk is what a regeneration would write.
    Current(PathBuf),
    /// There is no ledger on disk to compare.
    Missing(PathBuf),
    /// The first line that differs, 1-based, as derived and as found.
    Differs {
        path: PathBuf,
        line: usize,
        derived: String,
        found: String,
    },
}

/// `text` with the one token a regeneration always changes — the date in
/// the ledger header's `(derived YYYY-MM-DD)` — taken out. Blank lines at
/// the very end are left out too: a single file whose ledger is its last
/// section gains one the second time it is regenerated, and never again.
fn undated(text: &str) -> Vec<String> {
    text.trim_end_matches('\n')
        .split('\n')
        .map(|line| match line.find("(derived ") {
            Some(at) if line.starts_with("# Ledger — ") && line.ends_with(')') => {
                format!("{}(derived)", &line[..at])
            }
            _ => line.to_string(),
        })
        .collect()
}

/// Compare the ledger on disk with a fresh derivation from unit front
/// matter (the `regen --check` verb), reading only: the two may differ in
/// the derived date and in nothing else.
pub fn check(inst: &Instance, today: &str) -> Result<Check, CoreError> {
    let (path, derived) = planned(inst, today)?;
    let on_disk = match inst.shape {
        Shape::Bundle if !path.is_file() => return Ok(Check::Missing(path)),
        _ => model::read_text(&path)?,
    };
    if inst.shape == Shape::Single && model::section_span(&on_disk, "State Ledger").is_none() {
        return Ok(Check::Missing(path));
    }
    let (want, have) = (undated(&derived), undated(&on_disk));
    let Some(at) = (0..want.len().max(have.len())).find(|i| want.get(*i) != have.get(*i)) else {
        return Ok(Check::Current(path));
    };
    Ok(Check::Differs {
        path,
        line: at + 1,
        derived: want.get(at).cloned().unwrap_or_default(),
        found: have.get(at).cloned().unwrap_or_default(),
    })
}
