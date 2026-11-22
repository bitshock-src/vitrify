//! No new failures: a gate blames a batch only for rows that pass on the base.
//!
//! When Verification rows fail at a gate or a re-check, those rows run again
//! on the base — the commit the batch was merged onto — in a pristine
//! checkout of it:
//!
//! * **they pass on the base:** the batch broke them. The batch is rejected,
//!   as it always was;
//! * **they fail on the base too:** the batch did not break them. Their
//!   units are **rotten** — something already on mainline broke them, or the
//!   rows are flaky, which culprit finding ([`super::culprit`]) sorts out —
//!   and the batch is judged on its other rows.
//!
//! This is the rule the gate already applies to the §5 Test command, applied
//! per row. A row that failed beside others is run again alone before it
//! counts as failed at all (`gate::run_rows`); that stays where it is, and
//! only confirmed failures come here.
//!
//! The base is prepared the way every gate tree is: reset to the commit with
//! no untracked or ignored file left ([`PristineCheckout`]), then the SDD's
//! §5 Bootstrap and Build, then the failed rows one at a time. When the base
//! itself cannot be set up (its Bootstrap or Build fails), nothing can be
//! said about the rows there, and the batch is blamed for all of them — the
//! conservative answer, and the one a gate gave before this rule existed.
//!
//! [`classify`] is the pure rule; [`judge`] runs the base and applies it.
//! Both take what they run through two small injected pieces: a [`Runner`]
//! (one §5 command or Verification row in a directory) and a [`Prepare`]
//! (a pristine checkout at a commit).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::graph::Node;

use crate::gate::{self, Ran};
use crate::git::Git;

/// Runs one §5 command or one Verification row (through `sh -c`) in a
/// directory. Shared by gates, re-checks and culprit probes.
pub trait Runner: Send + Sync {
    fn run(&self, cmd: &str, dir: &Path) -> Ran;
}

/// The runner a run uses: `gate::run_command` (its own temp slot, its own
/// process group, stopped on interrupt).
pub struct ShellRunner;

impl Runner for ShellRunner {
    fn run(&self, cmd: &str, dir: &Path) -> Ran {
        gate::run_command(cmd, dir)
    }
}

impl<F> Runner for F
where
    F: Fn(&str, &Path) -> Ran + Send + Sync,
{
    fn run(&self, cmd: &str, dir: &Path) -> Ran {
        self(cmd, dir)
    }
}

/// Makes a pristine checkout at a commit: the tree of `commit`, with no
/// untracked or ignored file left. Returns the checkout's directory.
pub trait Prepare: Send + Sync {
    fn prepare(&self, commit: &str) -> Result<PathBuf, CoreError>;
}

/// The default [`Prepare`]: one detached git worktree at `path`, dedicated
/// to one use (a lane's base checks, a lane's culprit probes), reset before
/// every use with `reset --hard <commit>` and `clean -ffdx`. Only caches
/// outside the checkout survive. Created on first use; a checkout that
/// cannot be reset — a stale `index.lock` left by a killed process, a
/// half-removed directory, one deleted while git still lists it — is
/// removed and created again.
#[derive(Debug, Clone)]
pub struct PristineCheckout {
    /// Any checkout of the repository: the worktree is added from it.
    pub primary: Git,
    pub path: PathBuf,
}

impl PristineCheckout {
    pub fn new(primary: Git, path: impl Into<PathBuf>) -> PristineCheckout {
        PristineCheckout {
            primary,
            path: path.into(),
        }
    }

    fn reset(&self, commit: &str) -> Result<(), CoreError> {
        let wt = self.primary.at(&self.path);
        // an interrupted merge would refuse the reset
        let _ = wt.merge_abort();
        wt.reset_hard(commit).map_err(CoreError::from)?;
        wt.clean_all().map_err(CoreError::from)
    }

    fn create(&self, commit: &str) -> Result<(), CoreError> {
        if let Some(parent) = self.path.parent() {
            fs_err::create_dir_all(parent).map_err(|e| CoreError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        self.primary
            .worktree_add(&self.path, commit, true)
            .map_err(CoreError::from)
    }
}

impl Prepare for PristineCheckout {
    fn prepare(&self, commit: &str) -> Result<PathBuf, CoreError> {
        if self.path.join(".git").exists() && self.reset(commit).is_ok() {
            return Ok(self.path.clone());
        }
        // Missing, or beyond resetting: start over. A directory deleted
        // behind git's back is still registered as a worktree, and `worktree
        // add` refuses it until it is pruned: removing the registration
        // (or, where `remove` fails, pruning) comes first either way.
        self.primary
            .worktree_remove(&self.path)
            .map_err(CoreError::from)?;
        if self.path.exists() {
            fs_err::remove_dir_all(&self.path).map_err(|e| CoreError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        }
        self.create(commit)?;
        self.reset(commit)?;
        Ok(self.path.clone())
    }
}

/// One failed row's run on the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnBase {
    pub row: String,
    pub passed: bool,
    /// The tail of its output there.
    pub output: String,
}

/// What a set of failed rows says about the batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Rows that pass on the base: the batch broke them. Non-empty means the
    /// batch is rejected. In the order given.
    pub broken: Vec<String>,
    /// Rows that fail on the base too, in the order given.
    pub rotten_rows: Vec<String>,
    /// The units each rotten row belongs to: rotten units, sorted, each
    /// with its rotten rows in the order given.
    pub rotten: BTreeMap<Node, Vec<String>>,
    /// Why the base could not judge the rows, when it could not (its setup
    /// failed); every row is then blamed on the batch.
    pub base_unusable: Option<String>,
}

impl Verdict {
    /// Whether the batch broke nothing: every failed row fails on the base
    /// too.
    pub fn batch_clean(&self) -> bool {
        self.broken.is_empty()
    }
}

/// The rule, pure. `failed` are the rows that failed (confirmed) at the gate
/// or re-check, each distinct command once; `rows` gives each unit's rows,
/// so the units of a rotten row can be named; `passed_on_base` answers for
/// each failed row. A row `passed_on_base` does not answer for counts as
/// passing there: the batch is blamed, the conservative side.
pub fn classify(
    failed: &[String],
    rows: &BTreeMap<Node, Vec<String>>,
    passed_on_base: &BTreeMap<String, bool>,
) -> Verdict {
    let mut out = Verdict::default();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for row in failed {
        if !seen.insert(row) {
            continue;
        }
        if passed_on_base.get(row).copied().unwrap_or(true) {
            out.broken.push(row.clone());
        } else {
            out.rotten_rows.push(row.clone());
        }
    }
    for (node, unit_rows) in rows {
        let mine: Vec<String> = out
            .rotten_rows
            .iter()
            .filter(|r| unit_rows.contains(r))
            .cloned()
            .collect();
        if !mine.is_empty() {
            out.rotten.insert(node.clone(), mine);
        }
    }
    out
}

/// What [`judge`] found: the verdict and each row's run on the base, or an
/// interrupt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    Done {
        verdict: Verdict,
        on_base: Vec<OnBase>,
    },
    /// The run was interrupted while the base was being checked: nothing is
    /// decided.
    Interrupted,
}

/// Run the `failed` rows again on `base`: prepare the pristine checkout at
/// `base`, run `setup` (the §5 Bootstrap and Build, those the SDD declares,
/// in order), then each failed row once, one at a time; and [`classify`].
/// Takes gate-pool capacity for its commands like any gate does — the
/// caller's to account. Leaves the checkout at `base` as the rows left it;
/// the next [`Prepare::prepare`] cleans it.
pub fn judge(
    failed: &[String],
    rows: &BTreeMap<Node, Vec<String>>,
    base: &str,
    setup: &[String],
    prepare: &dyn Prepare,
    runner: &dyn Runner,
) -> Result<Judged, CoreError> {
    let distinct: Vec<String> = {
        let mut seen = BTreeSet::new();
        failed
            .iter()
            .filter(|r| seen.insert(r.as_str()))
            .cloned()
            .collect()
    };
    if distinct.is_empty() {
        return Ok(Judged::Done {
            verdict: Verdict::default(),
            on_base: Vec::new(),
        });
    }
    let dir = prepare.prepare(base)?;
    for cmd in setup {
        let ran = runner.run(cmd, &dir);
        if ran.interrupted {
            return Ok(Judged::Interrupted);
        }
        if !ran.ok() {
            let why = format!("`{cmd}` failed on the base (exit {})", ran.code);
            let mut verdict = classify(&distinct, rows, &BTreeMap::new());
            verdict.base_unusable = Some(why);
            return Ok(Judged::Done {
                verdict,
                on_base: Vec::new(),
            });
        }
    }
    let mut on_base = Vec::new();
    let mut passed: BTreeMap<String, bool> = BTreeMap::new();
    for row in &distinct {
        let ran = runner.run(row, &dir);
        if ran.interrupted {
            return Ok(Judged::Interrupted);
        }
        passed.insert(row.clone(), ran.ok());
        on_base.push(OnBase {
            row: row.clone(),
            passed: ran.ok(),
            output: ran.output,
        });
    }
    Ok(Judged::Done {
        verdict: classify(&distinct, rows, &passed),
        on_base,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn ran(code: i32) -> Ran {
        Ran {
            code,
            output: format!("exit {code}"),
            interrupted: false,
        }
    }

    fn rows() -> BTreeMap<Node, Vec<String>> {
        let mut m = BTreeMap::new();
        m.insert(
            Node::new("api", "U-001"),
            vec!["row-a".to_string(), "row-b".to_string()],
        );
        m.insert(Node::new("api", "U-002"), vec!["row-b".to_string()]);
        m.insert(Node::new("core", "U-001"), vec!["row-c".to_string()]);
        m
    }

    /// A preparer that hands out one directory and counts its calls.
    struct Fixed(Mutex<Vec<String>>);

    impl Prepare for Fixed {
        fn prepare(&self, commit: &str) -> Result<PathBuf, CoreError> {
            self.0.lock().unwrap().push(commit.to_string());
            Ok(PathBuf::from("/nowhere"))
        }
    }

    /// A scripted runner: a command passes unless listed as failing; every
    /// call is logged.
    struct Script {
        failing: Vec<&'static str>,
        interrupt: Option<&'static str>,
        log: Mutex<Vec<String>>,
    }

    impl Runner for Script {
        fn run(&self, cmd: &str, _dir: &Path) -> Ran {
            self.log.lock().unwrap().push(cmd.to_string());
            if self.interrupt == Some(cmd) {
                return Ran {
                    code: -1,
                    output: String::new(),
                    interrupted: true,
                };
            }
            ran(if self.failing.contains(&cmd) { 1 } else { 0 })
        }
    }

    fn script(failing: &[&'static str]) -> Script {
        Script {
            failing: failing.to_vec(),
            interrupt: None,
            log: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn a_row_that_passes_on_the_base_is_the_batchs() {
        let base: BTreeMap<String, bool> = [("row-a".to_string(), true)].into();
        let v = classify(&["row-a".to_string()], &rows(), &base);
        assert_eq!(v.broken, vec!["row-a"]);
        assert!(v.rotten.is_empty());
        assert!(!v.batch_clean());
    }

    #[test]
    fn a_row_that_fails_on_the_base_makes_every_unit_it_belongs_to_rotten() {
        let base: BTreeMap<String, bool> =
            [("row-b".to_string(), false), ("row-c".to_string(), true)].into();
        let v = classify(&["row-b".to_string(), "row-c".to_string()], &rows(), &base);
        assert_eq!(v.broken, vec!["row-c"]);
        assert_eq!(v.rotten_rows, vec!["row-b"]);
        assert_eq!(
            v.rotten.keys().cloned().collect::<Vec<_>>(),
            vec![Node::new("api", "U-001"), Node::new("api", "U-002")]
        );
        assert_eq!(v.rotten[&Node::new("api", "U-001")], vec!["row-b"]);
    }

    #[test]
    fn only_rotten_rows_leave_the_batch_clean() {
        let base: BTreeMap<String, bool> = [("row-c".to_string(), false)].into();
        let v = classify(&["row-c".to_string(), "row-c".to_string()], &rows(), &base);
        assert!(v.batch_clean());
        assert_eq!(v.rotten_rows, vec!["row-c"]);
        assert_eq!(
            v.rotten.keys().cloned().collect::<Vec<_>>(),
            vec![Node::new("core", "U-001")]
        );
    }

    #[test]
    fn a_row_with_no_answer_from_the_base_is_blamed_on_the_batch() {
        let v = classify(&["row-a".to_string()], &rows(), &BTreeMap::new());
        assert_eq!(v.broken, vec!["row-a"]);
    }

    #[test]
    fn judge_runs_setup_then_each_failed_row_once_on_the_base() {
        let prep = Fixed(Mutex::new(Vec::new()));
        let run = script(&["row-b"]);
        let failed = vec![
            "row-a".to_string(),
            "row-b".to_string(),
            "row-a".to_string(),
        ];
        let setup = vec!["bootstrap".to_string(), "build".to_string()];
        let Judged::Done { verdict, on_base } =
            judge(&failed, &rows(), "base1", &setup, &prep, &run).unwrap()
        else {
            panic!("interrupted");
        };
        assert_eq!(*prep.0.lock().unwrap(), vec!["base1"]);
        assert_eq!(
            *run.log.lock().unwrap(),
            vec!["bootstrap", "build", "row-a", "row-b"]
        );
        assert_eq!(verdict.broken, vec!["row-a"]);
        assert_eq!(verdict.rotten_rows, vec!["row-b"]);
        assert_eq!(on_base.len(), 2);
        assert!(on_base[0].passed && !on_base[1].passed);
    }

    #[test]
    fn a_base_that_does_not_build_blames_the_batch() {
        let prep = Fixed(Mutex::new(Vec::new()));
        let run = script(&["build", "row-a"]);
        let setup = vec!["build".to_string()];
        let Judged::Done { verdict, .. } =
            judge(&["row-a".to_string()], &rows(), "b", &setup, &prep, &run).unwrap()
        else {
            panic!("interrupted");
        };
        assert_eq!(verdict.broken, vec!["row-a"]);
        assert!(verdict.base_unusable.unwrap().contains("`build` failed"));
        assert_eq!(*run.log.lock().unwrap(), vec!["build"]);
    }

    #[test]
    fn an_interrupt_decides_nothing() {
        let prep = Fixed(Mutex::new(Vec::new()));
        let mut run = script(&[]);
        run.interrupt = Some("row-a");
        let j = judge(&["row-a".to_string()], &rows(), "b", &[], &prep, &run).unwrap();
        assert_eq!(j, Judged::Interrupted);
    }

    #[test]
    fn nothing_failed_runs_nothing() {
        let prep = Fixed(Mutex::new(Vec::new()));
        let run = script(&[]);
        let j = judge(&[], &rows(), "b", &["build".to_string()], &prep, &run).unwrap();
        assert!(matches!(j, Judged::Done { verdict, .. } if verdict.batch_clean()));
        assert!(prep.0.lock().unwrap().is_empty());
        assert!(run.log.lock().unwrap().is_empty());
    }
}
