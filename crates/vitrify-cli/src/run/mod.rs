//! `vitrify run` — the deterministic orchestrator: preflight, continuous
//! dispatch, batched serial integration, termination reports. Control flow
//! is Rust; inference happens only at the three enumerated points: workers,
//! escalation, and plan-time authoring (outside `run`).
//! Resume = re-derivation from repo state, never memory.
//!
//! A run builds its SDDs as lanes, one per SDD ([`lane`]), under one
//! scheduler ([`sched`]). A plain `vitrify run` in a repository of several
//! SDDs is a **project run**: every Active SDD is a lane (`--sdd` with
//! `--upstream`: that SDD and those it depends on). One SDD — the only one,
//! or the one `--sdd` names — is built exactly as it always was. What the lanes share is the run's: the worker
//! cap, the gate pool, the event log, the run record. Preflight has three
//! parts: the project's (before anything runs), each lane's static checks
//! (at the start), and each lane's dynamic checks (its §5 commands in its
//! own checkout, before its first build, then a re-check of the units a
//! change reached since their last recorded check — and only those).

pub mod escalate;
pub mod events;
pub mod history;
pub mod integrate;
mod lane;
pub mod lease;
pub mod mainline;
mod plan;
pub mod pool;
pub mod progress;
pub mod project;
pub mod retry;
mod sched;
pub mod worker;

use std::path::{Path, PathBuf};

use vitrify_core::config::{Config, Overrides};
use vitrify_core::harness::Role;
use vitrify_core::model::{self, Instance, Shape, UtcTime};
use vitrify_core::{CoreError, audit};

use crate::cmd;
use crate::gate::{self, ShRunner};
use crate::git::{Git, resolve_mainline, use_push};
use crate::home;
use crate::incremental::dirty::{Cascade, Mode, Snapshot};
use crate::incremental::recheck;
use crate::layout::{self, Layout};
use crate::out::{J, diag, emit};
use crate::signals;
use integrate::RottenRow;
use mainline::Mainline;
use pool::Pool;

pub struct RunFlags {
    pub dry_run: bool,
    pub cap: Option<i64>,
    pub sdd: Option<String>,
    pub mainline: Option<String>,
    /// Override `run.gate_jobs`.
    pub gate_jobs: Option<i64>,
    /// Stay alive when only blocked units remain, and resume when the spec
    /// changes.
    pub wait: bool,
    /// Trust no recorded check: every Verified unit's rows and the §5 Test
    /// command run again. Nothing is rebuilt for it, and built attempts are
    /// still adopted — they pass the full gate either way.
    pub recheck: bool,
    /// With `sdd`: build the SDDs it depends on too, each a lane.
    pub upstream: bool,
}

/// What every lane of a run shares: the repository, the configuration, the
/// harness, the caps, the job.
pub(crate) struct Ctx {
    /// The primary checkout.
    pub primary: Git,
    pub layout: Layout,
    /// The mainline branch.
    pub branch: String,
    /// Publish by `git push` (origin arbitrates) rather than a local CAS.
    pub push: bool,
    pub cfg: Config,
    /// Model sessions at once, across every lane.
    pub cap: usize,
    /// Commands at once — §5 commands and Verification rows — across every
    /// gate, head check and preflight of the run.
    pub gate_pool: &'static Pool,
    pub home: PathBuf,
    pub harness: String,
    /// The headless spelling the probe recorded for the selected harness.
    pub argv: Vec<String>,
    /// `argv` + the worker role's model/effort flags from config.toml.
    pub worker_argv: Vec<String>,
    /// `argv` + the retry role's flags (the worker's, unless config.toml
    /// names a retry profile).
    pub retry_argv: Vec<String>,
    /// `argv` + the escalation role's model/effort flags.
    pub escalation_argv: Vec<String>,
    /// Per-role summaries for the job header.
    pub roles: Vec<String>,
    /// The role profiles in config.toml failed to read or parse (a
    /// preflight failure).
    pub profile_error: Option<String>,
    pub sdd_arg: Option<String>,
    pub job_id: String,
    /// `.vitrify/logs/<job>/`: the job's own records.
    pub logs_dir: PathBuf,
    pub wait: bool,
    /// `--recheck`: no recorded check is trusted; preflight re-checks every
    /// Verified unit and the §5 Test command.
    pub recheck: bool,
    /// `--upstream`.
    pub upstream: bool,
    /// A project run: lanes are named on every line, the header, report and
    /// preflight are the project's, and a lane whose preflight fails is
    /// blocked instead of refusing the run.
    pub project: bool,
}

impl Ctx {
    /// The mainline handle of a lane: the SDD at `sdd_rel`, named `slug`,
    /// in this run's repository.
    pub fn mainline_for(&self, sdd_rel: PathBuf, slug: String) -> Mainline {
        Mainline {
            primary: self.primary.clone(),
            layout: self.layout.clone(),
            sdd_rel,
            slug,
            branch: self.branch.clone(),
            push: self.push,
        }
    }
}

/// One lane as preflight leaves it, for the scheduler to start.
pub(crate) struct LaneSeed {
    /// The SDD, pinned: its path, its slug, the repository it is built in.
    pub ml: Mainline,
    /// The SDD as the primary checkout held it at preflight.
    pub inst: Instance,
    /// Workers at once in this lane: 1 for a single-file SDD, whose every
    /// status edit shares one file.
    pub cap: usize,
    /// Rows the preflight re-check found failing: their units are rotten
    /// from the start.
    pub rotten: Vec<RottenRow>,
    /// Why its preflight failed, in a project run: the lane is blocked, and
    /// so is every unit that depends on its units.
    pub blocked: Vec<String>,
    /// Units the preflight re-check checked, and whether it ran the §5 Test
    /// command: under `--recheck` they need no second check.
    pub checked: Vec<String>,
    pub suite_ran: bool,
}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(flags: RunFlags) -> Result<u8, CoreError> {
    let here = Git::new(cmd::cwd());
    let root = here.primary_root().map_err(CoreError::from)?;
    let primary = Git::new(&root);
    let cfg = cmd::load_config(&Overrides {
        mainline: flags.mainline.clone(),
        gate_jobs: flags.gate_jobs,
        ..Default::default()
    })?;
    let mainline = resolve_mainline(&primary, flags.mainline.as_deref(), &cfg.mainline)
        .map_err(CoreError::from)?;
    let push = use_push(&primary, &mainline).map_err(CoreError::from)?;
    let Some(home) = cmd::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    // the home holds the probe record, the role profiles and the worktrees:
    // `vitrify init` sets it up, and nothing here creates it
    home::require_set_up(&home)?;
    let layout = Layout::resolve(&root, Some(&home), &cfg.run.worktree_dir)?;

    // harness: preferred, else the first the probe recorded of claude,
    // codex, opencode
    let (harness, argv) = select_harness(&home, &cfg);
    let harness = harness.unwrap_or_default();
    let argv = argv.unwrap_or_default();
    // per-role model/effort: the [harness.<name>] tables of config.toml,
    // a repository's own config.toml overriding them key by key
    let (worker_argv, retry_argv, escalation_argv, roles, profile_error) =
        match cmd::probe::role_profiles(&home, Some(&root)) {
            Ok(profiles) => {
                let profile = profiles.get(&harness);
                (
                    profiles.argv_for(&harness, &argv, Role::Worker),
                    profiles.argv_for(&harness, &argv, Role::Retry),
                    profiles.argv_for(&harness, &argv, Role::Escalation),
                    Role::ALL
                        .iter()
                        .filter(|r| profile.is_own(**r))
                        .map(|r| format!("{} {}", r.as_str(), profile.describe(*r)))
                        .collect(),
                    None,
                )
            }
            Err(e) => (
                argv.clone(),
                argv.clone(),
                argv.clone(),
                Vec::new(),
                Some(e.to_string()),
            ),
        };

    let now = UtcTime::now();
    let job_id = format!(
        "run-{}-p{}",
        now.format().replace(['-', ':'], ""),
        std::process::id()
    );
    let logs_dir = layout.job_logs(&job_id);
    // one bound on the commands the run starts, whichever lane starts them
    let gate_pool = gate::install_pool(cfg.run.rows_at_once());

    let mut ctx = Ctx {
        primary,
        layout,
        branch: mainline,
        push,
        cap: flags.cap.unwrap_or(cfg.run.worker_cap).max(1) as usize,
        gate_pool,
        cfg,
        home,
        harness,
        argv,
        worker_argv,
        retry_argv,
        escalation_argv,
        roles,
        profile_error,
        sdd_arg: flags.sdd.clone(),
        job_id,
        logs_dir,
        wait: flags.wait,
        recheck: flags.recheck,
        upstream: flags.upstream,
        project: false,
    };

    // From here an interrupt is ours to handle: the commands preflight runs
    // and the workers the loop starts are stopped by vitrify, not left
    // running behind a dead parent.
    signals::install();
    crate::cmd::claim::quiet_resync(true);

    // A primary checkout that an earlier commit could not move is behind
    // mainline: it catches up before preflight reads it (a dry run only
    // says so).
    let behind = if flags.dry_run {
        crate::cmd::claim::behind_note(&ctx.primary, &ctx.branch)
    } else {
        let _turn = crate::cmd::claim::tx_lock();
        crate::cmd::claim::catch_up_primary(&ctx.primary, &ctx.branch)
    };
    if let Some(note) = behind {
        diag(&note);
    }

    // ---------------- preflight: refuse to start on any failure
    let mut failures: Vec<String> = Vec::new();
    let lanes = preflight(&mut ctx, &mut failures, flags.dry_run)?;
    if let Some(sig) = signals::pending() {
        diag(&format!(
            "interrupted ({}) during preflight",
            signals::name(sig)
        ));
        signals::die_by(sig);
    }
    let lanes = match lanes {
        Ok(lanes) => lanes,
        Err(code) => {
            for fail in &failures {
                diag(&format!("preflight failed: {fail}"));
            }
            return Ok(code);
        }
    };

    // ---------------- job header BEFORE dispatch
    // A dry run records no job: its plan names none, so the same repository
    // state and configuration print the same plan.
    let plan = plan::build(
        &ctx,
        &lanes,
        if flags.dry_run {
            "dry-run"
        } else {
            &ctx.job_id
        },
    );
    if ctx.project {
        let lines = if flags.dry_run {
            project::dry_run(&plan)
        } else {
            project::header(&plan, true)
        };
        for line in lines {
            emit(&line);
        }
    } else {
        for lane in &lanes {
            print_header(&ctx, lane, !flags.dry_run);
        }
    }
    if flags.dry_run {
        for lane in &lanes {
            for line in lane::dead_run_notes(&ctx, &lane.inst) {
                emit(&line);
            }
        }
        let blocked: Vec<&str> = lanes
            .iter()
            .filter(|l| !l.blocked.is_empty())
            .map(|l| l.ml.slug.as_str())
            .collect();
        if !blocked.is_empty() {
            // the run would end Blocked: say so, and exit as it would
            emit(&format!(
                "dry-run: preflight failed for {} — the run would end Blocked (stderr says why)",
                blocked.join(", ")
            ));
            emit(&format!("Start: {}", start_command(&flags)));
            return Ok(2);
        }
        emit("dry-run: preflight green; stopping before dispatch");
        emit(&format!("Start: {}", start_command(&flags)));
        return Ok(0);
    }

    sched::orchestrate(ctx, lanes, plan)
}

/// The command that starts the run a dry run checked: the flags given,
/// without `--dry-run`, in the order the usage line lists them.
fn start_command(flags: &RunFlags) -> String {
    let mut words = vec!["vitrify".to_string(), "run".to_string()];
    if let Some(cap) = flags.cap {
        words.extend(["--cap".to_string(), cap.to_string()]);
    }
    if let Some(jobs) = flags.gate_jobs {
        words.extend(["--gate-jobs".to_string(), jobs.to_string()]);
    }
    if flags.wait {
        words.push("--wait".to_string());
    }
    if flags.recheck {
        words.push("--recheck".to_string());
    }
    if let Some(sdd) = &flags.sdd {
        words.extend(["--sdd".to_string(), shell_word(sdd)]);
    }
    if flags.upstream {
        words.push("--upstream".to_string());
    }
    if let Some(branch) = &flags.mainline {
        words.extend(["--mainline".to_string(), shell_word(branch)]);
    }
    words.join(" ")
}

/// `s` as one shell word: as it is when nothing in it needs quoting, else
/// single-quoted.
pub(crate) fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@+=,%".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// A preflight failure with its fix: ` → <fix>` closes the first line (a
/// cause can carry what it lists on the lines after).
fn with_fix(cause: &str, fix: &str) -> String {
    let cause = cause.strip_prefix("error: ").unwrap_or(cause);
    match cause.split_once('\n') {
        Some((first, rest)) => format!("{first} → {fix}\n{rest}"),
        None => format!("{cause} → {fix}"),
    }
}

/// An SDD that did not resolve or load, with its fix. A refusal that names
/// its next step already keeps its own; one that lists `--sdd` values
/// after a colon keeps the colon after the fix.
fn sdd_failure(cause: &str) -> String {
    let cause = cause.strip_prefix("error: ").unwrap_or(cause);
    let first = cause.lines().next().unwrap_or_default();
    if first.contains(" → ") {
        return cause.to_string();
    }
    let fix = if cause.contains("no SDD found") {
        "vitrify brief authoring"
    } else if cause.contains("multiple SDDs") || cause.contains("several SDDs") {
        "pass one of these --sdd values"
    } else {
        "vitrify locate"
    };
    match first.strip_suffix(':') {
        Some(lead) => format!("{lead} → {fix}:{}", &cause[first.len()..]),
        None => with_fix(cause, fix),
    }
}

fn select_harness(home: &Path, cfg: &Config) -> (Option<String>, Option<Vec<String>>) {
    let order: Vec<&str> = if cfg.harness.preferred.is_empty() {
        vec!["claude", "codex", "opencode"]
    } else {
        vec![cfg.harness.preferred.as_str()]
    };
    for name in order {
        if let Some(argv) = cmd::probe::recorded_headless(home, name) {
            return (Some(name.to_string()), Some(argv));
        }
    }
    (None, None)
}

// ---------------------------------------------------------------- preflight

/// Preflight: which SDDs the run builds, the project's checks, then each
/// lane's static checks, then each lane's dynamic checks. Every failure
/// found is collected. With one lane any failure refuses the run; in a
/// project run a lane whose own checks fail is blocked, and the rest go on.
/// `Err(code)` = refused (or interrupted).
fn preflight(
    ctx: &mut Ctx,
    failures: &mut Vec<String>,
    dry_run: bool,
) -> Result<Result<Vec<LaneSeed>, u8>, CoreError> {
    let chosen = match choose_lanes(ctx, failures)? {
        Ok(chosen) => chosen,
        Err(code) => return Ok(Err(code)),
    };
    let mut lanes: Vec<LaneSeed> = chosen
        .into_iter()
        .map(|(sdd_rel, inst)| {
            let slug = layout::sdd_slug(&inst);
            LaneSeed {
                ml: ctx.mainline_for(sdd_rel, slug),
                inst,
                cap: usize::MAX,
                rotten: Vec::new(),
                blocked: Vec::new(),
                checked: Vec::new(),
                suite_ran: false,
            }
        })
        .collect();

    let usable_root = project_preflight(ctx, &lanes, failures);
    if ctx.project {
        if !failures.is_empty() {
            return Ok(Err(2));
        }
        for lane in &mut lanes {
            let mut own: Vec<String> = Vec::new();
            if !lane_static(ctx, lane, &mut own, dry_run)? {
                return Ok(Err(2));
            }
            // A lane's dynamic preflight runs at its first build or
            // re-check, when the scheduler admits it, and its re-checks in
            // the loop, in dependency order. A dry run has no loop: it runs
            // them here, one lane at a time, and keeps no checkout beyond
            // `run.lane_cap`.
            if dry_run && own.is_empty() && usable_root {
                if !lane_dynamic(ctx, lane, &mut own, dry_run, false)? {
                    return Ok(Err(2));
                }
                if ctx.cfg.run.lane_limit().is_some() {
                    lane.ml.remove_checkouts();
                }
            }
            for f in &own {
                diag(&project::event_line(
                    &lane.ml.slug,
                    &format!("preflight failed: {f}"),
                ));
            }
            lane.blocked = own;
        }
    } else {
        for lane in &lanes {
            if !lane_static(ctx, lane, failures, dry_run)? {
                return Ok(Err(2));
            }
        }
        // a lane's commands run in its own checkout, made under the
        // worktree root: without a usable root there is nowhere to run them
        if usable_root {
            for lane in &mut lanes {
                if !lane_dynamic(ctx, lane, failures, dry_run, true)? {
                    return Ok(Err(2));
                }
            }
        }
    }

    for lane in &mut lanes {
        // single-file shape forces cap 1
        if lane.inst.shape == Shape::Single {
            lane.cap = 1;
            if ctx.cap > 1 && !ctx.project {
                diag(
                    "note: single-file SDD — worker cap forced to 1 (every status edit shares one file)",
                );
            }
        }
    }
    // the run never starts more workers than its lanes can take
    let room = lanes.iter().fold(0usize, |n, l| n.saturating_add(l.cap));
    ctx.cap = ctx.cap.min(room).max(1);

    if failures.is_empty() {
        Ok(Ok(lanes))
    } else {
        Ok(Err(2))
    }
}

/// Which SDDs the run builds, each pinned as its path under the repository
/// root and loaded from the primary checkout: `--sdd`'s, or the only one;
/// in a repository of several, every Active SDD — a project run, refused
/// (exit 4 or 2) when the repository as a whole does not hold together;
/// `--sdd` with `--upstream`: that SDD and the SDDs it depends on.
fn choose_lanes(
    ctx: &mut Ctx,
    failures: &mut Vec<String>,
) -> Result<Result<Vec<(PathBuf, Instance)>, u8>, CoreError> {
    let root = ctx.primary.dir.clone();
    if ctx.upstream && ctx.sdd_arg.is_none() {
        failures.push(with_fix(
            "--upstream adds the SDDs an SDD depends on, and names none",
            "pass --sdd <root> with it",
        ));
        return Ok(Err(2));
    }
    let project_wide = ctx.sdd_arg.is_none() && cmd::discover(&root).len() > 1;
    if !project_wide && !ctx.upstream {
        return Ok(match resolve_lane(ctx, failures)? {
            Some(lane) => Ok(vec![lane]),
            None => Err(2),
        });
    }
    let named = if ctx.upstream {
        match resolve_lane(ctx, failures)? {
            Some(lane) => Some(lane),
            None => return Ok(Err(2)),
        }
    } else {
        None
    };

    // the repository as a whole
    let (all, refused) = cmd::loaded(&root, named.as_ref().map(|(_, i)| i));
    for (_, e) in &refused {
        failures.push(sdd_failure(&e.to_string()));
    }
    if !failures.is_empty() {
        return Ok(Err(2));
    }
    let graph = match project::preflight(&all) {
        Ok(graph) => graph,
        Err(refused) => {
            failures.extend(refused.failures);
            return Ok(Err(refused.code));
        }
    };
    let active = |i: &Instance| i.meta.get_str_or("status", "Draft") == "Active";
    let mut chosen: Vec<&Instance> = match &named {
        Some((_, inst)) => {
            let slugs = project::select_lanes(&graph, vitrify_core::graph::slug_of(inst), true)
                .unwrap_or_default();
            all.iter()
                .filter(|i| {
                    i.root == inst.root
                        || (active(i) && slugs.iter().any(|s| s == vitrify_core::graph::slug_of(i)))
                })
                .collect()
        }
        None => all.iter().filter(|i| active(i)).collect(),
    };
    if chosen.is_empty() {
        failures.push(with_fix(
            "no SDD of the repository is Active",
            "finish authoring one and set it Active: vitrify brief authoring",
        ));
        return Ok(Err(2));
    }
    chosen.sort_by(|a, b| vitrify_core::graph::slug_of(a).cmp(vitrify_core::graph::slug_of(b)));
    ctx.project = project_wide || chosen.len() > 1;
    Ok(Ok(chosen
        .into_iter()
        .map(|i| {
            let rel = cmd::sdd_rel(&root, &i.root).unwrap_or_else(|| i.root.clone());
            (rel, i.clone())
        })
        .collect()))
}

/// The SDD the run builds — `--sdd`, else the only one — pinned as its path
/// under the repository root and loaded from the primary checkout. `None`
/// (with the failure recorded) when there is no such SDD.
fn resolve_lane(
    ctx: &Ctx,
    failures: &mut Vec<String>,
) -> Result<Option<(PathBuf, Instance)>, CoreError> {
    let root = ctx.primary.dir.clone();
    // SDD located. `--sdd` is a path from where the verb runs, which may be
    // a linked worktree: what it names there is found by its path under that
    // checkout's root. Without it, the primary checkout is searched.
    let located = match ctx.sdd_arg.as_deref() {
        Some(arg) => {
            let here = cmd::repo_root();
            cmd::resolve_sdd(&here, Some(arg), &cmd::cwd()).map(|h| {
                cmd::sdd_rel(&here, &h.root)
                    .or_else(|| cmd::sdd_rel(&root, &h.root))
                    .ok_or(h.root)
            })
        }
        None => cmd::resolve_sdd(&root, None, &cmd::cwd())
            .map(|h| cmd::sdd_rel(&root, &h.root).ok_or(h.root)),
    };
    // Pin it: from here on the SDD is this path under the repository root,
    // in whichever checkout is being read — never searched for again.
    let sdd_rel = match located {
        Ok(Ok(rel)) => rel,
        Ok(Err(outside)) => {
            failures.push(with_fix(
                &format!(
                    "SDD {} is outside the repository {}",
                    outside.display(),
                    root.display()
                ),
                "pass an --sdd inside this repository",
            ));
            return Ok(None);
        }
        Err(e) => {
            failures.push(sdd_failure(&e.to_string()));
            return Ok(None);
        }
    };
    // preflight judges the primary checkout's copy
    match cmd::resolve_sdd_in(&root, &sdd_rel).and_then(Instance::load) {
        Ok(inst) => Ok(Some((sdd_rel, inst))),
        Err(e) => {
            failures.push(sdd_failure(&e.to_string()));
            Ok(None)
        }
    }
}

/// The project's preflight, before anything runs: the local mainline not
/// ahead of origin, each lane's slug its SDD's alone, a harness recorded,
/// the worktree root usable. Returns whether the worktree root is usable.
fn project_preflight(ctx: &Ctx, lanes: &[LaneSeed], failures: &mut Vec<String>) -> bool {
    // origin arbitrates: the run builds origin's head, not the local
    // branch's. Commits the local branch holds and origin does not would be
    // read here (the SDD, its version, its staleness marks) and then left
    // out of everything the run builds and reports on.
    if ctx.push {
        let _ = ctx.primary.fetch_origin(&ctx.branch);
        let local = ctx.primary.rev_parse(&format!("refs/heads/{}", ctx.branch));
        let remote = ctx
            .primary
            .rev_parse(&format!("refs/remotes/origin/{}", ctx.branch));
        if let (Ok(local), Ok(remote)) = (local, remote)
            && local != remote
            && !ctx.primary.is_ancestor(&local, &remote).unwrap_or(false)
        {
            failures.push(with_fix(
                &format!(
                    "the local {b} holds commits origin/{b} does not — the run builds origin's head, so they would be left out",
                    b = ctx.branch
                ),
                &format!("publish them first: git push origin {}", shell_word(&ctx.branch)),
            ));
        }
    }
    // branch, worktree and checkout names carry the slug: it must be its
    // SDD's alone
    let others = cmd::discover(&ctx.primary.dir);
    for lane in lanes {
        for other in &others {
            if other.root != lane.inst.root
                && layout::slug_of(&other.meta, &other.root, other.shape) == lane.ml.slug
            {
                failures.push(with_fix(
                    &format!(
                        "SDD slug `{}` is also used by {} — branch and worktree names would collide",
                        lane.ml.slug,
                        other.root.display()
                    ),
                    "rename one of the `sdd:` slugs",
                ));
            }
        }
    }

    // ≥ 1 working harness probe
    if ctx.argv.is_empty() {
        failures.push(with_fix(
            "~/.vitrify/config.toml records no working harness",
            "vitrify probe",
        ));
    }
    if let Some(e) = &ctx.profile_error {
        failures.push(with_fix(
            e,
            "correct the line the error names, then vitrify probe",
        ));
    }

    // the worktrees must be somewhere no ignore file of the repository covers
    match ctx.layout.ensure_worktree_root(&ctx.primary) {
        Ok(()) => true,
        Err(e) => {
            failures.push(with_fix(
                &e.to_string(),
                "point run.worktree_dir at a directory no ignore file of the repository covers",
            ));
            false
        }
    }
}

/// A lane's static preflight, at the start of the run: its SDD Active and
/// committed at the mainline head, lint clean, the audit clean (or only
/// lapsed leases, which are fixed), the journal consistent with front
/// matter (the audit replays it). The audit runs no Verification row: the
/// dynamic preflight re-checks what a change reached. `false` =
/// interrupted.
fn lane_static(
    ctx: &Ctx,
    lane: &LaneSeed,
    failures: &mut Vec<String>,
    dry_run: bool,
) -> Result<bool, CoreError> {
    let root = &ctx.primary.dir;
    let inst = &lane.inst;
    let sdd_word = shell_word(&cmd::sdd_arg(&lane.ml.sdd_rel));
    if inst.meta.get_str_or("status", "Draft") != "Active" {
        failures.push(with_fix(
            &format!(
                "SDD status is {}, not Active",
                inst.meta.get_str_or("status", "Draft")
            ),
            "finish authoring and set it Active: vitrify brief authoring",
        ));
    }
    // committed at the mainline head: spec paths clean in the primary
    let porcelain = ctx.primary.status_porcelain().map_err(CoreError::from)?;
    let spec_rel = cmd::sdd_arg(&lane.ml.sdd_rel);
    let under_spec =
        |p: &str| spec_rel == "." || p == spec_rel || p.starts_with(&format!("{spec_rel}/"));
    let dirty_spec: Vec<&str> = porcelain
        .lines()
        .filter(|l| l.split_whitespace().nth(1).is_some_and(under_spec))
        .collect();
    if !dirty_spec.is_empty() {
        if crate::cmd::claim::behind_note(&ctx.primary, &ctx.branch).is_some() {
            // not the user's edits: the checkout holds an earlier commit
            failures.push(with_fix(
                &format!(
                    "the primary checkout is behind {} — its copy of the SDD is not the one at the mainline head",
                    ctx.branch
                ),
                "bring it up to date as the note above says",
            ));
        } else {
            failures.push(with_fix(
                &format!(
                    "SDD not committed at the mainline head (uncommitted spec changes): {}",
                    dirty_spec.join("; ")
                ),
                &format!("commit or stash the changes under {sdd_word}"),
            ));
        }
    }

    // lint (the resume-tolerant subset; built state is journal-governed once
    // the SDD is Active)
    let lint_findings = cmd::lint::pre_run_findings(inst, root, ctx.cfg.lint.readset_budget_bytes)?;
    for x in &lint_findings {
        failures.push(with_fix(
            &format!("lint [{}] {}: {}", x.check, x.subject, x.cause),
            &x.remediation,
        ));
    }

    // audit clean — or only lapsed leases, fixed and published
    let (findings, _) = audit::run(
        inst,
        root,
        &cmd::audit_options(
            audit::Options {
                skip_rotten: true,
                ..Default::default()
            },
            root,
            inst,
        ),
        &ShRunner,
        UtcTime::now(),
    )?;
    if signals::interrupted() {
        return Ok(false);
    }
    let audit_clean = findings.is_empty();
    if !audit_clean {
        if findings.iter().all(|f| f.check == 7) {
            if dry_run {
                // a dry run publishes nothing
                diag(&format!(
                    "preflight: {} lapsed lease(s) — a run releases them before it starts",
                    findings.len()
                ));
            } else {
                lane.ml.heal(true, false)?;
                diag(&format!(
                    "preflight: {} lapsed lease(s) fixed and published",
                    findings.len()
                ));
            }
        } else {
            for f in &findings {
                failures.push(with_fix(
                    &format!("audit [check {}] {}: {}", f.check, f.subject, f.cause),
                    &f.remediation,
                ));
            }
        }
    }
    Ok(true)
}

/// A lane's dynamic preflight, before its first build: its integration
/// checkout is made anew at the mainline head — whatever an earlier run
/// left there is gone — and the §5 Bootstrap, Build and Lint commands run
/// clean in it, never in the primary checkout. The Test command does not
/// gate the start: whether tests exist or pass before the build is the
/// SDD's test posture (schema §10). From the first integration on it gates
/// as a regression check. Then the lane's re-check ([`lane_recheck`]).
/// `false` = interrupted.
fn lane_dynamic(
    ctx: &Ctx,
    lane: &mut LaneSeed,
    failures: &mut Vec<String>,
    dry_run: bool,
    recheck: bool,
) -> Result<bool, CoreError> {
    let note = || {
        diag(
            "note: another vitrify process is integrating this SDD — waiting for its integration checkout",
        )
    };
    let Some((checkout, head)) = dynamic_checks(&lane.ml, failures, &note)? else {
        return Ok(false);
    };
    if !failures.is_empty() || !recheck {
        return Ok(true);
    }
    lane_recheck(ctx, lane, &checkout, &head, dry_run)
}

/// A lane's §5 Bootstrap, Build and Lint, in its integration checkout made
/// anew at the mainline head (under its lock: another run of the SDD may be
/// gating there — `waiting` is called once if so). Each failure is added to
/// `failures`. Returns the checkout and the head; `None` = interrupted.
pub(crate) fn dynamic_checks(
    ml: &Mainline,
    failures: &mut Vec<String>,
    waiting: &dyn Fn(),
) -> Result<Option<(PathBuf, String)>, CoreError> {
    let Some(_held) = ml.lock_checkout(waiting)? else {
        return Ok(None);
    };
    let head = ml.head()?;
    let checkout = ml.recreate_checkout(&head)?;
    let inst = ml.sdd_in(&checkout)?;
    let cmds = integrate::sdd_commands(&model::read_text(&inst.file)?);
    let steps: [(&str, &Option<String>); 3] = [
        ("bootstrap", &cmds.bootstrap),
        ("build", &cmds.build),
        ("lint", &cmds.lint),
    ];
    for (name, cmd_opt) in steps {
        let Some(c) = cmd_opt else {
            failures.push(with_fix(
                &format!("§5 Commands block missing the {name} command"),
                &format!("add the {name} command to the SDD's §5 Commands"),
            ));
            continue;
        };
        let (ok, output) = integrate::run_gate_command(c, &checkout);
        if signals::interrupted() {
            return Ok(None);
        }
        if !ok {
            let quoted = tail(&output, 5);
            let cause = format!("§5 {name} command failed: {c}");
            let fix = "run it from the repository root and fix what it reports";
            failures.push(if quoted.is_empty() {
                with_fix(&cause, fix)
            } else {
                format!("{}\n{quoted}", with_fix(&cause, fix))
            });
        }
    }
    Ok(Some((checkout, head)))
}

/// The lane's re-check, in its checkout at `head`: the Verified units a
/// change reached since their last recorded check (with `--recheck`, every
/// one) run their rows, then the §5 Test command. Those that pass are
/// recorded — fresh stamps and `verified` events by `recheck`, and
/// `suite: "pass"` when the Test command passed — as one mainline commit
/// (a dry run records nothing). A unit whose rows fail is rotten from the
/// start. With nothing dirty, nothing runs. `false` = interrupted.
fn lane_recheck(
    ctx: &Ctx,
    lane: &mut LaneSeed,
    checkout: &Path,
    head: &str,
    dry_run: bool,
) -> Result<bool, CoreError> {
    let git = ctx.primary.at(checkout);
    let slug = vitrify_core::graph::slug_of(&lane.inst).to_string();
    let mode = if ctx.recheck {
        Mode::Everything
    } else {
        Mode::Changes
    };
    let units: Vec<String> =
        match Snapshot::load(&git, checkout, head).and_then(|snap| snap.dirty(&git, mode)) {
            Ok(Cascade::Resolved(set)) => set.of_sdd(&slug).map(|d| d.node.unit.clone()).collect(),
            // without a whole repository to cascade along, every Verified
            // unit is checked
            _ => lane
                .inst
                .units
                .iter()
                .filter(|u| u.meta.get_str_or("status", "Draft") == "Verified")
                .map(|u| u.unit.clone())
                .collect(),
        };
    if units.is_empty() {
        return Ok(true);
    }
    let report = recheck::run(
        &git,
        head,
        &lane.ml.sdd_rel,
        &units,
        &integrate::PoolRunner(ctx.cfg.run.rows_at_once()),
    )?;
    if report.interrupted || signals::interrupted() {
        return Ok(false);
    }
    if let Some(broken) = &report.broken {
        diag(&format!(
            "note: the re-check could not run: `{}` failed at {}",
            broken.command,
            &head[..head.len().min(7)]
        ));
        return Ok(true);
    }
    // what failed is rotten: something on mainline broke it, or it is flaky
    lane.rotten
        .extend(integrate::rotten_of(&lane.inst, &report)?);
    lane.checked = report.units.iter().map(|u| u.unit.clone()).collect();
    lane.suite_ran = report.suite.is_some();
    let edits = report.edits();
    if !edits.verified.is_empty() {
        diag(&format!(
            "preflight: re-checked {} at {}{}",
            edits.verified.join(", "),
            edits.commit,
            if dry_run {
                " (a dry run records nothing)"
            } else {
                ""
            }
        ));
    }
    if !dry_run {
        integrate::record_recheck(&lane.ml, &edits);
    }
    Ok(true)
}

/// The last `n` lines of `text` that say something (blank lines skipped) —
/// what a report quotes of a command's output.
pub(crate) fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// The job header of one lane. `watch`: the run is starting, so the header
/// ends with the command that watches it (a dry run starts nothing to
/// watch).
fn print_header(ctx: &Ctx, lane: &LaneSeed, watch: bool) {
    let inst = &lane.inst;
    let slug = inst.meta.get_str_or("sdd", "?");
    let version = inst.meta.get_str_or("version", "?");
    emit(&format!(
        "SDD build job — {slug} @ {version}  [{}]",
        ctx.job_id
    ));
    let has_test = model::read_text(&inst.file)
        .map(|text| integrate::sdd_commands(&text).test.is_some())
        .unwrap_or(false);
    emit(&format!(
        "Done-condition: every non-Retired unit Verified @ {version}; audit clean; replay clean{}",
        if has_test { "; §5 Test passing" } else { "" }
    ));
    emit(&format!(
        "Mainline: {}   Remote: {}   Shape: {}",
        ctx.branch,
        if ctx.push { "origin" } else { "none" },
        inst.shape.as_str()
    ));
    emit(&format!("Plan: {}", plan(inst)));
    emit(&format!(
        "Workers: cap={}   Holder IDs: {}-w<i>   Harness: {}",
        ctx.cap,
        ctx.job_id,
        if ctx.harness.is_empty() {
            "-"
        } else {
            &ctx.harness
        }
    ));
    if !ctx.roles.is_empty() && !ctx.harness.is_empty() {
        emit(&format!("Roles: {}", ctx.roles.join("   ")));
    }
    emit(&format!(
        "Worktrees: {}   Branches: sdd/{}/<unit>/c<seq>",
        ctx.layout.wt_root.display(),
        lane.ml.slug
    ));
    emit(&format!(
        "Parameters: worker cap {} · gate jobs {}{} · lease {}m · renew at {} remaining · escalate after {} failed attempts of a unit, set it aside after {}",
        ctx.cap,
        ctx.gate_pool.size(),
        if ctx.cfg.run.rows_follow_cores() {
            " (one per core)"
        } else {
            ""
        },
        ctx.cfg.run.lease_minutes,
        ctx.cfg.run.renew_fraction,
        ctx.cfg.run.abort_failed_waves.max(1),
        ctx.cfg.run.abort_failed_waves.max(1) * 2
    ));
    if watch {
        emit(&format!(
            "Watch: vitrify run status --job {} [--follow] · once it ends, the same command prints the report · procedure: vitrify brief run",
            ctx.job_id
        ));
    }
}

/// The authored plan: units in order, parallel groups named, with current
/// state. Dispatch follows `depends_on` alone — every claimable unit, up to
/// the cap — so this line is orientation, not a schedule.
fn plan(inst: &Instance) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut units: Vec<&model::Unit> = inst.units.iter().collect();
    units.sort_by(|a, b| a.unit.cmp(&b.unit));
    let mut seen_groups: Vec<String> = Vec::new();
    for u in &units {
        let status = u.meta.get_str_or("status", "Draft");
        let group = u.meta.get_str_or("parallel_group", "").trim().to_string();
        if model::is_empty_marker(&group) {
            parts.push(format!("{} ({status})", u.unit));
        } else if !seen_groups.contains(&group) {
            seen_groups.push(group.clone());
            let members: Vec<String> = units
                .iter()
                .filter(|x| x.meta.get_str_or("parallel_group", "").trim() == group)
                .map(|x| x.unit.clone())
                .collect();
            parts.push(format!("[P:{group}] {}", members.join(", ")));
        }
    }
    parts.join(" → ")
}

// ------------------------------------------------------------- run record

/// One dispatched worker, as `run.json` shows it.
pub(crate) struct WorkerRec {
    /// The slug of the lane it works for: its logs are under
    /// `.vitrify/logs/<job>/<lane>/`.
    pub lane: String,
    pub holder: String,
    pub pid: u32,
    pub unit: String,
    pub claim_seq: i64,
    pub branch: String,
    pub worktree: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// `running`, then the attempt's outcome.
    pub outcome: String,
    /// The attempt started from an earlier one's work, so a reader can tell
    /// a fresh start from a second try.
    pub retry: bool,
    /// Why the attempt ended, first line only; `None` while it runs and
    /// when it verified, so the record says why without opening the logs.
    pub detail: Option<String>,
}

/// The run's record: what every lane did, in the order it happened.
#[derive(Default)]
pub(crate) struct RunState {
    pub workers: Vec<WorkerRec>,
    pub reports: Vec<String>,
    pub escalations: Vec<(String, String)>, // (evidence_path, verdict)
    /// The final report's lines, from `Complete.` or `Blocked.` on; `None`
    /// until it is printed.
    pub report: Option<Vec<String>>,
}

/// Write `body` to `dir/name` whole (tmp + rename), so a reader never sees
/// half a record.
fn write_record(dir: &Path, name: &str, body: &str) -> Result<(), CoreError> {
    std::fs::create_dir_all(dir).map_err(|source| CoreError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, body).map_err(|source| CoreError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| CoreError::Io { path, source })
}

/// Write the run record: `.vitrify/run.json` (the latest run in this
/// repository) and a copy beside the job's logs. Written when the run
/// starts and at every change, so a run that dies leaves its own record —
/// including the workers it had running. `sdd` names what the run builds.
pub(crate) fn write_run_json(
    ctx: &Ctx,
    sdd: &str,
    state: &RunState,
    started_at: &str,
    terminated: Option<(&str, &str)>,
) -> Result<(), CoreError> {
    let workers: Vec<J> = state
        .workers
        .iter()
        .map(|w| {
            J::Obj(vec![
                ("holder".into(), J::s(w.holder.clone())),
                ("pid".into(), J::Int(w.pid as i64)),
                ("unit".into(), J::s(w.unit.clone())),
                ("claim_seq".into(), J::Int(w.claim_seq)),
                ("branch".into(), J::s(w.branch.clone())),
                ("worktree".into(), J::s(w.worktree.clone())),
                ("started_at".into(), J::s(w.started_at.clone())),
                ("ended_at".into(), J::opt_str(w.ended_at.clone())),
                ("outcome".into(), J::s(w.outcome.clone())),
                ("retry".into(), J::Bool(w.retry)),
                ("detail".into(), J::opt_str(w.detail.clone())),
                ("lane".into(), J::s(w.lane.clone())),
            ])
        })
        .collect();
    let escalations: Vec<J> = state
        .escalations
        .iter()
        .map(|(p, v)| {
            J::Obj(vec![
                ("evidence_path".into(), J::s(p.clone())),
                ("verdict".into(), J::s(v.clone())),
            ])
        })
        .collect();
    let mut pairs = vec![
        ("job_id".into(), J::s(ctx.job_id.clone())),
        ("pid".into(), J::Int(std::process::id() as i64)),
        ("sdd".into(), J::s(sdd)),
        ("harness".into(), J::s(ctx.harness.clone())),
        ("started_at".into(), J::s(started_at)),
        (
            "done_condition".into(),
            J::s(
                "every non-Retired unit Verified @ current SDD version; audit clean; replay clean; §5 Test passing (when the SDD has one)",
            ),
        ),
        (
            "params".into(),
            J::Obj(vec![
                ("cap".into(), J::Int(ctx.cap as i64)),
                ("gate_jobs".into(), J::Int(ctx.gate_pool.size() as i64)),
                ("lease_minutes".into(), J::Int(ctx.cfg.run.lease_minutes)),
                (
                    "worker_timeout_minutes".into(),
                    J::Int(ctx.cfg.run.worker_timeout_minutes),
                ),
                (
                    "abort_failed_waves".into(),
                    J::Int(ctx.cfg.run.abort_failed_waves),
                ),
            ]),
        ),
        (
            "worktrees".into(),
            J::s(ctx.layout.wt_root.display().to_string()),
        ),
        ("workers".into(), J::Arr(workers)),
        (
            "reports".into(),
            J::Arr(state.reports.iter().map(|r| J::s(r.clone())).collect()),
        ),
        ("escalations".into(), J::Arr(escalations)),
    ];
    if let Some((end_state, detail)) = terminated {
        pairs.push((
            "terminated".into(),
            J::Obj(vec![
                ("state".into(), J::s(end_state)),
                ("at".into(), J::s(UtcTime::now().format())),
                ("detail".into(), J::s(detail)),
            ]),
        ));
    }
    pairs.push((
        "report".into(),
        state.report.as_ref().map_or(J::Null, |lines| {
            J::Arr(lines.iter().map(|l| J::s(l.clone())).collect())
        }),
    ));
    let body = J::Obj(pairs).dumps_indent2();
    ctx.layout.ensure_scratch()?;
    for dir in [&ctx.layout.scratch, &ctx.logs_dir] {
        write_record(dir, "run.json", &body)?;
    }
    Ok(())
}
