//! The orchestrator's loop.
//!
//! One scheduler thread owns every decision. Around it: a monitor thread
//! per running worker (lease renewal, the hard budget, the exit), one
//! integrator thread (the integration checkout and its gate), and one
//! lease keeper (work waiting for the gate). They report through a single
//! channel; nothing but the scheduler changes the run's state.
//!
//! Dispatch is continuous: whenever a slot is free, every claimable unit is
//! a candidate — any two are independent by construction (neither depends on
//! the other, each has its dependencies built, and ownership is exclusive) —
//! and each built unit goes to the integrator as it reports, to be gated
//! with whatever else is ready. State is re-derived from the mainline head,
//! never remembered: what a unit's last attempt left is in git, what a
//! change to the spec staled is in the traces.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vitrify_core::claims::Op;
use vitrify_core::model::{self, Instance, UnitView, UtcTime};
use vitrify_core::trace::{self, TraceMap};
use vitrify_core::writeset::WriteSet;
use vitrify_core::{CoreError, journal, ledger};

use super::events::{EventLog, RunEvent};
use super::integrate::{self, GateMemo, HeadCheck, IntegrateRequest, Integrator, Outcome};
use super::lease::{self, Keeper, MonitorParams, MonitorVerdict};
use super::mainline::Mainline;
use super::progress::GateBoard;
use super::{Ctx, RunState, VERSION, WorkerRec, escalate, retry, tail, worker, write_run_json};
use crate::cmd::{brief, claim};
use crate::gate;
use crate::layout;
use crate::out::{self, J, diag, emit};
use crate::signals;

/// How long the scheduler sleeps when nothing reports.
const TICK: Duration = Duration::from_secs(1);

enum Event {
    /// A worker's process ended (or was ended).
    Exited {
        unit: String,
        verdict: MonitorVerdict,
        expires: UtcTime,
    },
    Integrated {
        unit: String,
        outcome: Outcome,
    },
    /// The integrator hit an error that is not a verdict on any unit.
    IntegratorFailed(String),
    HeadChecked(Box<Result<HeadCheck, CoreError>>),
}

enum Job {
    Integrate(IntegrateRequest),
    CheckHead { want_suite: bool },
    Stop,
}

#[derive(PartialEq)]
enum Phase {
    /// The worker is running.
    Working,
    /// Reported `built`; waiting for, or in, the gate.
    Integrating,
}

/// One claim this run holds.
struct Active {
    holder: String,
    seq: i64,
    branch: String,
    wt: PathBuf,
    pid: u32,
    out_path: PathBuf,
    /// The worker's temp directory: this claim's alone until it is closed.
    _tmp: gate::TmpSlot,
    /// What the unit's inputs hashed to when it was claimed.
    trace: TraceMap,
    phase: Phase,
    /// Index of this attempt in `RunState::workers`.
    rec: usize,
    /// Why the scheduler ended the worker itself, if it did.
    killed: Option<&'static str>,
}

/// A unit the run has stopped dispatching: an open question, or too many
/// failed attempts. It returns to the pool when its reading set changes —
/// the only thing that can make another attempt go differently.
struct Parked {
    reason: String,
    trace: TraceMap,
}

#[derive(Default)]
struct Fails {
    all: u32,
    work: u32,
    escalated: bool,
}

enum End {
    Complete,
    Blocked,
    /// The escalation judge said the job cannot safely continue.
    Aborted(String),
    Interrupted(i32),
}

/// The lease backend: claim transactions on this run's SDD.
struct RealLease {
    ml: Mainline,
    lease_minutes: i64,
}

impl lease::LeaseBackend for RealLease {
    fn renew(&self, unit: &str, holder: &str) -> lease::RenewResult {
        match self.ml.claim(Op::Renew, unit, holder, self.lease_minutes) {
            Ok(outcome) => match outcome.claim_expires.as_deref().and_then(UtcTime::parse) {
                Some(expires) => lease::RenewResult::Renewed { expires },
                None => lease::RenewResult::Failed("unparseable expiry".into()),
            },
            Err(e) if e.exit_code() == 6 => lease::RenewResult::LeaseExpired,
            // exit 4: no longer Claimed by this holder — the lease is gone
            Err(e) if e.exit_code() == 4 => lease::RenewResult::LeaseExpired,
            Err(e) => lease::RenewResult::Failed(e.to_string()),
        }
    }
}

struct Scheduler {
    ctx: Ctx,
    state: RunState,
    started_at: String,
    events_tx: Sender<Event>,
    events: Receiver<Event>,
    jobs: Sender<Job>,
    integrator: Option<JoinHandle<()>>,
    keeper: Arc<Keeper>,
    keeper_thread: Option<JoinHandle<()>>,
    params: MonitorParams,
    /// Held for the life of the run: a later run reads a free lock as
    /// "that run is gone".
    job_lock: Option<std::fs::File>,
    /// A detached checkout of the mainline head, for reading spec state.
    head_wt: PathBuf,
    active: BTreeMap<String, Active>,
    parked: BTreeMap<String, Parked>,
    /// Attempts per unit that did not land, this run: all of them, those
    /// where the work itself failed, and whether the judge has been asked.
    fails: BTreeMap<String, Fails>,
    worker_n: u32,
    escalation_index: u32,
    inst: Option<Instance>,
    views: Vec<UnitView>,
    head: String,
    /// The head up to which staleness has been applied.
    healed_at: String,
    last_derive: Instant,
    dirty: bool,
    abort: Option<String>,
    failure: Option<String>,
    /// Claims lost to a publish race since the last one that succeeded.
    races_lost: u32,
    /// This pass changed mainline without starting a worker (a claim made
    /// and given back): the derived view is no longer current.
    churned: bool,
    /// The head the last termination check ran on (`--wait`).
    checked_head: Option<String>,
    audit_notes: Vec<String>,
    /// The run's `events.jsonl`, appended to as things happen, so a reader
    /// can see the history and not only the latest `run.json`.
    log: EventLog,
}

/// The first line of an attempt's reason, at most 200 chars, for
/// `run.json`; `None` when there is no reason (a verified attempt).
fn first_line_clipped(detail: &str) -> Option<String> {
    let line = detail.lines().next().unwrap_or("");
    if line.is_empty() {
        return None;
    }
    Some(line.chars().take(200).collect())
}

pub(crate) fn orchestrate(ctx: Ctx) -> Result<u8, CoreError> {
    let mut s = Scheduler::start(ctx)?;
    let end = s.drive();
    s.finish(end)
}

// ---------------------------------------------------------- dead runs

/// `run-<stamp>-p<pid>-w<n>` → (`run-<stamp>-p<pid>`, pid).
fn run_holder(holder: &str) -> Option<(String, u32)> {
    let (job, w) = holder.rsplit_once("-w")?;
    if w.is_empty() || !w.chars().all(|c| c.is_ascii_digit()) || !job.starts_with("run-") {
        return None;
    }
    let (_, pid) = job.rsplit_once("-p")?;
    Some((job.to_string(), pid.parse().ok()?))
}

#[derive(PartialEq)]
enum Liveness {
    Alive,
    Dead,
    /// No trace of the job in this checkout: another machine's, or another
    /// checkout's. Its lease is left to lapse.
    Foreign,
}

/// Whether the run `job` (which ran in this checkout, if at all) is gone.
/// A run holds a lock on `.vitrify/jobs/<job>.lock` for its whole life; the
/// operating system frees it however the process ends.
fn liveness(scratch: &Path, job: &str, pid: u32) -> Liveness {
    let lock = scratch.join("jobs").join(format!("{job}.lock"));
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&lock) {
        return match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                Liveness::Dead
            }
            Err(_) => Liveness::Alive,
        };
    }
    // a run from before job locks leaves only its log directory
    if scratch.join("logs").join(job).is_dir() {
        return if gate::alive(pid, false) {
            Liveness::Alive
        } else {
            Liveness::Dead
        };
    }
    Liveness::Foreign
}

/// Claims held by runs that are gone: (unit, holder, job, claim_seq).
fn dead_claims(ctx: &Ctx, inst: &Instance) -> Vec<(String, String, String, i64)> {
    let mut out = Vec::new();
    for u in &inst.units {
        if u.meta.get_str_or("status", "Draft") != "Claimed" {
            continue;
        }
        let holder = u.meta.get_str_or("claimed_by", "").to_string();
        let Some((job, pid)) = run_holder(&holder) else {
            continue;
        };
        if job != ctx.job_id && liveness(&ctx.ml.layout.scratch, &job, pid) == Liveness::Dead {
            out.push((u.unit.clone(), holder, job, u.meta.get_int("claim_seq")));
        }
    }
    out
}

/// `--dry-run`: what the run would release before dispatching.
pub(crate) fn dead_run_notes(ctx: &Ctx, inst: &Instance) -> Vec<String> {
    dead_claims(ctx, inst)
        .into_iter()
        .map(|(unit, holder, _, seq)| {
            format!(
                "Recovery: would release {unit} (c{seq}), claimed by {holder} — that run is gone"
            )
        })
        .collect()
}

// ---------------------------------------------------------- the scheduler

impl Scheduler {
    fn start(ctx: Ctx) -> Result<Scheduler, CoreError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source: std::io::Error| CoreError::Io { path, source }
        };
        ctx.ml.layout.ensure_scratch()?;
        std::fs::create_dir_all(&ctx.logs_dir).map_err(io(&ctx.logs_dir))?;
        let log = EventLog::open(&ctx.logs_dir);
        let jobs_dir = ctx.ml.layout.scratch.join("jobs");
        std::fs::create_dir_all(&jobs_dir).map_err(io(&jobs_dir))?;
        let lock_path = jobs_dir.join(format!("{}.lock", ctx.job_id));
        let job_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io(&lock_path))?;
        // Nobody else can hold this name for long — a recovering run only
        // probes a lock and lets go — so wait for it rather than run without.
        job_lock.lock().map_err(io(&lock_path))?;

        let params = MonitorParams {
            lease_minutes: ctx.cfg.run.lease_minutes,
            renew_fraction: ctx.cfg.run.renew_fraction,
            worker_timeout_minutes: ctx.cfg.run.worker_timeout_minutes,
            tick: TICK,
        };
        let (events_tx, events) = mpsc::channel::<Event>();
        let (jobs, jobs_rx) = mpsc::channel::<Job>();

        let integrator = Integrator::new(ctx.ml.clone(), ctx.cfg.run.rows_at_once())
            .with_board(GateBoard::to(ctx.logs_dir.join("gate.json")).with_events(log.clone()));
        let memo = GateMemo {
            suite: None,
            tables_green_at: ctx.audited_at.clone(),
        };
        let tx = events_tx.clone();
        let integrator = std::thread::spawn(move || integrator_loop(integrator, jobs_rx, tx, memo));

        let keeper = Arc::new(Keeper::default());
        let keeper_thread = {
            let keeper = Arc::clone(&keeper);
            let params = params.clone();
            let backend = RealLease {
                ml: ctx.ml.clone(),
                lease_minutes: ctx.cfg.run.lease_minutes,
            };
            std::thread::spawn(move || keeper.run(&params, &lease::RealClock, &backend))
        };

        let head_wt = ctx
            .ml
            .layout
            .scratch
            .join(format!("tx-{}-head", std::process::id()));
        Ok(Scheduler {
            started_at: UtcTime::now().format(),
            state: RunState::default(),
            events_tx,
            events,
            jobs,
            integrator: Some(integrator),
            keeper,
            keeper_thread: Some(keeper_thread),
            params,
            job_lock: Some(job_lock),
            head_wt,
            active: BTreeMap::new(),
            parked: BTreeMap::new(),
            fails: BTreeMap::new(),
            worker_n: 0,
            escalation_index: 0,
            inst: None,
            views: Vec::new(),
            head: String::new(),
            healed_at: String::new(),
            last_derive: Instant::now(),
            dirty: true,
            abort: None,
            failure: None,
            races_lost: 0,
            churned: false,
            checked_head: None,
            audit_notes: Vec::new(),
            log,
            ctx,
        })
    }

    fn record(&self, terminated: Option<(&str, &str)>) {
        if let Err(e) = write_run_json(&self.ctx, &self.state, &self.started_at, terminated) {
            diag(&format!("warning: run.json not written: {e}"));
        }
    }

    /// Log a `waiting: …` line the run printed while it waits under `--wait`.
    fn waiting(&self, line: &str) {
        self.event(RunEvent::new(UtcTime::now(), "waiting", None, None, line));
    }

    /// Append one event to the run's event log.
    fn event(&self, e: RunEvent) {
        self.log.write(&e);
    }

    fn limit(&self) -> u32 {
        self.ctx.cfg.run.abort_failed_waves.max(1) as u32
    }

    // ------------------------------------------------------------ loop

    fn drive(&mut self) -> Result<End, CoreError> {
        // the run's own record exists before anything is dispatched
        self.record(None);
        let sdd = crate::cmd::sdd_arg(&self.ctx.ml.sdd_rel);
        let harness = self.ctx.harness.clone();
        let cap = self.ctx.cap;
        let detail = if harness.is_empty() {
            format!("{sdd} · cap {cap}")
        } else {
            format!("{sdd} · {harness} · cap {cap}")
        };
        self.event(
            RunEvent::new(UtcTime::now(), "run-started", None, None, detail)
                .with("cap", J::Int(cap as i64))
                .with("harness", J::s(harness))
                .with("sdd", J::s(sdd)),
        );
        self.recover_dead_runs()?;

        loop {
            if let Some(sig) = signals::pending() {
                return Ok(End::Interrupted(sig));
            }
            if let Some(e) = self.failure.take() {
                return Err(CoreError::refusal(1, e));
            }
            if let Some(reason) = self.abort.take() {
                return Ok(End::Aborted(reason));
            }

            // nothing reported: look at mainline again every poll period —
            // spec changes and lapsed leases arrive without an event
            self.integrator_alive()?;
            let poll = Duration::from_secs(self.ctx.cfg.run.poll_seconds.max(1) as u64);
            if self.dirty || self.last_derive.elapsed() >= poll {
                if !self.derive()? {
                    // mainline could not be brought up to date this time:
                    // nothing is decided on a view that may be stale
                    std::thread::sleep(Duration::from_millis(300));
                    self.dirty = true;
                    continue;
                }
                self.churned = false;
                let dispatched = self.fill_slots()?;
                self.dirty = false;

                if self.active.is_empty() && dispatched == 0 && self.abort.is_none() {
                    if self.churned {
                        // a claim was made and given back: derive again
                        // before deciding anything (bounded: each such
                        // attempt counts toward setting the unit aside)
                        self.dirty = true;
                        continue;
                    }
                    if (1..=3).contains(&self.races_lost) {
                        // a claim lost only to a busy mainline: look again
                        self.races_lost += 1;
                        std::thread::sleep(Duration::from_millis(300));
                        self.dirty = true;
                        continue;
                    }
                    if let Some(end) = self.conclude()? {
                        return Ok(end);
                    }
                }
            }
            match self.events.recv_timeout(TICK) {
                Ok(event) => {
                    self.handle(event)?;
                    while let Ok(event) = self.events.try_recv() {
                        self.handle(event)?;
                    }
                    self.dirty = true;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(CoreError::refusal(
                        1,
                        "error: the orchestrator's threads are gone",
                    ));
                }
            }
        }
    }

    /// Read the mainline head: apply staleness where the spec moved, load
    /// the SDD, compute the views, return parked units whose reading set
    /// changed to the pool, and stop workers building from text that is no
    /// longer current. `false` = mainline was too busy to be put right;
    /// the view was left as it was.
    fn derive(&mut self) -> Result<bool, CoreError> {
        let mut head = self.ctx.ml.head()?;
        if self.healed_at.is_empty() {
            self.healed_at = head.clone(); // preflight left the audit clean
        }
        if head != self.healed_at {
            if integrate::spec_moved(&self.ctx.ml, &self.healed_at, &head)? {
                let healed = match self.ctx.ml.heal(false, true) {
                    Ok(healed) => healed,
                    Err(e) if e.exit_code() == 5 => {
                        // mainline too busy to publish it now: next pass
                        diag(&format!("note: {e}"));
                        return Ok(false);
                    }
                    Err(e) => return Err(e),
                };
                for (unit, cause) in &healed.stale {
                    emit(&format!("stale {unit}: {cause} — it will be rebuilt"));
                    self.event(RunEvent::new(
                        UtcTime::now(),
                        "stale",
                        Some(unit),
                        None,
                        cause.clone(),
                    ));
                }
                let notes: Vec<String> = healed
                    .other
                    .iter()
                    .filter(|f| f.check != 7)
                    .map(|f| format!("audit [check {}] {}: {}", f.check, f.subject, f.cause))
                    .collect();
                if notes != self.audit_notes {
                    for n in &notes {
                        diag(n);
                    }
                    self.audit_notes = notes;
                }
                if healed.commit.is_some() {
                    head = self.ctx.ml.head()?;
                }
            }
            self.healed_at = head.clone();
        }
        {
            // the head checkout is one more worktree: its turn like any other
            let _turn = claim::tx_lock();
            self.ctx.ml.checkout_at(&self.head_wt, &head)?;
        }
        let inst = self.ctx.ml.sdd_in(&self.head_wt)?;
        let now = UtcTime::now();
        self.views = inst
            .units
            .iter()
            .map(|u| model::unit_view(u, &inst.units, now))
            .collect();
        self.views.sort_by(|a, b| a.unit.cmp(&b.unit));

        // a parked unit rejoins when its reading set is no longer what it
        // was parked on
        let mut back: Vec<String> = Vec::new();
        for (unit, parked) in &self.parked {
            if let Some(u) = inst.unit(unit)
                && trace::compute(&inst, u)?.0 != parked.trace
            {
                back.push(unit.clone());
            }
        }
        for unit in back {
            self.parked.remove(&unit);
            self.fails.remove(&unit);
            emit(&format!(
                "{unit}: its reading set changed — back in the pool"
            ));
            self.event(RunEvent::new(
                UtcTime::now(),
                "back-in-pool",
                Some(&unit),
                None,
                "its reading set changed",
            ));
        }
        // a worker whose unit's inputs changed under it is building the
        // wrong thing: the gate would reject it; stop it now
        let mut stale: Vec<u32> = Vec::new();
        for (unit, a) in self.active.iter_mut() {
            if a.phase == Phase::Working
                && a.killed.is_none()
                && let Some(u) = inst.unit(unit)
                && trace::compute(&inst, u)?.0 != a.trace
            {
                diag(&format!(
                    "{unit}: its inputs changed on {} while it was being built — stopping the worker; the unit is rebuilt from the current text",
                    self.ctx.ml.branch
                ));
                a.killed = Some("stale-inputs");
                stale.push(a.pid);
            }
        }
        for pid in stale {
            gate::kill_group(pid);
        }

        self.inst = Some(inst);
        self.head = head;
        self.last_derive = Instant::now();
        Ok(true)
    }

    /// The integrator thread ends only when told to. If it is gone, units
    /// waiting for the gate would wait forever with their leases renewed.
    fn integrator_alive(&self) -> Result<(), CoreError> {
        if self.integrator.as_ref().is_some_and(|t| t.is_finished()) {
            return Err(CoreError::refusal(
                1,
                "error: the integrator thread ended unexpectedly",
            ));
        }
        Ok(())
    }

    /// Start a worker for each claimable unit while slots are free.
    fn fill_slots(&mut self) -> Result<usize, CoreError> {
        let mut dispatched = 0;
        let candidates: Vec<(String, bool)> = self
            .views
            .iter()
            .filter(|v| {
                v.claimable
                    && !self.active.contains_key(&v.unit)
                    && !self.parked.contains_key(&v.unit)
            })
            .map(|v| (v.unit.clone(), v.status == "Stale"))
            .collect();
        for (unit, was_stale) in candidates {
            // the cap bounds running workers; a unit waiting for the gate
            // holds a lease, not a slot
            let working = self
                .active
                .values()
                .filter(|a| a.phase == Phase::Working)
                .count();
            if working >= self.ctx.cap || signals::interrupted() || self.abort.is_some() {
                break;
            }
            if self.dispatch(&unit, was_stale)? {
                dispatched += 1;
            }
        }
        Ok(dispatched)
    }

    // -------------------------------------------------------- dispatch

    /// Claim `unit`, give it a worktree, start its worker.
    fn dispatch(&mut self, unit: &str, was_stale: bool) -> Result<bool, CoreError> {
        self.worker_n += 1;
        let holder = format!("{}-w{}", self.ctx.job_id, self.worker_n);
        let lease = self.ctx.cfg.run.lease_minutes;
        let outcome = match self.ctx.ml.claim(Op::Claim, unit, &holder, lease) {
            Ok(o) => o,
            Err(e) if matches!(e.exit_code(), 4 | 5) => {
                diag(&format!("claim lost for {unit}: {e}"));
                if e.exit_code() == 5 {
                    self.races_lost += 1; // mainline was busy, not the unit taken
                }
                return Ok(false);
            }
            Err(e) => return Err(e),
        };
        self.races_lost = 0;
        let seq = outcome.claim_seq;
        let branch = layout::unit_branch(&self.ctx.slug, unit, seq);
        let wt = self.ctx.ml.layout.unit_worktree(&self.ctx.slug, unit, seq);
        let primary = &self.ctx.ml.primary;

        // Both names belong to this claim alone. If either is taken, it is
        // by something this run did not make: leave it exactly as it is.
        let taken = if primary.branch_exists(&branch).map_err(CoreError::from)? {
            Some(format!("branch {branch}"))
        } else if wt.exists() {
            Some(format!("worktree {}", wt.display()))
        } else {
            None
        };
        if let Some(what) = taken {
            let why = format!(
                "{what} already exists and is not this run's — left untouched; remove or rename it if it is not wanted"
            );
            diag(&format!("{unit} (c{seq}): {why}"));
            self.release(unit, &holder);
            self.state
                .attempts
                .push((unit.to_string(), "name-taken".to_string(), why));
            self.count_failure(unit, "name-taken", false)?;
            // mainline has moved (a claim and its release): what this pass
            // derived is no longer the state to conclude on
            self.churned = true;
            return Ok(false);
        }

        let expires = outcome
            .claim_expires
            .as_deref()
            .and_then(UtcTime::parse)
            .unwrap_or_else(|| UtcTime::now().add_minutes(self.ctx.cfg.run.lease_minutes));
        match self.launch(unit, &holder, seq, &branch, &wt, &outcome.commit, was_stale) {
            Ok((handle, tmp, trace, retried)) => {
                let pid = handle.pid;
                let out_path = handle.out_path.clone();
                let started = UtcTime::now();
                self.state.workers.push(WorkerRec {
                    holder: holder.clone(),
                    pid,
                    unit: unit.to_string(),
                    claim_seq: seq,
                    branch: branch.clone(),
                    worktree: wt.display().to_string(),
                    started_at: started.format(),
                    ended_at: None,
                    outcome: "running".to_string(),
                    retry: retried,
                    detail: None,
                });
                let rec = self.state.workers.len() - 1;
                self.monitor(handle, unit, &holder, expires);
                self.active.insert(
                    unit.to_string(),
                    Active {
                        holder: holder.clone(),
                        seq,
                        branch,
                        wt,
                        pid,
                        out_path,
                        _tmp: tmp,
                        trace,
                        phase: Phase::Working,
                        rec,
                        killed: None,
                    },
                );
                emit(&format!(
                    "dispatch {unit} (c{seq}) → w{}{}",
                    self.worker_n,
                    if retried { "  [retry]" } else { "" }
                ));
                self.event(
                    RunEvent::new(
                        started,
                        "dispatched",
                        Some(unit),
                        Some(seq),
                        format!(
                            "→ w{}{}",
                            self.worker_n,
                            if retried { " (retry)" } else { "" }
                        ),
                    )
                    .with("holder", J::s(holder.clone()))
                    .with("retry", J::Bool(retried)),
                );
                self.record(None);
                Ok(true)
            }
            Err(e) => {
                // the worktree and branch are this claim's own, made a
                // moment ago and holding nothing: take them back
                if wt.join(".git").exists() {
                    let _ = primary.worktree_remove(&wt);
                }
                let _ = primary.delete_branch(&branch);
                self.release(unit, &holder);
                if e.exit_code() != 4 {
                    return Err(e);
                }
                // The unit cannot be handed to a worker as the spec stands
                // at its claim commit — its reading set does not resolve: a
                // pinned contract whose authority has moved on since
                // preflight, a consumed contract that is gone. No attempt
                // would change that, and it is no reason to stop building
                // the rest: the unit waits for the spec.
                let text = e.to_string();
                let why = text
                    .lines()
                    .map(|l| l.strip_prefix("error: ").unwrap_or(l))
                    .collect::<Vec<_>>()
                    .join("; ");
                diag(&format!(
                    "{unit} (c{seq}): not started — {why}; set aside until the spec changes"
                ));
                self.state.attempts.push((
                    unit.to_string(),
                    "not-started".to_string(),
                    why.clone(),
                ));
                self.park(unit, format!("its reading set does not resolve: {why}"))?;
                // mainline has moved (a claim and its release)
                self.churned = true;
                Ok(false)
            }
        }
    }

    /// The worktree, what the last attempt left, the prompt, the process.
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &self,
        unit: &str,
        holder: &str,
        seq: i64,
        branch: &str,
        wt: &Path,
        claim_commit: &str,
        was_stale: bool,
    ) -> Result<(worker::WorkerHandle, gate::TmpSlot, TraceMap, bool), CoreError> {
        let ml = &self.ctx.ml;
        if let Some(parent) = wt.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        ml.primary
            .worktree_add_branch(wt, branch, claim_commit)
            .map_err(CoreError::from)?;

        // everything the worker is told is read from its own worktree, at
        // the claim commit it was forked from
        let inst = ml.sdd_in(wt)?;
        let Some(u) = inst.unit(unit) else {
            return Err(CoreError::refusal(
                4,
                format!("error: unit not found at the claim commit: {unit}"),
            ));
        };
        let (trace_at_claim, _) = trace::compute(&inst, u)?;

        // what the last attempt left: its owned files, and why it failed
        let prev = retry::previous_attempt(&ml.primary, &inst, unit, seq)?;
        let mut seeded: Vec<String> = Vec::new();
        if let Some(prev) = &prev {
            let ws = WriteSet::of(&inst, u, wt)?;
            match retry::seed(&ml.primary.at(wt), &ws, claim_commit, prev) {
                Ok(paths) => seeded = paths,
                Err(e) => diag(&format!(
                    "warning: {unit}: could not carry over the files of {} ({e}); the worker starts without them",
                    prev.branch
                )),
            }
        }
        let stale_cause = if was_stale {
            Some(stale_cause(&inst, unit))
        } else {
            None
        };
        let sdd = crate::cmd::sdd_arg(&ml.sdd_rel);
        let session = retry::section(
            Some(&retry::Orchestrated {
                unit,
                holder,
                seq,
                branch,
                worktree: wt,
                sdd: &sdd,
            }),
            prev.as_ref(),
            &seeded,
            stale_cause.as_deref(),
        );
        let prompt =
            brief::build_worker_prompt_at(wt, unit, holder, &ml.sdd_rel, &ml.layout, &session)?;
        let _ = std::fs::write(
            self.ctx.logs_dir.join(format!("worker-{holder}.prompt.md")),
            &prompt,
        );

        // an attempt after a failed one runs under the retry profile
        let argv = if prev.is_some() {
            &self.ctx.retry_argv
        } else {
            &self.ctx.worker_argv
        };
        // a temp directory no other worker of this repository is using
        let tmp = gate::tmp_slot(&ml.layout.wt_root, "w").map_err(|source| CoreError::Io {
            path: ml.layout.wt_root.clone(),
            source,
        })?;
        let handle = worker::spawn_in(
            argv,
            &prompt,
            wt,
            &self.ctx.logs_dir,
            holder,
            unit,
            Some(&tmp.dir),
        )?;
        Ok((handle, tmp, trace_at_claim, prev.is_some()))
    }

    /// One thread per worker: renew its lease, enforce the hard budget,
    /// report its exit.
    fn monitor(&self, handle: worker::WorkerHandle, unit: &str, holder: &str, expires: UtcTime) {
        let tx = self.events_tx.clone();
        let params = self.params.clone();
        let backend = RealLease {
            ml: self.ctx.ml.clone(),
            lease_minutes: self.ctx.cfg.run.lease_minutes,
        };
        let unit = unit.to_string();
        let holder = holder.to_string();
        std::thread::spawn(move || {
            let mut child = handle.child;
            let pid = handle.pid;
            let (verdict, expires) = lease::monitor_worker_tracked(
                lease::MonitorTarget {
                    child: &mut child,
                    pid,
                    unit: &unit,
                    holder: &holder,
                    expires,
                },
                &params,
                &lease::RealClock,
                &backend,
            );
            let _ = tx.send(Event::Exited {
                unit,
                verdict,
                expires,
            });
        });
    }

    // ---------------------------------------------------------- events

    fn handle(&mut self, event: Event) -> Result<(), CoreError> {
        match event {
            Event::Exited {
                unit,
                verdict,
                expires,
            } => self.on_exit(&unit, verdict, expires),
            Event::Integrated { unit, outcome } => self.on_integrated(&unit, outcome),
            Event::IntegratorFailed(e) => {
                self.failure = Some(e);
                Ok(())
            }
            // only `conclude` asks for one, and it takes the answer itself
            Event::HeadChecked(_) => Ok(()),
        }
    }

    fn on_exit(
        &mut self,
        unit: &str,
        verdict: MonitorVerdict,
        expires: UtcTime,
    ) -> Result<(), CoreError> {
        let Some(a) = self.active.get(unit) else {
            return Ok(());
        };
        let (holder, seq, wt, out_path, killed) = (
            a.holder.clone(),
            a.seq,
            a.wt.clone(),
            a.out_path.clone(),
            a.killed,
        );
        let w = holder.rsplit('-').next().unwrap_or(&holder).to_string();

        if let Some(why) = killed {
            // the scheduler stopped this worker; there is no report to read
            let detail = match why {
                "stale-inputs" => format!(
                    "the unit's inputs changed on {} while it was being built",
                    self.ctx.ml.branch
                ),
                _ => "the run ended before the worker finished".to_string(),
            };
            return self.fail(unit, why, &detail, Counts::No);
        }

        let stdout_text = std::fs::read_to_string(&out_path).unwrap_or_default();
        let (report, parsed) = worker::report_or_released(&stdout_text, unit);
        self.state.reports.push(render_report_line(&report));
        // surface the worker's own account live — this is usually the
        // line that names the failure cause
        if parsed {
            diag(&format!(
                "report [{w}] {unit}: {}{}",
                report.outcome,
                if report.notes.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", report.notes)
                }
            ));
            if report.unit != unit {
                diag(&format!(
                    "warning: [{w}] reported for {} but was dispatched for {unit}; taken as {unit}",
                    report.unit
                ));
            }
        } else {
            diag(&format!(
                "report [{w}] {unit}: no parseable report line (treated as released) — see {}",
                out_path.display()
            ));
        }

        match verdict {
            MonitorVerdict::TimedOut => {
                diag(&format!(
                    "worker {holder} on {unit} exceeded the hard budget — killed; claim released"
                ));
                self.fail(
                    unit,
                    "timeout",
                    &format!(
                        "the worker exceeded the hard budget of {} minutes and was stopped",
                        self.ctx.cfg.run.worker_timeout_minutes
                    ),
                    Counts::Work,
                )
            }
            MonitorVerdict::LeaseLost => {
                diag(&format!(
                    "worker {holder} on {unit} lost its lease — killed; the unit returns to the pool"
                ));
                self.fail(
                    unit,
                    "lease-lost",
                    "the lease could not be renewed and expired",
                    Counts::Quietly,
                )
            }
            MonitorVerdict::Exited => match report.outcome.as_str() {
                "built" if parsed => {
                    commit_handoff_if_dirty(&self.ctx.ml, &wt, unit, seq);
                    // the lease is the orchestrator's to keep until the
                    // gate has decided
                    self.keeper.hold(unit, &holder, expires);
                    if let Some(a) = self.active.get_mut(unit) {
                        a.phase = Phase::Integrating;
                        if let Some(rec) = self.state.workers.get_mut(a.rec) {
                            rec.outcome = "integrating".to_string();
                        }
                    }
                    self.event(RunEvent::new(
                        UtcTime::now(),
                        "integrating",
                        Some(unit),
                        Some(seq),
                        report.notes.clone(),
                    ));
                    self.record(None);
                    let _ = self.jobs.send(Job::Integrate(IntegrateRequest {
                        unit: unit.to_string(),
                        token_seq: seq,
                        builder_holder: holder,
                        integrator_id: format!("{}-integrator", self.ctx.job_id),
                    }));
                    Ok(())
                }
                "blocked" if parsed => {
                    let q = report.q.clone().unwrap_or_else(|| "Q-???".to_string());
                    self.record_question(&q, unit, &holder, &report.notes)?;
                    self.park(unit, format!("open {q}: {}", report.notes))?;
                    self.fail(
                        unit,
                        "blocked",
                        &format!("{q}: {}", report.notes),
                        Counts::No,
                    )
                }
                _ => {
                    let detail = if report.notes.is_empty() {
                        "the worker ended without building".to_string()
                    } else {
                        report.notes.clone()
                    };
                    self.fail(unit, "released", &detail, Counts::Work)
                }
            },
        }
    }

    fn on_integrated(&mut self, unit: &str, outcome: Outcome) -> Result<(), CoreError> {
        self.keeper.drop_unit(unit);
        let Some(a) = self.active.get(unit) else {
            return Ok(());
        };
        let (seq, branch, holder) = (a.seq, a.branch.clone(), a.holder.clone());
        match outcome {
            Outcome::Verified { .. } => {
                emit(&format!("integrated {unit} (c{seq})"));
                self.fails.remove(unit);
                self.state
                    .attempts
                    .push((unit.to_string(), "verified".to_string(), String::new()));
                self.close(unit, "verified", "");
                Ok(())
            }
            Outcome::RejectedFencing { reason } => {
                diag(&format!("fencing rejection for {unit}: {reason}"));
                self.fail(unit, "rejected-fencing", &reason, Counts::Quietly)
            }
            Outcome::SkippedLeaseExpired => {
                diag(&format!(
                    "{unit}: token current but lease expired — back to the pool"
                ));
                self.fail(
                    unit,
                    "skipped",
                    "the lease had expired when the gate started",
                    Counts::Quietly,
                )
            }
            Outcome::RejectedWriteSet { paths, detail } => {
                diag(&format!(
                    "write-set violation integrating {unit}: {}",
                    paths.join(", ")
                ));
                self.state.gate_failures.push((
                    unit.to_string(),
                    "write-set check".to_string(),
                    detail.clone(),
                ));
                self.fail(
                    unit,
                    "rejected-write-set",
                    &format!("the work changed paths outside the unit's write-set:\n{detail}"),
                    Counts::Work,
                )
            }
            Outcome::RejectedNotBuilt { reason } => {
                diag(&format!("{unit}: {reason}"));
                self.state.gate_failures.push((
                    unit.to_string(),
                    "Built stamp".to_string(),
                    reason.clone(),
                ));
                self.fail(unit, "rejected-not-built", &reason, Counts::Work)
            }
            Outcome::RejectedStaleInputs { keys } => {
                let detail = format!(
                    "built from text that has since changed on {}: {}",
                    self.ctx.ml.branch,
                    keys.join(", ")
                );
                diag(&format!(
                    "{unit}: {detail} — it is rebuilt from the current text"
                ));
                self.fail(unit, "stale-inputs", &detail, Counts::No)
            }
            Outcome::RejectedGate { command, output } => {
                diag(&format!("gate red integrating {unit}: {command}"));
                self.state
                    .gate_failures
                    .push((unit.to_string(), command.clone(), output.clone()));
                self.fail(
                    unit,
                    "rejected-gate",
                    &format!(
                        "failing gate: `{command}`\n{}",
                        retry::clip(&output, 40, 4000)
                    ),
                    Counts::Work,
                )
            }
            Outcome::OwnershipConflict { paths } => {
                let verdict = self.escalate(
                    unit,
                    &branch,
                    "merge conflict on paths this unit may write — mainline also changed them after the branch forked",
                    &paths.join("\n"),
                )?;
                let detail = format!(
                    "merge conflict with {} on: {}",
                    self.ctx.ml.branch,
                    paths.join(", ")
                );
                self.enforce(unit, &holder, verdict)?;
                self.fail(unit, "ownership-conflict", &detail, Counts::Quietly)
            }
            Outcome::LostRace => {
                diag(&format!(
                    "integration of {unit} lost the publish race after retries"
                ));
                self.fail(
                    unit,
                    "lost-race",
                    "mainline kept moving; the integration was not published",
                    Counts::Quietly,
                )
            }
            // the run is ending: `finish` sets the work aside
            Outcome::Interrupted => Ok(()),
        }
    }

    // ------------------------------------------------------- outcomes

    /// End this run's record of an attempt, with why it ended (`""` when
    /// it verified), and log its outcome.
    fn close(&mut self, unit: &str, outcome: &str, detail: &str) {
        let detail = first_line_clipped(detail);
        if let Some(a) = self.active.remove(unit)
            && let Some(rec) = self.state.workers.get_mut(a.rec)
        {
            let ended = UtcTime::now();
            let duration_s = UtcTime::parse(&rec.started_at)
                .map(|s| ended.0 - s.0)
                .unwrap_or(0);
            rec.outcome = outcome.to_string();
            rec.ended_at = Some(ended.format());
            rec.detail = detail.clone();
            let text = match &detail {
                Some(d) => format!("{outcome} — {d}"),
                None => outcome.to_string(),
            };
            let e = RunEvent::new(ended, "outcome", Some(unit), Some(a.seq), text)
                .with("duration_s", J::Int(duration_s))
                .with("outcome", J::s(outcome));
            self.event(e);
        }
        self.record(None);
    }

    /// An attempt that did not land: its work goes to its branch with the
    /// reason, its worktree is removed, its claim released.
    fn fail(
        &mut self,
        unit: &str,
        label: &str,
        detail: &str,
        counts: Counts,
    ) -> Result<(), CoreError> {
        if let Some(a) = self.active.get(unit) {
            retry::abandon(&self.ctx.ml.primary, &a.wt, unit, a.seq, label, detail);
            let holder = a.holder.clone();
            self.release(unit, &holder);
        }
        self.state
            .attempts
            .push((unit.to_string(), label.to_string(), detail.to_string()));
        self.close(unit, label, detail);
        match counts {
            Counts::No => Ok(()),
            Counts::Quietly => self.count_failure(unit, label, false),
            Counts::Work => self.count_failure(unit, label, true),
        }
    }

    /// Count a failed attempt. At the limit, work failures get one
    /// escalation call; at twice the limit the unit is set aside.
    fn count_failure(&mut self, unit: &str, label: &str, work: bool) -> Result<(), CoreError> {
        let limit = self.limit();
        let (all, ask) = {
            let f = self.fails.entry(unit.to_string()).or_default();
            f.all += 1;
            if work {
                f.work += 1;
            }
            // the judge is asked once: when the work itself — not a lost
            // race or a lapsed lease in between — has failed `limit` times
            let ask = work && f.work >= limit && !f.escalated;
            if ask {
                f.escalated = true;
            }
            (f.all, ask)
        };
        if ask {
            let (command, output) = self
                .state
                .gate_failures
                .iter()
                .rev()
                .find(|(u, _, _)| u == unit)
                .map(|(_, c, o)| (c.clone(), o.clone()))
                .unwrap_or_else(|| {
                    let last = self
                        .state
                        .attempts
                        .iter()
                        .rev()
                        .find(|(u, _, _)| u == unit)
                        .map(|(_, l, d)| format!("{l}: {d}"))
                        .unwrap_or_default();
                    (format!("worker attempt ({label})"), last)
                });
            let verdict = self.escalate(unit, "-", &command, &output)?;
            self.enforce(unit, "escalation", verdict)?;
        }
        if all >= limit * 2 && !self.parked.contains_key(unit) && self.abort.is_none() {
            diag(&format!(
                "{unit}: {all} attempts did not land (last: {label}) — set aside for this run"
            ));
            return self.park(
                unit,
                format!("set aside after {all} attempts that did not land; last outcome {label}"),
            );
        }
        Ok(())
    }

    /// Stop dispatching `unit` until its reading set changes.
    fn park(&mut self, unit: &str, reason: String) -> Result<(), CoreError> {
        let trace = match &self.inst {
            Some(inst) => match inst.unit(unit) {
                Some(u) => trace::compute(inst, u)?.0,
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        self.parked
            .insert(unit.to_string(), Parked { reason, trace });
        Ok(())
    }

    /// Land a `Q-###` in the Decision Log. A mainline too busy to take it
    /// does not end the run: the question is in the run's own record, and
    /// the unit is parked either way.
    fn record_question(
        &self,
        q: &str,
        unit: &str,
        reporter: &str,
        notes: &str,
    ) -> Result<(), CoreError> {
        match self.ctx.ml.land_q_entry(q, unit, reporter, notes) {
            Err(e) if e.exit_code() == 5 => {
                diag(&format!(
                    "warning: {q} for {unit} was not recorded in the Decision Log ({e}); record it by hand: {notes}"
                ));
                Ok(())
            }
            other => other,
        }
    }

    fn release(&self, unit: &str, holder: &str) {
        let lease = self.ctx.cfg.run.lease_minutes;
        // a release that only lost the race for the ref is tried again: a
        // claim left standing keeps the unit out of the pool for its lease
        for attempt in 0..3 {
            match self.ctx.ml.claim(Op::Release, unit, holder, lease) {
                Ok(_) => return,
                Err(e) if e.exit_code() == 4 => return, // not ours any more
                Err(e) if e.exit_code() == 5 && attempt < 2 => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => {
                    diag(&format!("release of {unit} failed: {e}"));
                    return;
                }
            }
        }
    }

    // ------------------------------------------------------ escalation

    fn escalate(
        &mut self,
        unit: &str,
        branch: &str,
        command: &str,
        output: &str,
    ) -> Result<escalate::Verdict, CoreError> {
        let (journal_tail, fencing) = match &self.inst {
            Some(inst) => {
                let (events, _) = journal::read(&inst.handle())?;
                let tail: Vec<String> = events
                    .iter()
                    .rev()
                    .take(20)
                    .rev()
                    .map(journal::render_line)
                    .collect();
                let fencing = inst
                    .unit(unit)
                    .map(|u| {
                        format!(
                            "status={}, claim_seq={}, lease to {}",
                            u.meta.get_str_or("status", "Draft"),
                            u.meta.get_int("claim_seq"),
                            u.meta.get_str_or("claim_expires", "—")
                        )
                    })
                    .unwrap_or_else(|| "-".to_string());
                (tail.join("\n"), fencing)
            }
            None => (String::new(), "-".to_string()),
        };
        let evidence = escalate::Evidence {
            unit: unit.to_string(),
            branch: branch.to_string(),
            fencing,
            command: command.to_string(),
            output_tail: output.to_string(),
            journal_tail,
        };
        // the escalation role's model/effort from ~/.vitrify/config.toml,
        // appended to the selected harness's recorded spelling
        let (verdict, evidence_path) = escalate::escalate(
            &self.ctx.home,
            VERSION,
            &self.ctx.escalation_argv,
            &evidence,
            &self.ctx.logs_dir,
            self.escalation_index,
        )?;
        self.escalation_index += 1;
        self.state.escalations.push((
            evidence_path.display().to_string(),
            verdict.verdict.as_str().to_string(),
        ));
        emit(&format!(
            "escalation verdict: {} ({})",
            verdict.verdict.as_str(),
            verdict.reason
        ));
        self.event(
            RunEvent::new(
                UtcTime::now(),
                "escalation",
                Some(unit),
                None,
                verdict.reason.clone(),
            )
            .with("verdict", J::s(verdict.verdict.as_str())),
        );
        Ok(verdict)
    }

    /// The orchestrator, not the judge, acts on a verdict. `retry` and
    /// `release` leave the unit in the pool (its claim is already
    /// released); `block` records the question and parks the unit; `abort`
    /// ends the run.
    fn enforce(
        &mut self,
        unit: &str,
        reporter: &str,
        verdict: escalate::Verdict,
    ) -> Result<(), CoreError> {
        match verdict.verdict {
            escalate::VerdictKind::Retry | escalate::VerdictKind::Release => Ok(()),
            escalate::VerdictKind::Block => {
                let q = verdict.q.clone().unwrap_or_else(|| "Q-???".into());
                let target = verdict.unit.clone().unwrap_or_else(|| unit.to_string());
                self.record_question(&q, &target, reporter, &verdict.reason)?;
                self.park(&target, format!("open {q}: {}", verdict.reason))
            }
            escalate::VerdictKind::Abort => {
                self.abort = Some(verdict.reason.clone());
                Ok(())
            }
        }
    }

    // -------------------------------------------------------- recovery

    /// Release the claims of runs that died in this checkout, and save what
    /// their workers had written.
    fn recover_dead_runs(&mut self) -> Result<(), CoreError> {
        self.derive()?;
        let Some(inst) = &self.inst else {
            return Ok(());
        };
        let dead = dead_claims(&self.ctx, inst);
        let mut jobs: Vec<String> = Vec::new();
        for (unit, holder, job, seq) in dead {
            emit(&format!(
                "recovered {unit} (c{seq}): its holder {holder} is gone — claim released"
            ));
            self.event(
                RunEvent::new(
                    UtcTime::now(),
                    "recovered",
                    Some(&unit),
                    Some(seq),
                    format!("its holder {holder} is gone — claim released"),
                )
                .with("holder", J::s(holder.clone())),
            );
            // whatever its worker wrote becomes the next attempt's draft
            let wt = self.ctx.ml.layout.unit_worktree(&self.ctx.slug, &unit, seq);
            retry::abandon(
                &self.ctx.ml.primary,
                &wt,
                &unit,
                seq,
                "interrupted",
                &format!("the run that held this claim ({job}) ended without releasing it"),
            );
            self.release(&unit, &holder);
            if !jobs.contains(&job) {
                jobs.push(job);
            }
        }
        for job in &jobs {
            self.sweep_dead_job(job);
        }
        if !jobs.is_empty() {
            self.dirty = true;
        }
        Ok(())
    }

    /// What a dead run left in `.vitrify/`: its lock, its transaction
    /// worktrees — and possibly workers that outlived it.
    fn sweep_dead_job(&self, job: &str) {
        let scratch = &self.ctx.ml.layout.scratch;
        let _ = std::fs::remove_file(scratch.join("jobs").join(format!("{job}.lock")));
        if let Some((_, pid)) = job.rsplit_once("-p")
            && let Ok(entries) = std::fs::read_dir(scratch)
        {
            let prefix = format!("tx-{pid}-");
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = self.ctx.ml.primary.worktree_remove(&entry.path());
                }
            }
        }
        // a run killed outright cannot stop its workers; say so
        let record = scratch.join("logs").join(job).join("run.json");
        let Ok(text) = std::fs::read_to_string(record) else {
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        for w in v["workers"].as_array().into_iter().flatten() {
            if w["outcome"] == "running"
                && let Some(pid) = w["pid"].as_u64()
                && gate::alive(pid as u32, true)
            {
                diag(&format!(
                    "note: a worker of {job} (process group {pid}, unit {}) is still running — stop it with: kill -9 -- -{pid}",
                    w["unit"].as_str().unwrap_or("?")
                ));
            }
        }
    }

    // ----------------------------------------------------- termination

    /// Nothing is running and nothing can be started. Decide whether that
    /// is the end, and which one.
    fn conclude(&mut self) -> Result<Option<End>, CoreError> {
        let Some(inst) = self.inst.clone() else {
            return Ok(None);
        };
        // Verified is a status, not a stamp: a change entry bumps the SDD's
        // version and stales only the units whose reading set it touched
        // (schema §11, §12); the others stay Verified with the stamp of the
        // version they were last built at. Whether each one's trace still
        // matches the current content is the head audit's check 5, below.
        let all_verified = inst.units.iter().all(|u| {
            let status = u.meta.get_str_or("status", "Draft");
            status == "Retired" || status == "Verified"
        });
        if self.ctx.wait {
            if self.checked_head.as_deref() == Some(self.head.as_str()) {
                return Ok(None); // already reported on this head
            }
            if !all_verified {
                self.checked_head = Some(self.head.clone());
                self.report(&inst, None, false, true);
                emit("waiting: the run resumes when the spec changes (interrupt to stop)");
                self.waiting("waiting: the run resumes when the spec changes (interrupt to stop)");
                return Ok(None);
            }
        }

        // the head itself: audit, and — when everything is Verified — the
        // whole suite. The integrator owns the checkout; ask it.
        let _ = self.jobs.send(Job::CheckHead {
            want_suite: all_verified,
        });
        let mut asked_at = Instant::now();
        let check = loop {
            match self.events.recv_timeout(TICK) {
                Ok(Event::HeadChecked(result)) => break (*result)?,
                Ok(other) => self.handle(other)?,
                Err(RecvTimeoutError::Timeout) => {
                    self.integrator_alive()?;
                    // an interrupt stops the commands the check runs, and
                    // the answer follows at once; if it does not, do not
                    // wait for it
                    if signals::pending().is_none() {
                        asked_at = Instant::now();
                    } else if asked_at.elapsed() > Duration::from_secs(10) {
                        break HeadCheck {
                            head: String::new(),
                            findings: Vec::new(),
                            broken: None,
                            suite: None,
                            interrupted: true,
                        };
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(CoreError::refusal(
                        1,
                        "error: the integrator thread is gone",
                    ));
                }
            }
        };
        if let Some(sig) = signals::pending() {
            return Ok(Some(End::Interrupted(sig)));
        }
        if check.interrupted {
            return Ok(None);
        }
        let (complete, lines) = self.report(&inst, Some(&check), all_verified, false);
        if complete {
            self.keep_report(lines);
            return Ok(Some(End::Complete));
        }
        if self.ctx.wait {
            self.checked_head = Some(self.head.clone());
            emit("waiting: the run resumes when the repository changes (interrupt to stop)");
            self.waiting(
                "waiting: the run resumes when the repository changes (interrupt to stop)",
            );
            return Ok(None);
        }
        self.keep_report(lines);
        Ok(Some(End::Blocked))
    }

    /// Print the termination report; returns whether the done-condition
    /// holds, and the lines printed from the status line on. `check` is the
    /// state of the head (absent when the run is ending without one:
    /// interrupted, aborted, or still waiting).
    fn report(
        &self,
        inst: &Instance,
        check: Option<&HeadCheck>,
        all_verified: bool,
        interim: bool,
    ) -> (bool, Vec<String>) {
        let mut lines: Vec<String> = Vec::new();
        let mut emit = |line: &str| lines.push(line.to_string());
        let (events, _) = journal::read(&inst.handle()).unwrap_or_default();
        let proj = journal::replay(&events);
        // consistent = the journal projects what front matter says (with no
        // events at all, that nothing is beyond Ready)
        let replay_clean =
            proj.order_findings.is_empty() && journal::verify(&proj, &inst.units).is_empty();
        let progress = ledger::progress_line(&inst.units);
        let suite_ok = check
            .and_then(|c| c.suite.as_ref())
            .is_none_or(|(_, passed, _)| *passed);
        let complete = all_verified
            && replay_clean
            && !events.is_empty()
            && suite_ok
            && check.is_some_and(|c| c.findings.is_empty() && c.broken.is_none());

        if complete {
            emit("Complete.");
            emit(&format!("  {progress}"));
            emit("  audit: exit 0 (clean)");
            emit("  replay: exit 0 (consistent)");
            if let Some((cmd, _, _)) = check.and_then(|c| c.suite.as_ref()) {
                emit(&format!("  §5 Test: exit 0 ({cmd})"));
            }
        } else {
            emit(if interim {
                "Blocked (waiting)."
            } else {
                "Blocked."
            });
            emit(&format!("  {progress}"));
            if let Some(c) = check {
                if let Some((cmd, output)) = &c.broken {
                    emit(&format!(
                        "  §5 command failing at the {} head: `{cmd}`: {}",
                        self.ctx.ml.branch,
                        tail(output, 3)
                    ));
                }
                if !c.findings.is_empty() {
                    emit(&format!("  audit: {} finding(s)", c.findings.len()));
                    for f in &c.findings {
                        emit(&format!(
                            "    [check {}] {}: {}",
                            f.check, f.subject, f.cause
                        ));
                    }
                }
                if let Some((cmd, false, output)) = &c.suite {
                    emit(&format!(
                        "  §5 Test failing at the {} head: `{cmd}`: {}",
                        self.ctx.ml.branch,
                        tail(output, 3)
                    ));
                }
            }
            if !replay_clean {
                emit("  replay: inconsistent (see `vitrify journal replay`)");
            }
            for v in &self.views {
                if v.status == "Verified" || v.status == "Retired" {
                    continue;
                }
                emit(&format!("  {}: {}", v.unit, self.blocker(v)));
            }
        }
        // the report ends with the journal tail
        emit("  journal tail:");
        for e in events.iter().rev().take(5).rev() {
            emit(&format!("    {}", journal::render_line(e)));
        }
        out::emit("");
        for line in &lines {
            out::emit(line);
        }
        (complete, lines)
    }

    /// Keep the final report: `report.txt` beside the job's logs, and the
    /// run record's `report` (written with the record's next update).
    fn keep_report(&mut self, lines: Vec<String>) {
        let path = self.ctx.logs_dir.join("report.txt");
        let mut text = lines.join("\n");
        text.push('\n');
        if let Err(e) =
            std::fs::create_dir_all(&self.ctx.logs_dir).and_then(|_| std::fs::write(&path, text))
        {
            diag(&format!("warning: {} not written: {e}", path.display()));
        }
        self.state.report = Some(lines);
    }

    /// Why a unit is not Verified, from the most specific cause down.
    fn blocker(&self, v: &UnitView) -> String {
        let attempts = || {
            self.state
                .attempts
                .iter()
                .filter(|(u, _, _)| u == &v.unit)
                .count()
        };
        if let Some(parked) = self.parked.get(&v.unit) {
            return parked.reason.clone();
        }
        if let Some((_, command, output)) = self
            .state
            .gate_failures
            .iter()
            .rev()
            .find(|(u, _, _)| u == &v.unit)
        {
            return format!("failing gate `{command}`: {}", tail(output, 3));
        }
        // the workers' own accounts: last attempt's outcome + notes
        if let Some((_, outcome, notes)) = self
            .state
            .attempts
            .iter()
            .rev()
            .find(|(u, _, _)| u == &v.unit)
        {
            let n = attempts();
            let first = notes.lines().next().unwrap_or("");
            return if first.is_empty() {
                format!("{n} attempt(s); last outcome {outcome}")
            } else {
                format!("{n} attempt(s); last outcome {outcome} — {first}")
            };
        }
        let unmet = |x: &UnitView| {
            x.depends_on
                .iter()
                .find(|(_, s)| s != "Built" && s != "Verified")
                .cloned()
        };
        if let Some((dep, status)) = unmet(v) {
            let mut chain = vec![format!("{dep} ({status})")];
            let mut cur = dep;
            while let Some((next, status)) =
                self.views.iter().find(|x| x.unit == cur).and_then(unmet)
            {
                if chain.len() > self.views.len() {
                    break; // a cycle lint would have refused; never loop on one
                }
                chain.push(format!("{next} ({status})"));
                cur = next;
            }
            return format!("dependency chain blocked: {}", chain.join(" ← "));
        }
        format!("not attempted this run (status {})", v.status)
    }

    // ---------------------------------------------------------- finish

    /// Stop whatever is still running, hand back every claim, close the
    /// record. Every way out of the loop comes through here.
    fn finish(mut self, end: Result<End, CoreError>) -> Result<u8, CoreError> {
        // The terminal's Ctrl-C reaches git too: a command cut down by the
        // signal fails, and that failure is the interrupt, not an error.
        let end = match (end, signals::pending()) {
            (Err(_), Some(sig)) => Ok(End::Interrupted(sig)),
            (end, _) => end,
        };
        let (label, detail): (&'static str, String) = match &end {
            Ok(End::Complete) => ("complete", "done-condition met".to_string()),
            Ok(End::Blocked) => {
                // the report's progress line, and where the blockers are
                let progress = self
                    .state
                    .report
                    .as_ref()
                    .and_then(|lines| lines.get(1))
                    .map(|l| l.trim())
                    .filter(|l| l.starts_with("Progress:"));
                let detail = match progress {
                    Some(p) => format!("{p} — the report names each blocker"),
                    None => "the report names each blocker".to_string(),
                };
                ("blocked", detail)
            }
            Ok(End::Aborted(reason)) => ("blocked", format!("escalation verdict abort: {reason}")),
            Ok(End::Interrupted(sig)) => ("interrupted", signals::name(*sig).to_string()),
            Err(e) => ("error", e.to_string()),
        };
        let unfinished = !matches!(&end, Ok(End::Complete) | Ok(End::Blocked));
        // What an attempt the run cuts short is recorded as. A run the
        // judge aborted ends "blocked", but its workers were not blocked on
        // anything: they were stopped.
        let cut_short = match &end {
            Ok(End::Aborted(_)) => "aborted",
            _ => label,
        };
        if unfinished {
            self.stop_work(cut_short);
        }
        // the threads end with the run
        let _ = self.jobs.send(Job::Stop);
        self.keeper.stop();
        if let Some(t) = self.integrator.take() {
            let _ = t.join();
        }
        if let Some(t) = self.keeper_thread.take() {
            let _ = t.join();
        }
        // whatever the integrator finished before it stopped
        while let Ok(event) = self.events.try_recv() {
            self.absorb(event);
        }
        if unfinished {
            self.hand_back(cut_short);
        }

        if let Ok(End::Aborted(reason)) = &end {
            emit(&format!("escalation verdict abort — stopping: {reason}"));
            if self.derive_quietly()
                && let Some(inst) = self.inst.clone()
            {
                let (_, lines) = self.report(&inst, None, false, false);
                self.keep_report(lines);
            }
        }
        self.record(Some((label, &detail)));
        self.event(
            RunEvent::new(
                UtcTime::now(),
                "run-ended",
                None,
                None,
                format!("{label}: {detail}"),
            )
            .with("state", J::s(label)),
        );

        // a primary checkout that could not follow gets one more try, and —
        // if it still cannot — the last word on how to bring it up to date
        let behind = {
            let _turn = claim::tx_lock();
            claim::catch_up_primary(&self.ctx.ml.primary, &self.ctx.ml.branch)
        };
        if let Some(warning) = behind {
            diag(&warning);
        }

        // this run's own scratch
        if self.head_wt.exists() {
            let _ = self.ctx.ml.primary.worktree_remove(&self.head_wt);
        }
        drop(self.job_lock.take());
        let _ = std::fs::remove_file(
            self.ctx
                .ml
                .layout
                .scratch
                .join("jobs")
                .join(format!("{}.lock", self.ctx.job_id)),
        );
        match end {
            Ok(End::Complete) => Ok(0),
            Ok(End::Blocked) | Ok(End::Aborted(_)) => Ok(2),
            Ok(End::Interrupted(sig)) => {
                emit("");
                emit(&format!(
                    "Interrupted ({}): workers stopped, claims released, work in progress kept on its branches. Run again to continue.",
                    signals::name(sig)
                ));
                signals::die_by(sig)
            }
            Err(e) => Err(e),
        }
    }

    /// Stop every worker and the gate, and wait for them to say so.
    fn stop_work(&mut self, why: &'static str) {
        signals::request_stop();
        let mut waiting = 0;
        for a in self.active.values_mut() {
            if a.phase == Phase::Working {
                if a.killed.is_none() {
                    a.killed = Some(why);
                }
                gate::kill_group(a.pid);
                waiting += 1;
            }
        }
        // each monitor reports its worker's end
        let deadline = Instant::now() + Duration::from_secs(30);
        while waiting > 0 && Instant::now() < deadline {
            match self.events.recv_timeout(Duration::from_millis(200)) {
                Ok(event) => {
                    if matches!(event, Event::Exited { .. }) {
                        waiting -= 1;
                    }
                    self.absorb(event);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// An event that arrives while the run is ending. A unit the integrator
    /// did publish is Verified, whatever else is going on; everything else
    /// is left for [`Scheduler::hand_back`].
    fn absorb(&mut self, event: Event) {
        match event {
            // its monitor has seen the worker end; `hand_back` does the rest
            Event::Exited { .. } => {}
            Event::Integrated {
                unit,
                outcome: Outcome::Verified { .. },
            } => {
                self.keeper.drop_unit(&unit);
                if let Some(a) = self.active.get(&unit) {
                    emit(&format!("integrated {unit} (c{})", a.seq));
                    self.state
                        .attempts
                        .push((unit.clone(), "verified".to_string(), String::new()));
                    self.close(&unit, "verified", "");
                }
            }
            _ => {}
        }
    }

    /// Every claim still held goes back: the work to its branch, the unit
    /// to the pool.
    fn hand_back(&mut self, why: &'static str) {
        let units: Vec<String> = self.active.keys().cloned().collect();
        for unit in units {
            let Some(a) = self.active.get(&unit) else {
                continue;
            };
            self.keeper.drop_unit(&unit);
            let detail = match a.phase {
                Phase::Working => "the run ended before the worker finished",
                Phase::Integrating => {
                    "the work was reported built; the run ended before the gate had decided"
                }
            };
            retry::abandon(&self.ctx.ml.primary, &a.wt, &unit, a.seq, why, detail);
            let holder = a.holder.clone();
            self.release(&unit, &holder);
            self.state
                .attempts
                .push((unit.clone(), why.to_string(), detail.to_string()));
            self.close(&unit, why, detail);
        }
    }

    /// A last look at the head for the final report; never fails the exit.
    fn derive_quietly(&mut self) -> bool {
        match self.derive() {
            Ok(_) => true,
            Err(e) => {
                diag(&format!("warning: final state not derived: {e}"));
                self.inst.is_some()
            }
        }
    }
}

impl Drop for Scheduler {
    /// A scheduler that goes away with workers still running — a panic
    /// unwinding through it; `finish` leaves none — stops them. Their
    /// claims are released by the next run, which finds this run's job
    /// lock free.
    fn drop(&mut self) {
        for a in self.active.values() {
            if a.phase == Phase::Working {
                gate::kill_group(a.pid);
            }
        }
    }
}

/// Whether a failed attempt counts toward the unit's limit, and whether
/// reaching the limit calls the escalation judge.
enum Counts {
    /// Not the attempt's doing (the spec changed, the run is ending, a
    /// question is open).
    No,
    /// Counted, but nothing a judge could rule on (a lost race, a lapsed
    /// lease, a taken name).
    Quietly,
    /// The work itself failed.
    Work,
}

/// The integrator thread: every built unit that is waiting, as one batch.
fn integrator_loop(
    integrator: Integrator,
    jobs: Receiver<Job>,
    events: Sender<Event>,
    memo: GateMemo,
) {
    let mut memo = memo;
    let mut carried: Option<Job> = None;
    loop {
        let job = match carried.take() {
            Some(job) => job,
            None => match jobs.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
        };
        match job {
            Job::Stop => break,
            Job::CheckHead { want_suite } => {
                let result = integrator.check_head(&mut memo, want_suite);
                let _ = events.send(Event::HeadChecked(Box::new(result)));
            }
            Job::Integrate(first) => {
                let mut batch = vec![first];
                loop {
                    match jobs.try_recv() {
                        Ok(Job::Integrate(req)) => batch.push(req),
                        Ok(other) => {
                            carried = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                match integrator.integrate_batch(&batch, &mut memo, None) {
                    Ok(outcomes) => {
                        for (req, outcome) in batch.iter().zip(outcomes) {
                            let _ = events.send(Event::Integrated {
                                unit: req.unit.clone(),
                                outcome,
                            });
                        }
                    }
                    Err(e) => {
                        let _ = events.send(Event::IntegratorFailed(e.to_string()));
                    }
                }
            }
        }
    }
}

/// Why a unit is Stale, from the journal: the declaring event's cause.
fn stale_cause(inst: &Instance, unit: &str) -> String {
    let (events, _) = journal::read(&inst.handle()).unwrap_or_default();
    for e in events.iter().rev() {
        if e.event == "stale" && e.unit.as_deref() == Some(unit) {
            return e
                .cause
                .clone()
                .unwrap_or_else(|| "one of its inputs changed".to_string());
        }
        if e.event == "change" && e.stale.iter().flatten().any(|u| u == unit) {
            return match &e.entry {
                Some(entry) => format!("change entry {entry}"),
                None => "a change entry".to_string(),
            };
        }
    }
    "one of its inputs changed".to_string()
}

fn render_report_line(r: &worker::Report) -> String {
    format!(
        "{{\"unit\":\"{}\",\"outcome\":\"{}\",\"branch\":\"{}\",\"claim_seq\":{},\"q\":{},\"notes\":\"{}\"}}",
        r.unit,
        r.outcome,
        r.branch,
        r.claim_seq,
        r.q.as_deref()
            .map(|q| format!("\"{q}\""))
            .unwrap_or_else(|| "null".into()),
        r.notes.replace('"', "'")
    )
}

/// Commit the handoff worktree with the fixed build message. Workers never
/// run `git commit`: protocol git operations belong to vitrify (RA-6), and a
/// worker is a harness session whose permission policy may forbid commits
/// anyway — a green build is reported `built` with the work left in place.
/// A clean tree (nothing new) is a no-op; a commit failure is diagnosed and
/// left for the integration gate to reject (the branch tip stays at the
/// claim commit).
fn commit_handoff_if_dirty(ml: &Mainline, wt: &Path, unit: &str, seq: i64) {
    if !wt.is_dir() {
        return;
    }
    let git = ml.primary.at(wt);
    match git.is_dirty() {
        Ok(true) => {}
        _ => return,
    }
    let committed = git
        .add_all()
        .and_then(|()| git.commit(&format!("sdd: build {unit} (c{seq})")));
    match committed {
        Ok(()) => diag(&format!("committed handoff worktree for {unit} (c{seq})")),
        Err(e) => diag(&format!("handoff commit for {unit} (c{seq}) failed: {e}")),
    }
}
