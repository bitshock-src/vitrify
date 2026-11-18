//! The write-set law (schema §13.3), mechanically: which changes one unit's
//! work may carry into integration. Pure — the caller supplies the changed
//! paths and both versions of the unit's spec file.
//!
//! A unit's work may change exactly:
//!
//! * paths its `owns` globs match (the same matcher the audit uses) — never
//!   one of the SDD's own files, whatever a glob matches;
//! * its own spec entry, in [`HANDOFF_FIELDS`] only — what
//!   `vitrify trace record --stamp built` writes.
//!
//! Everything else is rejected: files other units own, `unmanaged` paths,
//! files no unit owns, and every other file of the SDD. Tests get no special
//! treatment — a test file is writable exactly when the unit owns it, which
//! is what makes an SDD's test posture binding.

use std::path::Path;

use crate::model::{Instance, Shape, Unit};
use crate::{CoreError, audit, trace};

/// The front-matter fields an implementer's handoff may change in its own
/// unit entry (`trace record --stamp built`).
pub const HANDOFF_FIELDS: [&str; 3] = ["status", "built", "trace"];

/// The files a bundle SDD keeps directly in its directory, and the
/// directories it keeps units and contracts in (schema §1).
const BUNDLE_FILES: [&str; 5] = [
    "sdd.md",
    "ledger.md",
    "journal.ndjson",
    "changelog.md",
    "decisions.md",
];
const BUNDLE_DIRS: [&str; 2] = ["units", "contracts"];

/// Why a changed path is outside the write-set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Not owned by the unit (owned by another unit, `unmanaged`, or owned
    /// by no one).
    NotOwned(String),
    /// The unit's own spec file changed beyond [`HANDOFF_FIELDS`] (spec
    /// text, or — single shape — anything outside the unit's own fields).
    SpecEdit(String),
    /// Another file of the SDD: other units' entries, contracts, the core
    /// file, ledger, journal, changelog, decisions.
    SpecFile(String),
}

impl Violation {
    pub fn path(&self) -> &str {
        match self {
            Violation::NotOwned(p) | Violation::SpecEdit(p) | Violation::SpecFile(p) => p,
        }
    }

    /// One human line naming the path and the rule it broke.
    pub fn describe(&self, unit: &str) -> String {
        match self {
            Violation::NotOwned(p) => format!("{p}: not owned by {unit}"),
            Violation::SpecEdit(p) => format!(
                "{p}: spec text edited — only {} may change",
                HANDOFF_FIELDS.join(", ")
            ),
            Violation::SpecFile(p) => {
                format!("{p}: a file of the SDD — no unit's work may change it")
            }
        }
    }
}

/// What one unit's work may change.
#[derive(Debug, Clone)]
pub struct WriteSet {
    pub unit: String,
    pub owns: Vec<String>,
    /// Repo-relative path of the file holding the unit's spec entry: its
    /// unit file (bundle) or the SDD file (single).
    pub spec_file: String,
    /// Repo-relative files and directories of the SDD itself; no `owns`
    /// glob covers them.
    pub spec_paths: Vec<String>,
    pub shape: Shape,
}

fn rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

impl WriteSet {
    /// The write-set of `unit` as `inst` defines it; `root` is the repository
    /// root the instance was loaded under.
    pub fn of(inst: &Instance, unit: &Unit, root: &Path) -> Result<WriteSet, CoreError> {
        let (spec_file, spec_paths) = match inst.shape {
            Shape::Bundle => {
                let dir = rel(&inst.root, root);
                let paths = if dir.is_empty() {
                    // a bundle at the repository root: its own files only
                    BUNDLE_FILES
                        .iter()
                        .chain(BUNDLE_DIRS.iter())
                        .map(|p| p.to_string())
                        .collect()
                } else {
                    vec![dir]
                };
                (rel(&unit.file, root), paths)
            }
            Shape::Single => {
                let file = rel(&inst.file, root);
                (file.clone(), vec![file])
            }
        };
        Ok(WriteSet {
            unit: unit.unit.clone(),
            owns: unit
                .meta
                .get_list("owns")
                .iter()
                .map(|g| g.trim_end().to_string())
                .collect(),
            spec_file,
            spec_paths,
            shape: inst.shape,
        })
    }

    /// One of the SDD's own files.
    pub fn is_spec(&self, path: &str) -> bool {
        self.spec_paths
            .iter()
            .any(|p| path == p || path.starts_with(&format!("{p}/")))
    }

    /// Owned by the unit and not a file of the SDD.
    pub fn covers(&self, path: &str) -> bool {
        !self.is_spec(path) && self.owns.iter().any(|g| audit::matches(path, g))
    }

    /// `before` → `after` (two versions of [`WriteSet::spec_file`]) changes
    /// nothing but this unit's [`HANDOFF_FIELDS`].
    pub fn spec_edit_allowed(&self, before: &str, after: &str) -> bool {
        let mask = |t: &str| trace::mask_unit_fields(self.shape, t, &self.unit, &HANDOFF_FIELDS);
        match (mask(before), mask(after)) {
            (Some(b), Some(a)) => b == a,
            _ => false,
        }
    }

    /// Classify every changed path. `spec_versions` returns the
    /// (before, after) text of the spec file — `None` for a side where it is
    /// absent — and is consulted only when that file changed.
    pub fn violations(
        &self,
        changed: &[String],
        spec_versions: impl FnOnce() -> (Option<String>, Option<String>),
    ) -> Vec<Violation> {
        let mut out = Vec::new();
        let mut spec_versions = Some(spec_versions);
        for path in changed {
            if *path == self.spec_file {
                let (before, after) = spec_versions.take().map(|f| f()).unwrap_or((None, None));
                let ok = match (before, after) {
                    (Some(b), Some(a)) => self.spec_edit_allowed(&b, &a),
                    _ => false,
                };
                if !ok {
                    out.push(Violation::SpecEdit(path.clone()));
                }
            } else if self.is_spec(path) {
                out.push(Violation::SpecFile(path.clone()));
            } else if !self.covers(path) {
                out.push(Violation::NotOwned(path.clone()));
            }
        }
        out
    }
}
