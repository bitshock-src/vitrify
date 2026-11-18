//! `vitrify run` — the deterministic orchestrator: preflight, continuous
//! dispatch, batched serial integration, termination reports. Control flow
//! is Rust; inference happens only at the three enumerated points: workers,
//! escalation, and plan-time authoring (outside `run`).
//! Resume = re-derivation from repo state, never memory.

pub mod escalate;
pub mod events;
pub mod integrate;
pub mod lease;
pub mod mainline;
pub mod progress;
pub mod retry;
mod sched;
pub mod worker;

use std::path::{Path, PathBuf};

use vitrify_core::config::{Config, Overrides};
use vitrify_core::harness::Role;
use vitrify_core::model::{self, Instance, Shape, UtcTime};
use vitrify_core::{CoreError, audit};

use crate::cmd;
use crate::gate::MemoRunner;
use crate::git::{Git, resolve_mainline, use_push};
use crate::home;
use crate::layout::{self, Layout};
use crate::out::{J, diag, emit};
use crate::signals;
use mainline::Mainline;

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
}

pub(crate) struct Ctx {
    /// Repository, SDD (pinned in preflight), mainline branch, publish mode.
    pub ml: Mainline,
    pub cfg: Config,
    pub cap: usize,
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
    pub logs_dir: PathBuf,
    pub wait: bool,
    /// The SDD's name in branch and worktree names.
    pub slug: String,
    /// The mainline commit preflight's full audit ran clean on, when the
    /// primary checkout held exactly that commit.
    pub audited_at: Option<String>,
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
    let logs_dir = layout.scratch.join("logs").join(&job_id);

    let mut ctx = Ctx {
        ml: Mainline {
            primary,
            layout,
            sdd_rel: PathBuf::new(),
            branch: mainline,
            push,
        },
        cap: flags.cap.unwrap_or(cfg.run.worker_cap).max(1) as usize,
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
        slug: String::new(),
        audited_at: None,
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
        crate::cmd::claim::behind_note(&ctx.ml.primary, &ctx.ml.branch)
    } else {
        let _turn = crate::cmd::claim::tx_lock();
        crate::cmd::claim::catch_up_primary(&ctx.ml.primary, &ctx.ml.branch)
    };
    if let Some(note) = behind {
        diag(&note);
    }

    // ---------------- preflight: refuse to start on any failure
    let mut failures: Vec<String> = Vec::new();
    let inst = preflight(&mut ctx, &mut failures, flags.dry_run)?;
    if let Some(sig) = signals::pending() {
        diag(&format!(
            "interrupted ({}) during preflight",
            signals::name(sig)
        ));
        signals::die_by(sig);
    }
    let inst = match inst {
        Some(inst) => inst,
        None => {
            for fail in &failures {
                diag(&format!("preflight failed: {fail}"));
            }
            return Ok(2);
        }
    };

    // ---------------- job header BEFORE dispatch
    print_header(&ctx, &inst, !flags.dry_run);
    if flags.dry_run {
        for line in sched::dead_run_notes(&ctx, &inst) {
            emit(&line);
        }
        emit("dry-run: preflight green; stopping before dispatch");
        emit(&format!("Start: {}", start_command(&flags)));
        return Ok(0);
    }

    sched::orchestrate(ctx)
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
    if let Some(sdd) = &flags.sdd {
        words.extend(["--sdd".to_string(), shell_word(sdd)]);
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

fn preflight(
    ctx: &mut Ctx,
    failures: &mut Vec<String>,
    dry_run: bool,
) -> Result<Option<Instance>, CoreError> {
    let root = ctx.ml.primary.dir.clone();
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
    let inst = match cmd::resolve_sdd_in(&root, &sdd_rel).and_then(Instance::load) {
        Ok(inst) => inst,
        Err(e) => {
            failures.push(sdd_failure(&e.to_string()));
            return Ok(None);
        }
    };
    ctx.ml.sdd_rel = sdd_rel;
    ctx.slug = layout::sdd_slug(&inst);

    let sdd_word = shell_word(&cmd::sdd_arg(&ctx.ml.sdd_rel));
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
    let porcelain = ctx.ml.primary.status_porcelain().map_err(CoreError::from)?;
    let spec_rel = cmd::sdd_arg(&ctx.ml.sdd_rel);
    let under_spec =
        |p: &str| spec_rel == "." || p == spec_rel || p.starts_with(&format!("{spec_rel}/"));
    let dirty_spec: Vec<&str> = porcelain
        .lines()
        .filter(|l| l.split_whitespace().nth(1).is_some_and(under_spec))
        .collect();
    if !dirty_spec.is_empty() {
        if crate::cmd::claim::behind_note(&ctx.ml.primary, &ctx.ml.branch).is_some() {
            // not the user's edits: the checkout holds an earlier commit
            failures.push(with_fix(
                &format!(
                    "the primary checkout is behind {} — its copy of the SDD is not the one at the mainline head",
                    ctx.ml.branch
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
    // origin arbitrates: the run builds origin's head, not the local
    // branch's. Commits the local branch holds and origin does not would be
    // read here (the SDD, its version, its staleness marks) and then left
    // out of everything the run builds and reports on.
    if ctx.ml.push {
        let _ = ctx.ml.primary.fetch_origin(&ctx.ml.branch);
        let local = ctx
            .ml
            .primary
            .rev_parse(&format!("refs/heads/{}", ctx.ml.branch));
        let remote = ctx
            .ml
            .primary
            .rev_parse(&format!("refs/remotes/origin/{}", ctx.ml.branch));
        if let (Ok(local), Ok(remote)) = (local, remote)
            && local != remote
            && !ctx.ml.primary.is_ancestor(&local, &remote).unwrap_or(false)
        {
            failures.push(with_fix(
                &format!(
                    "the local {b} holds commits origin/{b} does not — the run builds origin's head, so they would be left out",
                    b = ctx.ml.branch
                ),
                &format!("publish them first: git push origin {}", shell_word(&ctx.ml.branch)),
            ));
        }
    }
    // branch and worktree names carry the slug: it must be this SDD's alone
    for other in cmd::discover(&root) {
        if other.root != inst.root
            && layout::slug_of(&other.meta, &other.root, other.shape) == ctx.slug
        {
            failures.push(with_fix(
                &format!(
                    "SDD slug `{}` is also used by {} — branch and worktree names would collide",
                    ctx.slug,
                    other.root.display()
                ),
                "rename one of the `sdd:` slugs",
            ));
        }
    }

    // lint (the resume-tolerant subset; built state is journal-governed once
    // the SDD is Active)
    let lint_findings =
        cmd::lint::pre_run_findings(&inst, &root, ctx.cfg.lint.readset_budget_bytes)?;
    for x in &lint_findings {
        failures.push(with_fix(
            &format!("lint [{}] {}: {}", x.check, x.subject, x.cause),
            &x.remediation,
        ));
    }

    // audit clean — or only lapsed leases, fixed and published. Check 4
    // re-runs every Verified unit's rows: each distinct command once,
    // run.gate_jobs at a time.
    let jobs = ctx.cfg.run.rows_at_once();
    let runner = MemoRunner::for_verified(&inst, &root, jobs)?;
    let (findings, _) = audit::run(
        &inst,
        &root,
        &cmd::audit_options(audit::Options::default(), &root, &inst),
        &runner,
        UtcTime::now(),
    )?;
    if signals::interrupted() {
        return Ok(None);
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
                ctx.ml.heal(true, false)?;
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

    // §5 bootstrap, build and lint run clean at the head. The Test command
    // does not gate the start: whether tests exist or pass before the build
    // is the SDD's test posture (schema §10). From the first integration on
    // it gates as a regression check.
    let core_text = model::read_text(&inst.file)?;
    let cmds = integrate::sdd_commands(&core_text);
    for (name, cmd_opt) in [
        ("bootstrap", &cmds.bootstrap),
        ("build", &cmds.build),
        ("lint", &cmds.lint),
    ] {
        match cmd_opt {
            Some(c) => {
                let (ok, output) = integrate::run_gate_command(c, &root);
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
            None => failures.push(with_fix(
                &format!("§5 Commands block missing the {name} command"),
                &format!("add the {name} command to the SDD's §5 Commands"),
            )),
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
    if let Err(e) = ctx.ml.layout.ensure_worktree_root(&ctx.ml.primary) {
        failures.push(with_fix(
            &e.to_string(),
            "point run.worktree_dir at a directory no ignore file of the repository covers",
        ));
    }

    // single-file shape forces cap 1
    if inst.shape == Shape::Single && ctx.cap > 1 {
        diag("note: single-file SDD — worker cap forced to 1 (every status edit shares one file)");
        ctx.cap = 1;
    }

    // What the audit just established holds for the mainline head only if
    // the primary checkout is that commit, unmodified.
    if audit_clean
        && failures.is_empty()
        && let Ok(head) = ctx.ml.head()
        && ctx.ml.primary.rev_parse("HEAD").ok().as_deref() == Some(head.as_str())
        && ctx.ml.primary.clean_against(&head).unwrap_or(false)
    {
        ctx.audited_at = Some(head);
    }

    if failures.is_empty() {
        Ok(Some(inst))
    } else {
        Ok(None)
    }
}

/// The last `n` lines of `text` that say something (blank lines skipped) —
/// what a report quotes of a command's output.
pub(crate) fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// The job header. `watch`: the run is starting, so the header ends with
/// the command that watches it (a dry run starts nothing to watch).
fn print_header(ctx: &Ctx, inst: &Instance, watch: bool) {
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
        ctx.ml.branch,
        if ctx.ml.push { "origin" } else { "none" },
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
        ctx.ml.layout.wt_root.display(),
        ctx.slug
    ));
    emit(&format!(
        "Parameters: worker cap {} · gate jobs {}{} · lease {}m · renew at {} remaining · escalate after {} failed attempts of a unit, set it aside after {}",
        ctx.cap,
        ctx.cfg.run.rows_at_once(),
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

#[derive(Default)]
pub(crate) struct RunState {
    pub workers: Vec<WorkerRec>,
    pub reports: Vec<String>,
    pub escalations: Vec<(String, String)>, // (evidence_path, verdict)
    pub gate_failures: Vec<(String, String, String)>, // (unit, command, output)
    pub attempts: Vec<(String, String, String)>, // (unit, final outcome label, detail)
    /// The final report's lines, from `Complete.` or `Blocked.` on; `None`
    /// until it is printed.
    pub report: Option<Vec<String>>,
}

/// Write the run record: `.vitrify/run.json` (the latest run in this
/// repository) and a copy beside the job's logs. Written when the run
/// starts and at every change, so a run that dies leaves its own record —
/// including the workers it had running.
pub(crate) fn write_run_json(
    ctx: &Ctx,
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
        ("sdd".into(), J::s(cmd::sdd_arg(&ctx.ml.sdd_rel))),
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
                (
                    "gate_jobs".into(),
                    J::Int(ctx.cfg.run.rows_at_once() as i64),
                ),
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
            J::s(ctx.ml.layout.wt_root.display().to_string()),
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
    ctx.ml.layout.ensure_scratch()?;
    for dir in [ctx.ml.layout.scratch.clone(), ctx.logs_dir.clone()] {
        std::fs::create_dir_all(&dir).map_err(|source| CoreError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = dir.join("run.json");
        let tmp = dir.join("run.json.tmp");
        std::fs::write(&tmp, &body).map_err(|source| CoreError::Io {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, &path).map_err(|source| CoreError::Io { path, source })?;
    }
    Ok(())
}
