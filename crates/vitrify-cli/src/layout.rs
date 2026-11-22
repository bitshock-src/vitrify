//! Where vitrify keeps its working copies, and what it calls them.
//!
//! Two places. `<repo>/.vitrify/` is scratch nothing is ever built in:
//! transaction worktrees, logs, `run.json`, the repo-level config. The unit
//! worktrees and the integration checkouts — where workers build and gate
//! commands run — live **outside the working tree**, by default under the
//! vitrify home, so that no ignore file of the repository (its own
//! `.gitignore`, or the `*` vitrify writes into `.vitrify/`) is an ancestor
//! of the files being checked. Tools that honour parent ignore files would
//! otherwise treat every source file as ignored and pass without looking.
//!
//! Branch and worktree names carry the SDD's slug: several SDDs in one
//! repository all have a `U-001`, and their work must never share a name.
//! So do a run's lanes — one per SDD: each has its own integration
//! checkout and lock, its own head checkout, and its own records under the
//! job's logs, so runs of different SDDs never wait on each other's gate.

use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::model::{Instance, Meta, Shape};
use vitrify_core::trace::hash12;

use crate::git::Git;

/// The directories of one repository's vitrify state.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The primary checkout.
    pub primary: PathBuf,
    /// `<primary>/.vitrify` — transactions, logs, `run.json`.
    pub scratch: PathBuf,
    /// Root of the unit worktrees and the integration checkouts.
    pub wt_root: PathBuf,
}

impl Layout {
    /// The layout for `primary`. `worktree_dir` is `run.worktree_dir`: empty
    /// puts the worktrees under the vitrify home (`home`), keyed by the
    /// repository; a relative path is taken from the repository root.
    pub fn resolve(
        primary: &Path,
        home: Option<&Path>,
        worktree_dir: &str,
    ) -> Result<Layout, CoreError> {
        let wt_root = if worktree_dir.trim().is_empty() {
            let Some(home) = home else {
                return Err(CoreError::refusal(
                    1,
                    "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
                ));
            };
            let name = primary
                .file_name()
                .map(|n| ref_part(&n.to_string_lossy()))
                .unwrap_or_else(|| "repo".to_string());
            let key = hash12(&primary.to_string_lossy());
            home.join("worktrees").join(format!("{name}-{}", &key[..8]))
        } else {
            let p = PathBuf::from(worktree_dir.trim());
            if p.is_absolute() { p } else { primary.join(p) }
        };
        Ok(Layout::at(primary, &wt_root))
    }

    /// An explicit layout (tests, and callers that already know the root).
    pub fn at(primary: &Path, wt_root: &Path) -> Layout {
        Layout {
            primary: primary.to_path_buf(),
            scratch: primary.join(".vitrify"),
            wt_root: normalize(wt_root),
        }
    }

    /// The integration checkout of the SDD `slug`: a detached worktree of
    /// the mainline head, where its gates and §5 commands run.
    pub fn mainline(&self, slug: &str) -> PathBuf {
        self.wt_root.join(format!("mainline-{slug}"))
    }

    /// The lock on that checkout: runs of one SDD gate one at a time,
    /// across processes; runs of different SDDs gate side by side. (A slug
    /// never ends in `.lock`, so no checkout is ever named like a lock.)
    pub fn mainline_lock(&self, slug: &str) -> PathBuf {
        self.wt_root.join(format!("mainline-{slug}.lock"))
    }

    /// The checkout where the lane of the SDD `slug` runs a gate's failing
    /// rows again on the gate's base, apart from its integration checkout.
    pub fn base_checkout(&self, slug: &str) -> PathBuf {
        self.wt_root.join(format!("base-{slug}"))
    }

    /// This process's detached checkout of the mainline head for the SDD
    /// `slug`, where the run reads spec state.
    pub fn head_checkout(&self, slug: &str) -> PathBuf {
        self.scratch
            .join(format!("tx-{}-{slug}-head", std::process::id()))
    }

    /// `.vitrify/logs/<job>/`: the job's own records — its `run.json`,
    /// `project.json`, `events.jsonl` and final report.
    pub fn job_logs(&self, job: &str) -> PathBuf {
        self.scratch.join("logs").join(job)
    }

    /// `.vitrify/logs/<job>/<slug>/`: one lane's records — its worker logs
    /// and prompts, its gate board, its escalations.
    pub fn lane_logs(&self, job: &str, slug: &str) -> PathBuf {
        self.job_logs(job).join(slug)
    }

    /// One claim's worktree.
    pub fn unit_worktree(&self, slug: &str, unit: &str, seq: i64) -> PathBuf {
        self.wt_root.join(format!("{slug}-{unit}-c{seq}"))
    }

    /// The worktree root relative to the primary checkout, when it lies
    /// inside it.
    pub fn in_tree(&self) -> Option<PathBuf> {
        self.wt_root
            .strip_prefix(&self.primary)
            .ok()
            .map(Path::to_path_buf)
    }

    /// Create `.vitrify/` with the ignore file that keeps it out of the
    /// repository. Nothing is built under it, so the `*` hides nothing a
    /// tool needs to see.
    pub fn ensure_scratch(&self) -> Result<(), CoreError> {
        ensure_scratch(&self.primary).map(|_| ())
    }

    /// Create the worktree root. A root inside the working tree is added to
    /// `.git/info/exclude`, and refused when one of the repository's own
    /// ignore files already covers it — the case in which ignore-aware
    /// tools check nothing there.
    pub fn ensure_worktree_root(&self, primary: &Git) -> Result<(), CoreError> {
        std::fs::create_dir_all(&self.wt_root).map_err(|source| CoreError::Io {
            path: self.wt_root.clone(),
            source,
        })?;
        let Some(rel) = self.in_tree() else {
            return Ok(());
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            return Err(CoreError::refusal(
                2,
                "error: run.worktree_dir is the repository root itself — name a directory",
            ));
        }
        primary
            .exclude_locally(&format!("/{rel}/"))
            .map_err(CoreError::from)?;
        if let Some(source) = primary
            .ignored_by(&format!("{rel}/probe"))
            .map_err(CoreError::from)?
            && !source.contains("info/exclude")
        {
            return Err(CoreError::refusal(
                2,
                format!(
                    "error: the worktree directory {rel}/ is ignored by {source} — tools that honour parent ignore files would check nothing in vitrify's worktrees; remove that pattern, or point run.worktree_dir elsewhere"
                ),
            ));
        }
        Ok(())
    }
}

/// `path` with `.` and `..` resolved by name alone (the directory need not
/// exist yet): `../repo-wt` beside the repository is outside it, whatever
/// the spelling.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Create `<primary>/.vitrify` and its ignore file; returns the directory.
pub fn ensure_scratch(primary: &Path) -> Result<PathBuf, CoreError> {
    let scratch = primary.join(".vitrify");
    std::fs::create_dir_all(&scratch).map_err(|source| CoreError::Io {
        path: scratch.clone(),
        source,
    })?;
    let _ = std::fs::write(scratch.join(".gitignore"), "*\n");
    Ok(scratch)
}

/// `s` as one component of a git ref or a directory name: anything outside
/// `[A-Za-z0-9._-]` becomes `-`, leading dots and dashes go, and a `.lock`
/// suffix (which git forbids) is defused.
pub fn ref_part(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("..") {
        out = out.replace("..", ".");
    }
    let mut out = out
        .trim_start_matches(['.', '-'])
        .trim_end_matches('.')
        .to_string();
    if out.ends_with(".lock") {
        out.push('-');
    }
    if out.is_empty() {
        out.push_str("sdd");
    }
    out
}

/// The SDD's name in branch and worktree names: its `sdd:` slug, else the
/// name of its directory (bundle) or file (single).
pub fn slug_of(meta: &Meta, root: &Path, shape: Shape) -> String {
    let named = meta.get_str("sdd").map(str::trim).unwrap_or("");
    if !named.is_empty() {
        return ref_part(named);
    }
    let stem = match shape {
        Shape::Bundle => root.file_name().map(|n| n.to_string_lossy().to_string()),
        Shape::Single => root.file_name().map(|n| {
            let n = n.to_string_lossy();
            n.strip_suffix(".sdd.md")
                .or_else(|| n.strip_suffix(".md"))
                .unwrap_or(&n)
                .to_string()
        }),
    };
    ref_part(&stem.unwrap_or_default())
}

pub fn sdd_slug(inst: &Instance) -> String {
    slug_of(&inst.meta, &inst.root, inst.shape)
}

/// `sdd/<slug>/<unit>/` — every attempt branch of one unit.
pub fn branch_prefix(slug: &str, unit: &str) -> String {
    format!("sdd/{slug}/{}/", ref_part(unit))
}

/// `sdd/<slug>/<unit>/c<seq>` — the branch name carries the fencing token.
pub fn unit_branch(slug: &str, unit: &str, seq: i64) -> String {
    format!("{}c{seq}", branch_prefix(slug, unit))
}

/// The claim sequence a branch name carries (`…/c<seq>`).
pub fn branch_seq(branch: &str) -> Option<i64> {
    branch.rsplit('/').next()?.strip_prefix('c')?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_the_slug_and_the_token() {
        assert_eq!(
            unit_branch("shop-cart", "U-001", 3),
            "sdd/shop-cart/U-001/c3"
        );
        assert_eq!(branch_seq("sdd/shop-cart/U-001/c3"), Some(3));
        assert_eq!(branch_seq("sdd/shop-cart/U-001/x3"), None);
        let l = Layout::at(Path::new("/r"), Path::new("/w"));
        assert_eq!(
            l.unit_worktree("shop-cart", "U-001", 3),
            PathBuf::from("/w/shop-cart-U-001-c3")
        );
        assert_eq!(
            l.mainline("shop-cart"),
            PathBuf::from("/w/mainline-shop-cart")
        );
        assert_eq!(
            l.mainline_lock("shop-cart"),
            PathBuf::from("/w/mainline-shop-cart.lock")
        );
        assert_eq!(
            l.lane_logs("run-x", "shop-cart"),
            PathBuf::from("/r/.vitrify/logs/run-x/shop-cart")
        );
        assert_eq!(
            l.head_checkout("shop-cart"),
            PathBuf::from(format!(
                "/r/.vitrify/tx-{}-shop-cart-head",
                std::process::id()
            ))
        );
        assert!(l.in_tree().is_none());
        assert_eq!(
            Layout::at(Path::new("/r"), Path::new("/r/wt")).in_tree(),
            Some(PathBuf::from("wt"))
        );
    }

    #[test]
    fn ref_part_is_always_a_legal_component() {
        assert_eq!(ref_part("shop-cart"), "shop-cart");
        assert_eq!(ref_part("my sdd/v2"), "my-sdd-v2");
        assert_eq!(ref_part("..hidden"), "hidden");
        assert_eq!(ref_part("a..b"), "a.b");
        assert_eq!(ref_part("x.lock"), "x.lock-");
        assert_eq!(ref_part("~^:"), "sdd");
    }

    #[test]
    fn default_root_is_outside_the_working_tree_and_keyed_by_repository() {
        let a = Layout::resolve(Path::new("/src/app"), Some(Path::new("/h")), "").unwrap();
        let b = Layout::resolve(Path::new("/other/app"), Some(Path::new("/h")), "").unwrap();
        assert!(a.wt_root.starts_with("/h/worktrees"));
        assert!(a.in_tree().is_none());
        assert_ne!(a.wt_root, b.wt_root, "same name, different repository");
        // an explicit directory: absolute as given, relative from the root
        let c = Layout::resolve(Path::new("/src/app"), None, "/big/disk").unwrap();
        assert_eq!(c.wt_root, PathBuf::from("/big/disk"));
        let d = Layout::resolve(Path::new("/src/app"), None, "vitrify-wt").unwrap();
        assert_eq!(d.in_tree(), Some(PathBuf::from("vitrify-wt")));
        // beside the repository is outside it, however it is spelled
        let e = Layout::resolve(Path::new("/src/app"), None, "../app-wt").unwrap();
        assert_eq!(e.wt_root, PathBuf::from("/src/app-wt"));
        assert!(e.in_tree().is_none());
        let f = Layout::resolve(Path::new("/src/app"), None, "./a/../wt").unwrap();
        assert_eq!(f.in_tree(), Some(PathBuf::from("wt")));
        // no home and no directory: nowhere to put them
        assert!(Layout::resolve(Path::new("/src/app"), None, "").is_err());
    }
}
