//! The TDD pin (schema §2, TDD schema "Document Metadata"): what an SDD's
//! `tdd` and `tdd_version` front matter can be held to mechanically.
//!
//! The TDD schema leaves the form of a TDD's metadata to its author, so
//! vitrify cannot require one. Two things are checkable: that `tdd:` names
//! a file, and — where the TDD carries the metadata table of vitrify's own
//! template — that an Active SDD pins its Revision, and that its Status is
//! Approved.

use std::path::{Path, PathBuf};

use crate::model::{Meta, is_empty_marker};

/// The rows of the template's metadata table a pin is held to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub revision: Option<String>,
    pub status: Option<String>,
}

/// Cells of a markdown table row; `\|` is a pipe inside a cell.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for c in t.chars() {
        match c {
            '|' if !escaped => {
                out.push(cell.trim().to_string());
                cell = String::new();
            }
            _ => cell.push(c),
        }
        escaped = c == '\\' && !escaped;
    }
    out.push(cell.trim().to_string());
    // a row starts and ends with a pipe: drop the empty cells outside them
    if out.first().is_some_and(String::is_empty) {
        out.remove(0);
    }
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// The `Revision` and `Status` rows of a TDD's metadata table, when it has
/// them: `| **Revision** | 2.1 |`.
pub fn metadata(text: &str) -> Metadata {
    let mut meta = Metadata::default();
    for line in crate::newline::split_universal(text) {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let row = cells(line);
        if row.len() < 2 {
            continue;
        }
        let key = row[0].trim_matches(['*', '`', ' ']);
        let value = row[1].trim_matches(['*', '`', ' ']).to_string();
        if key.eq_ignore_ascii_case("Revision") && meta.revision.is_none() {
            meta.revision = Some(value);
        } else if key.eq_ignore_ascii_case("Status") && meta.status.is_none() {
            meta.status = Some(value);
        }
    }
    meta
}

/// The file an SDD's `tdd:` names: the path as written, taken from the
/// directory of the SDD's core file — as the schema's and the templates'
/// examples are — or, failing that, from the repository root. `None` when
/// it names no file either way.
pub fn resolve(written: &str, sdd_file: &Path, repo_root: &Path) -> Option<PathBuf> {
    let path = Path::new(written);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let beside = sdd_file.parent().unwrap_or(Path::new(".")).join(path);
    if beside.is_file() {
        return Some(beside);
    }
    let rooted = repo_root.join(path);
    rooted.is_file().then_some(rooted)
}

/// The revision a `tdd_version` names: its first word (`2.1` of
/// `2.1 (Approved, 2026-09-14)`).
pub fn pinned_revision(tdd_version: &str) -> &str {
    tdd_version
        .trim()
        .split(|c: char| c.is_whitespace() || c == '(' || c == ',')
        .next()
        .unwrap_or("")
}

/// Where an SDD's TDD pin does not hold: `(subject, cause)` pairs. An SDD
/// that names no TDD has nothing to check. `read` returns a file's text.
pub fn findings(
    meta: &Meta,
    sdd_file: &Path,
    repo_root: &Path,
    read: impl Fn(&Path) -> Option<String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let written = meta.get_str("tdd").unwrap_or("").trim();
    if is_empty_marker(written) {
        return out;
    }
    let Some(file) = resolve(written, sdd_file, repo_root) else {
        out.push((
            "tdd".to_string(),
            format!(
                "`{written}` names no file — from the SDD's own directory, or from the repository root"
            ),
        ));
        return out;
    };
    // an SDD may go Active only against an Approved TDD; a Draft one is
    // still being derived, a Deprecated one no longer builds
    if meta.get_str("status").map(str::trim) != Some("Active") {
        return out;
    }
    let Some(text) = read(&file) else {
        return out;
    };
    let table = metadata(&text);
    // `Approved`, whatever the cell goes on to say (`Approved — 2026-09-14`)
    let approved = |status: &str| {
        status
            .split(|c: char| !c.is_alphanumeric())
            .find(|w| !w.is_empty())
            .is_some_and(|w| w.eq_ignore_ascii_case("Approved"))
    };
    if let Some(status) = &table.status
        && !approved(status)
    {
        out.push((
            "tdd".to_string(),
            format!(
                "the pinned TDD's Status is `{status}` — an Active SDD pins an Approved TDD (TDD schema, Document Metadata)"
            ),
        ));
    }
    if let Some(revision) = &table.revision {
        let pinned = meta.get_str("tdd_version").unwrap_or("").trim();
        if pinned_revision(pinned) != revision.as_str() {
            out.push((
                "tdd_version".to_string(),
                format!(
                    "{} does not name the TDD's Revision, {revision} — re-pinning is a change entry",
                    if pinned.is_empty() {
                        "nothing".to_string()
                    } else {
                        format!("`{pinned}`")
                    }
                ),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "## Document Metadata\n\n| Document Metadata | Details |\n| --- | --- |\n| **Author(s)** | Team |\n| **Revision** | 2.1 |\n| **Status** | Approved |\n";

    #[test]
    fn reads_the_template_table() {
        let m = metadata(TABLE);
        assert_eq!(m.revision.as_deref(), Some("2.1"));
        assert_eq!(m.status.as_deref(), Some("Approved"));
        // the unfilled template: escaped pipes stay inside the cell
        let m = metadata(
            "| **Revision** | [rev] |\n| **Status** | [Draft \\| In Review \\| Approved \\| Deprecated] |\n",
        );
        assert_eq!(m.revision.as_deref(), Some("[rev]"));
        assert_eq!(
            m.status.as_deref(),
            Some("[Draft \\| In Review \\| Approved \\| Deprecated]")
        );
        // a TDD with metadata of its own form has no rows to hold
        assert_eq!(
            metadata("Revision: 3\nStatus: Approved\n"),
            Metadata::default()
        );
    }

    #[test]
    fn the_pinned_revision_is_the_first_word() {
        assert_eq!(pinned_revision("2.1 (Approved, 2026-09-14)"), "2.1");
        assert_eq!(pinned_revision("1.20(Approved)"), "1.20");
        assert_eq!(pinned_revision(""), "");
    }

    fn meta(block: &str) -> Meta {
        Meta(crate::yamlish::parse(block))
    }

    #[test]
    fn an_active_sdd_is_held_to_the_table_and_a_draft_only_to_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("specs/x")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let sdd = root.join("specs/x/sdd.md");
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        let check = |block: &str| findings(&meta(block), &sdd, root, read);

        // no TDD named: nothing to check
        assert!(check("status: Active\n").is_empty());
        // named, and not there
        let got = check("status: Draft\ntdd: ../../docs/x.md\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "tdd");

        std::fs::write(
            root.join("docs/x.md"),
            TABLE.replace("Approved", "In Review"),
        )
        .unwrap();
        // from the SDD's directory, or from the repository root
        assert!(check("status: Draft\ntdd: ../../docs/x.md\n").is_empty());
        assert!(check("status: Draft\ntdd: docs/x.md\n").is_empty());
        // Active against an unapproved TDD, at a stale revision
        let got = check(
            "status: Active\ntdd: ../../docs/x.md\ntdd_version: 2.0 (Approved, 2026-01-01)\n",
        );
        let subjects: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(subjects, ["tdd", "tdd_version"]);
        assert!(got[0].1.contains("In Review"));
        assert!(got[1].1.contains("2.1"));

        std::fs::write(root.join("docs/x.md"), TABLE).unwrap();
        assert!(
            check(
                "status: Active\ntdd: ../../docs/x.md\ntdd_version: 2.1 (Approved, 2026-09-14)\n"
            )
            .is_empty()
        );
        // the Status cell may say more than the one word
        std::fs::write(
            root.join("docs/x.md"),
            TABLE.replace("| Approved |", "| **Approved** — 2026-09-14 |"),
        )
        .unwrap();
        assert!(check("status: Active\ntdd: ../../docs/x.md\ntdd_version: 2.1\n").is_empty());
        std::fs::write(
            root.join("docs/x.md"),
            TABLE.replace("| Approved |", "| Not approved |"),
        )
        .unwrap();
        assert_eq!(
            check("status: Active\ntdd: ../../docs/x.md\ntdd_version: 2.1\n").len(),
            1
        );
        // a TDD without the table: the path is all there is to hold
        std::fs::write(root.join("docs/x.md"), "# TDD\n\nRevision 7, approved.\n").unwrap();
        assert!(check("status: Active\ntdd: ../../docs/x.md\ntdd_version: 1\n").is_empty());
    }
}
