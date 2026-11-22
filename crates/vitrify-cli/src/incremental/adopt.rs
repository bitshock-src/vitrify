//! Adoption of attempts that were built but never landed.
//!
//! A run that is interrupted, or a lease that lapses while the work waits
//! for the gate, leaves an attempt that its worker finished: the branch
//! carries the unit's Built stamp and the trace it was built from. Sending
//! that unit back to a worker would pay for a model session to write what is
//! already written. Instead the attempt is **adopted**: the scheduler claims
//! the unit again, [`relabel`] makes the attempt's work the new claim's
//! branch, and the unit goes straight to the gate with no worker session.
//!
//! An attempt is adoptable ([`adoptable`]) when
//!
//! * its worker is gone — the run that made it ended or died. That is the
//!   caller's to decide: this module only looks at git;
//! * its branch — the newest attempt branch of the unit since the unit was
//!   last verified — carries a Built stamp for the unit, with a recorded
//!   trace, at its tip;
//! * that trace still matches the unit's reading set at the current mainline
//!   head: nothing it was built from has moved.
//!
//! An attempt the gate (or its worker) already judged is not adopted: one set
//! aside as `rejected-gate`, `rejected-write-set`, `ownership-conflict`,
//! `blocked` or `released` would only be judged the same way again, and a
//! rejected adoption — set aside on the new claim's branch — would otherwise
//! be adopted again, forever. Those go to a worker, as today. An attempt
//! that changed paths outside the unit's write-set is not adopted either:
//! the gate would reject it, and its files go to the next worker as a draft
//! instead.
//!
//! # Re-labelling
//!
//! The attempt's own commit cannot simply be given the new claim's branch
//! name. It forked from the old claim's commit, and mainline has rewritten
//! the unit's fencing fields twice since (the release, the new claim), next
//! to the status line the attempt changed: the gate's merge would conflict
//! in the unit's own front matter. [`relabel`] therefore builds the new
//! claim's branch the way a worker would have: forked from the new claim's
//! commit, with one commit that carries the attempt's work — the files the
//! unit owns that the attempt changed, as the attempt left them, and the
//! unit's handoff fields (`status`, `built`, `trace`) from its Built stamp.
//! Nothing else is carried, so the branch is exactly the attempt's work,
//! relative to the new claim.
//!
//! Fencing holds. The token is the claim sequence the branch name carries;
//! the old claim's branch keeps the old sequence, which the gate's fencing
//! check rejects, and only the deliberate new claim's branch — made by
//! [`relabel`], which refuses an existing name — can land. The gate then runs
//! as for any attempt: fencing, the write-set check against the new claim,
//! the input check, every row. Adoption skips the model, never the gate.

use std::path::{Path, PathBuf};

use vitrify_core::model::{self, Instance, Shape};
use vitrify_core::writeset::WriteSet;
use vitrify_core::{CoreError, journal, trace, yamlish};

use crate::cmd::resolve_sdd_in;
use crate::git::Git;
use crate::layout;

/// First words of the commit that sets an attempt aside (as the scheduler
/// writes it: `sdd: abandoned attempt <unit> (c<n>) — <outcome>`).
const ABANDONED: &str = "sdd: abandoned attempt";

/// Outcomes that are a verdict on the work itself: an attempt set aside
/// with one of these is not adopted.
const JUDGED: [&str; 5] = [
    "rejected-gate",
    "rejected-write-set",
    "ownership-conflict",
    "blocked",
    "released",
];

/// An attempt that can go to the gate without a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub unit: String,
    /// Its branch, `sdd/<slug>/<unit>/c<seq>`.
    pub branch: String,
    /// The claim sequence its branch carries: the old token.
    pub seq: i64,
    /// The commit at the branch's tip.
    pub commit: String,
    /// Where the attempt left mainline: its old claim's commit.
    pub fork: String,
    /// The paths the unit owns that the attempt changed since `fork`
    /// (deletions included), sorted.
    pub paths: Vec<String>,
    /// The SDD's root relative to the checkout root, as `--sdd` names it.
    pub sdd_rel: PathBuf,
    /// The Built stamp's `built` text.
    pub built: String,
    /// The Built stamp's recorded trace.
    pub trace: Vec<(String, String)>,
}

/// Why a unit has no adoptable attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No attempt branch since the unit was last verified.
    NoBranch,
    /// The newest attempt was set aside with a verdict on its work.
    Judged { branch: String, outcome: String },
    /// The branch's tip does not stamp the unit Built with a trace.
    NotBuilt { branch: String, status: String },
    /// The attempt changed paths outside the unit's write-set, sorted.
    WriteSet { branch: String, paths: Vec<String> },
    /// Inputs the attempt was built from have moved since: the trace keys
    /// that no longer match (`unit`, a contract key, `global`), sorted.
    TraceMoved { branch: String, keys: Vec<String> },
}

impl Refusal {
    /// One human line: why the unit goes to a worker instead.
    pub fn describe(&self) -> String {
        match self {
            Refusal::NoBranch => "no attempt since it was last verified".to_string(),
            Refusal::Judged { branch, outcome } => {
                format!("its newest attempt ({branch}) was set aside as {outcome}")
            }
            Refusal::NotBuilt { branch, status } => {
                format!("its newest attempt ({branch}) is not stamped Built (status {status})")
            }
            Refusal::WriteSet { branch, paths } => format!(
                "its newest attempt ({branch}) changed paths outside the unit's write-set: {}",
                paths.join(", ")
            ),
            Refusal::TraceMoved { branch, keys } => format!(
                "its newest attempt ({branch}) was built from inputs that have moved: {}",
                keys.join(", ")
            ),
        }
    }
}

/// The adoptable attempt of `unit`, or why there is none. `inst` is the SDD
/// as the current mainline head holds it, loaded from the checkout at `root`
/// (checked out at that head); `git` reads the repository's branches (any
/// checkout of it will do). The caller has established that no worker of a
/// live run holds the attempt.
pub fn adoptable(
    git: &Git,
    inst: &Instance,
    root: &Path,
    unit: &str,
) -> Result<Result<Attempt, Refusal>, CoreError> {
    let Some(u) = inst.unit(unit) else {
        return Err(CoreError::refusal(
            4,
            format!("error: unit {unit} not found at the mainline head"),
        ));
    };
    let slug = layout::sdd_slug(inst);
    let Some((seq, branch)) = newest_attempt(git, inst, &slug, unit)? else {
        return Ok(Err(Refusal::NoBranch));
    };
    let commit = git.rev_parse(&branch).map_err(CoreError::from)?;
    let message = git.commit_message(&commit).unwrap_or_default();
    let subject = message.lines().next().unwrap_or("");
    if let Some(rest) = subject.strip_prefix(ABANDONED)
        && let Some((_, outcome)) = rest.rsplit_once(" — ")
        && JUDGED.contains(&outcome.trim())
    {
        return Ok(Err(Refusal::Judged {
            branch,
            outcome: outcome.trim().to_string(),
        }));
    }
    // the Built stamp at the tip, as the branch has the unit's entry
    let ws = WriteSet::of(inst, u, root)?;
    let stamped = git
        .show_text(&commit, &ws.spec_file)
        .map_err(CoreError::from)?
        .and_then(|text| unit_meta_in(inst.shape, &text, unit))
        .unwrap_or_default();
    let status = stamped.get_str_or("status", "Draft").to_string();
    let recorded = stamped.get_map("trace").cloned().unwrap_or_default();
    let built = stamped.get_str_or("built", "").trim().to_string();
    if status != "Built" || recorded.is_empty() {
        return Ok(Err(Refusal::NotBuilt {
            branch,
            status: if recorded.is_empty() && status == "Built" {
                "Built, no trace".to_string()
            } else {
                status
            },
        }));
    }
    // the write-set check the gate will make, so a sure rejection costs no
    // claim
    let head = git.at(root).rev_parse("HEAD").map_err(CoreError::from)?;
    let fork = git.merge_base(&head, &commit).map_err(CoreError::from)?;
    let changed = git.changed_paths(&fork, &commit).map_err(CoreError::from)?;
    let versions = || {
        (
            git.show_text(&fork, &ws.spec_file).ok().flatten(),
            git.show_text(&commit, &ws.spec_file).ok().flatten(),
        )
    };
    let mut outside: Vec<String> = ws
        .violations(&changed, versions)
        .iter()
        .map(|v| v.path().to_string())
        .collect();
    if !outside.is_empty() {
        outside.sort();
        outside.dedup();
        return Ok(Err(Refusal::WriteSet {
            branch,
            paths: outside,
        }));
    }
    // the trace against the reading set the head holds now
    let (current, problems) = trace::compute(inst, u)?;
    if !problems.is_empty() {
        return Err(CoreError::refusal(
            4,
            problems
                .iter()
                .map(|p| format!("error: {p}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    let mut keys: Vec<String> = trace::verify(&recorded, &current)
        .into_iter()
        .map(|m| m.key)
        .collect();
    if !keys.is_empty() {
        keys.sort();
        keys.dedup();
        return Ok(Err(Refusal::TraceMoved { branch, keys }));
    }
    let mut paths: Vec<String> = changed.into_iter().filter(|p| ws.covers(p)).collect();
    paths.sort();
    paths.dedup();
    let sdd_rel = inst
        .root
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| inst.root.clone());
    Ok(Ok(Attempt {
        unit: unit.to_string(),
        branch,
        seq,
        commit,
        fork,
        paths,
        sdd_rel,
        built,
        trace: recorded,
    }))
}

/// Re-label `attempt` as the work of the new claim `new_seq`, whose claim
/// transaction made `claim_commit`: create the branch
/// `sdd/<slug>/<unit>/c<new_seq>` at `claim_commit` and commit on it the
/// attempt's work (see the module doc), in a scratch worktree at `scratch`
/// — where the new claim's worker would have worked
/// (`Layout::unit_worktree`) — that is removed again. Returns the branch.
///
/// Refuses, creating nothing, when the branch or `scratch` already exists,
/// or `new_seq` does not supersede the attempt's claim. On a later failure
/// the branch and the scratch worktree are removed again. The old branch is
/// left as it is: its token can never land.
pub fn relabel(
    primary: &Git,
    slug: &str,
    attempt: &Attempt,
    new_seq: i64,
    claim_commit: &str,
    scratch: &Path,
) -> Result<String, CoreError> {
    let refuse = |why: String| {
        Err(CoreError::refusal(
            1,
            format!(
                "error: cannot adopt {} (c{}) under claim c{new_seq}: {why}",
                attempt.unit, attempt.seq
            ),
        ))
    };
    if new_seq <= attempt.seq {
        return refuse("the new claim must supersede it".to_string());
    }
    let branch = layout::unit_branch(slug, &attempt.unit, new_seq);
    if primary.branch_exists(&branch).map_err(CoreError::from)? {
        return refuse(format!("branch {branch} already exists"));
    }
    if scratch.exists() {
        return refuse(format!("{} already exists", scratch.display()));
    }
    if let Some(parent) = scratch.parent() {
        fs_err::create_dir_all(parent).map_err(|source| CoreError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    primary
        .worktree_add_branch(scratch, &branch, claim_commit)
        .map_err(CoreError::from)?;
    let carried = carry(primary, attempt, new_seq, scratch);
    let _ = primary.worktree_remove(scratch);
    if let Err(e) = carried {
        let _ = primary.delete_branch(&branch);
        return Err(e);
    }
    Ok(branch)
}

/// Commit the attempt's work in the worktree `scratch`, on the new branch.
fn carry(primary: &Git, attempt: &Attempt, new_seq: i64, scratch: &Path) -> Result<(), CoreError> {
    let wt = primary.at(scratch);
    let mut present: Vec<String> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    for p in &attempt.paths {
        let there = wt
            .verify_ref(&format!("{}:{p}", attempt.commit))
            .map_err(CoreError::from)?;
        if there.is_some() {
            present.push(p.clone());
        } else {
            gone.push(p.clone());
        }
    }
    wt.restore_paths(&attempt.commit, &present)
        .map_err(CoreError::from)?;
    for p in &gone {
        let path = scratch.join(p);
        if path.exists() {
            fs_err::remove_file(&path).map_err(|source| CoreError::Io {
                path: path.clone(),
                source,
            })?;
        }
    }
    // the handoff fields, from the attempt's stamp, onto the new claim's entry
    let inst = Instance::load(resolve_sdd_in(scratch, &attempt.sdd_rel)?)?;
    let Some(unit) = inst.unit(&attempt.unit) else {
        return Err(CoreError::refusal(
            4,
            format!(
                "error: unit {} not found at the new claim's commit",
                attempt.unit
            ),
        ));
    };
    model::rewrite_unit_meta(&inst.handle(), unit, |inner| {
        let b = yamlish::set_scalar_in_place(inner, "status", "Built");
        let b = yamlish::set_scalar_in_place(&b, "built", &attempt.built);
        yamlish::set_map_in_place(&b, "trace", &attempt.trace)
    })?;
    wt.add_all().map_err(CoreError::from)?;
    wt.commit_note(
        &format!(
            "sdd: adopt {} (c{}) as c{new_seq}",
            attempt.unit, attempt.seq
        ),
        &format!(
            "Built work of {} adopted from {} ({}), with no worker session.",
            attempt.unit,
            attempt.branch,
            &attempt.commit[..7.min(attempt.commit.len())]
        ),
    )
    .map_err(CoreError::from)
}

/// The newest attempt branch of `unit` made after the unit was last
/// verified (its sequence and name): earlier ones were superseded by that
/// verification.
fn newest_attempt(
    git: &Git,
    inst: &Instance,
    slug: &str,
    unit: &str,
) -> Result<Option<(i64, String)>, CoreError> {
    let (events, _) = journal::read(&inst.handle())?;
    let verified_at = events
        .iter()
        .filter(|e| e.event == "verified" && e.unit.as_deref() == Some(unit))
        .filter_map(|e| e.claim_seq)
        .max()
        .unwrap_or(0);
    let mut best: Option<(i64, String)> = None;
    for branch in git
        .branches_under(&layout::branch_prefix(slug, unit))
        .map_err(CoreError::from)?
    {
        let Some(seq) = layout::branch_seq(&branch) else {
            continue;
        };
        if seq > verified_at && best.as_ref().is_none_or(|(s, _)| seq > *s) {
            best = Some((seq, branch));
        }
    }
    Ok(best)
}

/// A unit's front matter as `text` — its unit file, or the single-file SDD
/// holding its region — has it.
fn unit_meta_in(shape: Shape, text: &str, uid: &str) -> Option<model::Meta> {
    match shape {
        Shape::Bundle => model::front_matter(text),
        Shape::Single => {
            let (start, end) = model::find_unit_span(text, uid)?;
            let region = &text[start..end];
            let (_, _, inner_start, inner_end) = model::find_meta_block_span(region)?;
            Some(model::Meta(yamlish::parse(&region[inner_start..inner_end])))
        }
    }
}
