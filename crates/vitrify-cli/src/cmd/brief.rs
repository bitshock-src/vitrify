//! `vitrify brief [<task>]`: with no task, the procedure index; with one,
//! the operative procedure — disk override else embedded.
//! `build-unit --unit --holder` emits the full generated worker prompt: the
//! filled `vitrify-job` block, the brief's delegation contract, and the
//! unit's resolved reading set inlined.
//!
//! Every brief names the binary's version (`{version}`); `brief run` also
//! names the configured status-check interval (`{status_check_seconds}`).
//!
//! Every path a brief names is the real one for this repository: `{sdd}` is
//! the SDD's root relative to the checkout (so `--sdd {sdd}` works from the
//! primary checkout and from any worktree, in a repository with one SDD or
//! many), and `{worktrees}` / `{worktree}` are where vitrify keeps its
//! working copies ([`Layout`]).

use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::config::Overrides;
use vitrify_core::model::{self, Instance};
use vitrify_core::readset;

use crate::cli::BriefPhase;
use crate::git::Git;
use crate::home;
use crate::layout::{self, Layout};
use crate::out::emit;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shown in place of the worktree root when it cannot be resolved (outside
/// a git repository, or with no vitrify home).
const NO_WORKTREES: &str = "<worktrees>";

pub fn run(
    phase: Option<BriefPhase>,
    unit: Option<&str>,
    holder: Option<&str>,
    sdd: Option<&str>,
) -> Result<u8, CoreError> {
    let Some(phase) = phase else {
        if unit.is_some() || holder.is_some() {
            return Err(CoreError::refusal(
                2,
                "error: --unit/--holder apply only to `brief build-unit`",
            ));
        }
        emit(&index());
        return Ok(0);
    };
    if (unit.is_some() || holder.is_some()) && !matches!(phase, BriefPhase::BuildUnit) {
        return Err(CoreError::refusal(
            2,
            "error: --unit/--holder apply only to `brief build-unit`",
        ));
    }
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    let rel = format!("briefs/{}.md", phase.as_str());
    let (body, _origin) = home::resolve(&home_dir, VERSION, &rel)?;

    match (phase, unit, holder) {
        (BriefPhase::BuildUnit, Some(unit), Some(holder)) => {
            let prompt = build_worker_prompt(&body, unit, holder, sdd)?;
            emit(&prompt);
        }
        (BriefPhase::BuildUnit, Some(_), None) | (BriefPhase::BuildUnit, None, Some(_)) => {
            return Err(CoreError::refusal(
                2,
                "error: `brief build-unit` needs both --unit and --holder (or neither)",
            ));
        }
        (BriefPhase::BuildUnit, None, None) => {
            // the template as the worker sees it, its job slots left open
            emit(body.replace("{version}", VERSION).trim_end_matches('\n'))
        }
        _ => {
            let mut body = body;
            if matches!(phase, BriefPhase::Run) {
                // a bad value is an error naming the key and its file
                let cfg = super::load_config(&Overrides::default())?;
                body = body.replace(
                    "{status_check_seconds}",
                    &cfg.run.status_check_seconds.to_string(),
                );
            }
            let worktrees = here_layout()
                .map(|l| l.wt_root.display().to_string())
                .unwrap_or_else(|| NO_WORKTREES.to_string());
            emit(
                body.replace("{worktrees}", &worktrees)
                    .replace("{version}", VERSION)
                    .trim_end_matches('\n'),
            );
        }
    }
    Ok(0)
}

/// What each task is for, and who follows it — the index's two columns.
fn describe(phase: BriefPhase) -> (&'static str, &'static str) {
    match phase {
        BriefPhase::Onboard => (
            "take a repository from nothing to ready-to-build, or name the gaps",
            "session",
        ),
        BriefPhase::Authoring => (
            "a PRD to an Approved TDD and an Active, lint-clean SDD",
            "session",
        ),
        BriefPhase::Change => (
            "change TDDs and SDDs, one or many, and record what each change stales",
            "session",
        ),
        BriefPhase::Run => ("start, watch and read a `vitrify run` build", "session"),
        BriefPhase::Kickoff => (
            "drive the build loop from a session instead of `vitrify run`",
            "session",
        ),
        BriefPhase::BuildUnit => (
            "build one unit (--unit --holder prints a worker's whole prompt)",
            "workers",
        ),
        BriefPhase::Integrate => (
            "take one built unit through the gate to Verified",
            "integrator",
        ),
    }
}

/// The procedure index `vitrify brief` prints with no task.
fn index() -> String {
    let mut out = format!("vitrify {VERSION} procedures — print one with: vitrify brief <task>\n");
    for phase in BriefPhase::ALL {
        let (what, who) = describe(phase);
        out.push_str(&format!("\n  {:<12}{:<71}{}", phase.as_str(), what, who));
    }
    out
}

/// The layout of the repository the verb runs in, when there is one.
fn here_layout() -> Option<Layout> {
    let primary = Git::new(super::cwd()).primary_root().ok()?;
    let cfg = super::load_config(&Overrides::default()).ok()?;
    Layout::resolve(
        &primary,
        super::home_dir().as_deref(),
        &cfg.run.worktree_dir,
    )
    .ok()
}

/// Fill the `vitrify-job` block and inline the resolved reading set — the
/// four-part delegation contract. The manual entry: the SDD comes from
/// `--sdd` or discovery in the checkout the verb runs in.
pub fn build_worker_prompt(
    template: &str,
    unit_id: &str,
    holder: &str,
    sdd: Option<&str>,
) -> Result<String, CoreError> {
    let root = super::repo_root();
    let inst = Instance::load(super::resolve_sdd(&root, sdd, &super::cwd())?)?;
    let layout = here_layout();
    // a worker starting by hand is told what an earlier attempt left behind
    let session = match (&layout, inst.unit(unit_id)) {
        (Some(layout), Some(unit)) => {
            let next = next_seq(unit);
            let git = Git::new(&layout.primary);
            crate::run::retry::previous_attempt(&git, &inst, unit_id, next)
                .ok()
                .flatten()
                .map(|prev| crate::run::retry::section(None, Some(&prev), &[], None))
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    fill(
        &root,
        &inst,
        template,
        unit_id,
        holder,
        layout.as_ref(),
        &session,
    )
}

/// The orchestrator's entry: the prompt for a unit whose claim is made and
/// whose worktree is `root`. Everything — the claim's sequence, the reading
/// set, the paths the worker is shown — is read from that worktree, at the
/// claim commit it was forked from. `session` is the orchestrator's note to
/// this worker (what is already done for it, what an earlier attempt left).
pub fn build_worker_prompt_at(
    root: &Path,
    unit_id: &str,
    holder: &str,
    sdd_rel: &Path,
    layout: &Layout,
    session: &str,
) -> Result<String, CoreError> {
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    let (template, _) = crate::home::resolve(&home_dir, VERSION, "briefs/build-unit.md")?;
    let inst = Instance::load(super::resolve_sdd_in(root, sdd_rel)?)?;
    fill(
        root,
        &inst,
        &template,
        unit_id,
        holder,
        Some(layout),
        session,
    )
}

/// The claim sequence the unit's work branch carries: the current claim's
/// when Claimed, else the one a fresh claim will take.
fn next_seq(unit: &model::Unit) -> i64 {
    let seq = unit.meta.get_int("claim_seq");
    if unit.meta.get_str_or("status", "Draft") == "Claimed" {
        seq
    } else {
        seq + 1
    }
}

fn fill(
    root: &Path,
    inst: &Instance,
    template: &str,
    unit_id: &str,
    holder: &str,
    layout: Option<&Layout>,
    session: &str,
) -> Result<String, CoreError> {
    let Some(unit) = inst.unit(unit_id) else {
        return Err(CoreError::refusal(
            4,
            format!("error: unit not found: {unit_id}"),
        ));
    };
    let cfg = super::load_config(&Overrides::default())?;

    let seq = next_seq(unit);
    let slug = layout::sdd_slug(inst);
    let branch = layout::unit_branch(&slug, unit_id, seq);
    let sdd = match super::sdd_rel(root, &inst.root) {
        Some(rel) => super::sdd_arg(&rel),
        None => inst.root.display().to_string(),
    };
    let (worktrees, worktree) = match layout {
        Some(l) => (
            l.wt_root.display().to_string(),
            l.unit_worktree(&slug, unit_id, seq).display().to_string(),
        ),
        None => (
            NO_WORKTREES.to_string(),
            format!("{NO_WORKTREES}/{slug}-{unit_id}-c{seq}"),
        ),
    };

    let session = session.trim_matches('\n');
    let mut template = template.to_string();
    if session.is_empty() {
        // the slot and its paragraph break leave nothing behind
        template = template
            .replace("{session}\n\n", "")
            .replace("{session}\n", "")
            .replace("{session}", "");
    } else if !template.contains("{session}") {
        // an overridden brief without the slot still carries the note
        template.push_str("\n\n{session}\n");
    }
    // the session note goes in last, verbatim: it may quote a gate's output
    let mut prompt = template
        .replace("{unit}", unit_id)
        .replace("{holder}", holder)
        .replace("{branch}", &branch)
        .replace("{sdd}", &sdd)
        .replace("{worktrees}", &worktrees)
        .replace("{worktree}", &worktree)
        .replace("{lease_minutes}", &cfg.run.lease_minutes.to_string())
        .replace("{version}", VERSION)
        .replace("{session}", session);

    // a pinned contract's authority is looked up among the SDDs of the
    // checkout the prompt is generated in — a worker's own worktree, at its
    // claim commit, under `vitrify run`
    let items = readset::resolve(inst, unit_id, &super::peers(root))?;
    prompt = prompt.trim_end_matches('\n').to_string();
    prompt.push_str("\n\n## Resolved reading set (read these, nothing else)\n");
    for (i, item) in items.iter().enumerate() {
        let region_note = item
            .region
            .as_deref()
            .map(|r| format!("  ::  {r}"))
            .unwrap_or_default();
        prompt.push_str(&format!(
            "\n### Reading {} — {}{}\n\n",
            i + 1,
            item.path.display(),
            region_note
        ));
        let text = readset::text_of(inst, item)?;
        prompt.push_str(text.trim_end_matches('\n'));
        prompt.push('\n');
    }
    Ok(prompt.trim_end_matches('\n').to_string())
}
