//! The orchestrator's loop.
//!
//! One scheduler thread owns every decision, over the run's lanes — one per
//! SDD ([`Lane`]). Around it: a monitor thread per running worker (lease
//! renewal, the hard budget, the exit), and per lane one integrator thread
//! (the lane's integration checkout and its gate) and one lease keeper
//! (work waiting for the gate). They report through a single channel, each
//! message naming its lane; nothing but the scheduler changes the run's
//! state. What the lanes share is the scheduler's: the worker cap, the
//! gate pool, the job lock, the event log, the run record.
//!
//! Dispatch is continuous: whenever a slot is free, every claimable unit is
//! a candidate — any two are independent by construction (neither depends on
//! the other, each has its dependencies built, and ownership is exclusive) —
//! and each built unit goes to its lane's integrator as it reports, to be
//! gated with whatever else is ready. State is re-derived from the mainline
//! head, never remembered: what a unit's last attempt left is in git, what
//! a change to the spec staled is in the traces.
//!
//! Work is kept. A unit whose last attempt was built but never landed, and
//! whose inputs have not moved since, is **adopted**: claimed again and sent
//! to the gate with no worker session, taking no slot. A unit whose rows
//! fail on their gate's base too is **rotten**: it is reported with the
//! culprit commit a search finds (off the critical path, on a thread of its
//! own), never rebuilt, and the units that depend on it wait while the rest
//! of the lane keeps going.
//!
//! A project run is the same loop over several lanes. Each pass orders every
//! lane's ready units by their critical-path priority over the whole
//! repository's unit graph (ties by slug, then unit id) and dispatches while
//! workers are below the cap; `run.lane_cap` admits lanes highest-priority
//! work first. A lane whose Verified units a change in another lane reached
//! re-checks them — after the merge, once nothing they depend on has work
//! pending — in one batch with no worker. A blocked, set-aside or rotten unit
//! holds up only what depends on it: every lane keeps going.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use vitrify_core::cascade::{Anchors, Closures, Ownership};
use vitrify_core::claims::Op;
use vitrify_core::facf;
use vitrify_core::graph::{self as unit_graph, self, Node, Priorities};
use vitrify_core::model::{self, Instance, UtcTime};
use vitrify_core::trace::{self, TraceMap};
use vitrify_core::writeset::WriteSet;
use vitrify_core::{CoreError, journal, ledger};

use super::events::{EventLog, RunEvent};
use super::integrate::{self, HeadCheck, IntegrateRequest, Outcome, Rechecked, RottenRow};
use super::lane::{self, Active, Checkout, Event, Job, Lane, LaneId, Phase, RealLease};
use super::lease::{self, MonitorParams, MonitorVerdict};
use super::mainline::Mainline;
use super::project::{
    self, LaneOutcome, LaneReport, LaneState, Plan, ProjectRecord, adopted_line, event_line,
};
use super::{
    Ctx, LaneSeed, RunState, VERSION, WorkerRec, escalate, plan, retry, tail, worker,
    write_run_json,
};
use crate::cmd::{brief, claim};
use crate::gate;
use crate::incremental::adopt;
use crate::incremental::blame::{PristineCheckout, ShellRunner};
use crate::incremental::culprit;
use crate::incremental::dirty::{Cascade, DiffCache, DirtySet, Mode, Snapshot};
use crate::layout;
use crate::out::{self, J, diag, emit};
use crate::signals;

/// How long the scheduler sleeps when nothing reports.
const TICK: Duration = Duration::from_secs(1);

enum End {
    Complete,
    Blocked,
    /// The escalation judge said the job cannot safely continue.
    Aborted(String),
    Interrupted(i32),
}

struct Scheduler {
    ctx: Ctx,
    state: RunState,
    started_at: String,
    events_tx: Sender<Event>,
    events: Receiver<Event>,
    params: MonitorParams,
    /// Held for the life of the run: a later run reads a free lock as
    /// "that run is gone".
    job_lock: Option<std::fs::File>,
    /// One per SDD the run builds.
    lanes: Vec<Lane>,
    /// Workers started this run, across lanes: the `w<n>` of holder IDs.
    worker_n: u32,
    /// Attempts adopted this run: the `a<n>` of their claims' holder IDs.
    adopt_n: u32,
    last_derive: Instant,
    dirty: bool,
    abort: Option<String>,
    failure: Option<String>,
    /// Claims lost to a publish race since the last one that succeeded.
    races_lost: u32,
    /// This pass changed mainline without starting a worker (a claim made
    /// and given back): the derived view is no longer current.
    churned: bool,
    /// The run's `events.jsonl`, appended to as things happen, so a reader
    /// can see the history and not only the latest `run.json`.
    log: EventLog,
    /// The repository at the head the lanes last derived from.
    repo: Option<Repo>,
    /// What the dirty set's diffs found at the last head read.
    diffs: DiffCache,
    /// Derives again in this pass, for what arrived as it derived.
    rederived: u8,
    /// The plan the header showed; its lanes carry their current states.
    plan: Plan,
    /// Units of the planned critical path, in the order they landed.
    critical_ran: Vec<Node>,
    /// Estimated seconds per unit, when `run.priority = "history"`.
    weights: Option<BTreeMap<Node, u64>>,
    /// The last pass written to the event log: (working, idle slots,
    /// ready units left waiting for a slot).
    last_pass: Option<(usize, usize, usize)>,
}

/// The whole repository at one head, as every lane's decisions read it: its
/// unit graph, which Verified units a change reached, and the priorities.
struct Repo {
    head: String,
    snapshot: Snapshot,
    /// `None` when the cascade could not be read.
    dirty: Option<DirtySet>,
    priorities: Option<Priorities>,
    /// The cycles of the whole unit graph, when it has any: what is on one
    /// or behind one is not dispatched.
    cycles: Option<graph::Cycles>,
}

impl Repo {
    /// Whether `n` is on a cycle, or waits behind one.
    fn cyclic(&self, n: &Node) -> bool {
        self.cycles
            .as_ref()
            .is_some_and(|c| c.waiting.contains(n) || c.cycles.iter().any(|p| p.contains(n)))
    }
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

pub(crate) fn orchestrate(ctx: Ctx, lanes: Vec<LaneSeed>, plan: Plan) -> Result<u8, CoreError> {
    let mut s = Scheduler::start(ctx, lanes, plan)?;
    let end = s.drive();
    s.finish(end)
}

// ---------------------------------------------------------- the scheduler

impl Scheduler {
    fn start(ctx: Ctx, seeds: Vec<LaneSeed>, plan: Plan) -> Result<Scheduler, CoreError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source: std::io::Error| CoreError::Io { path, source }
        };
        ctx.layout.ensure_scratch()?;
        std::fs::create_dir_all(&ctx.logs_dir).map_err(io(&ctx.logs_dir))?;
        let log = EventLog::open(&ctx.logs_dir);
        let jobs_dir = ctx.layout.scratch.join("jobs");
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
        let mut lanes = Vec::with_capacity(seeds.len());
        for (id, seed) in seeds.into_iter().enumerate() {
            lanes.push(Lane::start(
                id,
                seed,
                &ctx,
                events_tx.clone(),
                log.clone(),
                &params,
            )?);
        }

        Ok(Scheduler {
            started_at: UtcTime::now().format(),
            state: RunState::default(),
            events_tx,
            events,
            params,
            job_lock: Some(job_lock),
            lanes,
            worker_n: 0,
            adopt_n: 0,
            last_derive: Instant::now(),
            dirty: true,
            abort: None,
            failure: None,
            races_lost: 0,
            churned: false,
            log,
            repo: None,
            diffs: DiffCache::default(),
            rederived: 0,
            plan,
            critical_ran: Vec::new(),
            weights: ctx
                .cfg
                .run
                .priority_by_history()
                .then(|| super::history::weights(&ctx.layout.scratch)),
            last_pass: None,
            ctx,
        })
    }

    /// What the run builds, as the run record names it: the lanes' SDDs.
    fn sdd(&self) -> String {
        self.lanes
            .iter()
            .map(Lane::sdd)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn record(&self, terminated: Option<(&str, &str)>) {
        if let Err(e) = write_run_json(
            &self.ctx,
            &self.sdd(),
            &self.state,
            &self.started_at,
            terminated,
        ) {
            diag(&format!("warning: run.json not written: {e}"));
        }
    }

    /// Write `project.json`: the plan, each lane's state as it is now, and
    /// the critical path as it ran. `ended`: the run is over.
    fn record_lanes(&mut self, ended: bool) {
        for lane in &self.lanes {
            let slug = lane.graph_slug();
            if let Some(p) = self.plan.lanes.iter_mut().find(|p| p.slug == slug) {
                p.state = lane.state;
                // the counts as they stand now; what the run set out to
                // regenerate, adopt and re-check stays the plan's
                if !lane.views.is_empty() {
                    let live = lane.views.iter().filter(|v| v.status != "Retired");
                    p.units = live.clone().count();
                    p.verified = live.filter(|v| v.status == "Verified").count();
                }
            }
        }
        let record = ProjectRecord {
            started_at: self.started_at.clone(),
            ended_at: ended.then(|| UtcTime::now().format()),
            plan: self.plan.clone(),
            critical_path_ran: self.critical_ran.clone(),
        };
        if let Err(e) = record.write(&self.ctx.logs_dir) {
            diag(&format!("warning: project.json not written: {e}"));
        }
    }

    /// Bring each lane's state up to date — what it would be doing now —
    /// and rewrite `project.json` when one changed.
    fn update_states(&mut self, ready: &BTreeSet<LaneId>, due: &BTreeSet<LaneId>) {
        let mut changed = false;
        for (l, lane) in self.lanes.iter_mut().enumerate() {
            let state = if !lane.blocked.is_empty() {
                LaneState::Blocked
            } else if lane.rechecking.is_some() {
                LaneState::Recheck
            } else if !lane.active.is_empty() || ready.contains(&l) {
                LaneState::Build
            } else if due.contains(&l) {
                LaneState::Recheck
            } else if lane
                .views
                .iter()
                .any(|v| !matches!(v.status.as_str(), "Verified" | "Retired"))
            {
                LaneState::Idle
            } else {
                LaneState::UpToDate
            };
            if state != lane.state {
                lane.state = state;
                changed = true;
            }
        }
        if changed {
            self.record_lanes(false);
        }
    }

    /// A line of lane `l`'s for stdout: in a project run it names the lane.
    fn say(&self, l: LaneId, line: &str) -> String {
        self.lanes[l].say(line)
    }

    /// Log a `waiting: …` line the run printed while it waits under `--wait`.
    fn waiting(&self, line: &str) {
        self.event(RunEvent::new(UtcTime::now(), "waiting", None, None, line));
    }

    /// Append one event to the run's event log.
    fn event(&self, e: RunEvent) {
        self.log.write(&e);
    }

    /// Append one event of lane `l` to the run's event log, naming the lane.
    fn lane_event(&self, l: LaneId, e: RunEvent) {
        self.log
            .write(&e.with("lane", J::s(self.lanes[l].ml.slug.clone())));
    }

    /// Every lane's rotten units, as the unit graph names them.
    fn rotten_nodes(&self) -> BTreeSet<Node> {
        self.lanes
            .iter()
            .flat_map(|l| l.rotten.keys().map(|u| l.node(u)))
            .collect()
    }

    /// The SDDs this run builds, by the slugs the unit graph names them by.
    fn lane_slugs(&self) -> Vec<String> {
        self.lanes.iter().map(Lane::graph_slug).collect()
    }

    fn limit(&self) -> u32 {
        self.ctx.cfg.run.abort_failed_waves.max(1) as u32
    }

    fn lease_minutes(&self) -> i64 {
        self.ctx.cfg.run.lease_minutes
    }

    /// Workers running now, across lanes: what the cap bounds.
    fn working(&self) -> usize {
        self.lanes.iter().map(Lane::working).sum()
    }

    // ------------------------------------------------------------ loop

    fn drive(&mut self) -> Result<End, CoreError> {
        // the run's own record exists before anything is dispatched
        self.record(None);
        self.record_lanes(false);
        let sdd = self.sdd();
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
        // With no lane cap, every lane's dynamic preflight runs before the
        // first pass, all at once, each on its own integrator's thread; under
        // a cap, a lane's runs when it is admitted.
        if self.ctx.project && self.ctx.cfg.run.lane_limit().is_none() {
            for l in 0..self.lanes.len() {
                if self.lanes[l].blocked.is_empty() {
                    self.ready_for_work(l, &[]);
                }
            }
            while self.lanes.iter().any(|l| l.checkout == Checkout::Preparing)
                && signals::pending().is_none()
            {
                match self.events.recv_timeout(TICK) {
                    Ok(event) => self.handle(event)?,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        }
        // what the preflight re-check found rotten
        for l in 0..self.lanes.len() {
            let rows = std::mem::take(&mut self.lanes[l].preflight_rotten);
            if !rows.is_empty() {
                self.on_rotten(l, rows);
            }
        }

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
            self.integrators_alive()?;
            let poll = Duration::from_secs(self.ctx.cfg.run.poll_seconds.max(1) as u64);
            if self.dirty || self.last_derive.elapsed() >= poll {
                if !self.derive()? {
                    // mainline could not be brought up to date this time:
                    // nothing is decided on a view that may be stale
                    std::thread::sleep(Duration::from_millis(300));
                    self.dirty = true;
                    continue;
                }
                // Whatever a lane's threads reported before the head moved —
                // a rotten verdict is sent before its batch is published —
                // is taken in before anything is decided against it; and
                // what was taken in may have moved the head again, so the
                // view is derived anew before deciding.
                let mut drained = false;
                while let Ok(event) = self.events.try_recv() {
                    self.handle(event)?;
                    drained = true;
                }
                // (twice at most: lanes publishing back to back would
                // otherwise keep the pass from ever deciding)
                if drained && self.rederived < 2 && self.head_moved() {
                    self.rederived += 1;
                    self.dirty = true;
                    continue;
                }
                self.rederived = 0;
                self.churned = false;
                let dispatched = self.fill_slots()? + self.start_rechecks();
                self.dirty = false;

                // a culprit search still running has a finding to report
                let idle = self.lanes.iter().all(|l| {
                    l.active.is_empty()
                        && l.searches == 0
                        && l.rechecking.is_none()
                        && !matches!(l.checkout, Checkout::Preparing | Checkout::Releasing)
                });
                if idle && dispatched == 0 && self.abort.is_none() {
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

    /// Derive every lane's view from the mainline head ([`Lane::derive`]).
    /// `false` = mainline was too busy to be put right for one of them.
    fn derive(&mut self) -> Result<bool, CoreError> {
        for l in 0..self.lanes.len() {
            if l > 0 {
                let (read, rest) = self.lanes.split_at_mut(l);
                rest[0].share_graph(&read[0]);
            }
            let lane = &mut self.lanes[l];
            if !lane.derive()? {
                return Ok(false);
            }
        }
        self.last_derive = Instant::now();
        self.read_repo();
        self.join_new_lanes()?;
        Ok(true)
    }

    /// Whether mainline moved past the head the lanes were derived at.
    fn head_moved(&self) -> bool {
        let Some(lane) = self.lanes.first() else {
            return false;
        };
        lane.ml.head().map_or(true, |h| h != lane.head)
    }

    /// In a project run over every Active SDD, an SDD that became Active on
    /// mainline mid-run joins as a lane, after its own preflight.
    fn join_new_lanes(&mut self) -> Result<(), CoreError> {
        if !self.ctx.project || self.ctx.upstream {
            return Ok(());
        }
        let Some(repo) = &self.repo else {
            return Ok(());
        };
        let joining: Vec<String> = repo
            .snapshot
            .sdds
            .iter()
            .filter(|s| s.inst.meta.get_str_or("status", "Draft") == "Active")
            .filter(|s| !self.lanes.iter().any(|l| l.graph_slug() == s.slug))
            .map(|s| s.root.clone())
            .collect();
        for root in joining {
            let rel = std::path::PathBuf::from(&root);
            // the primary checkout follows mainline: its copy is the one
            // preflight judges, as at the start
            let Ok(inst) =
                crate::cmd::resolve_sdd_in(&self.ctx.primary.dir, &rel).and_then(Instance::load)
            else {
                continue;
            };
            let slug = crate::layout::sdd_slug(&inst);
            let mut seed = LaneSeed {
                ml: self.ctx.mainline_for(rel, slug.clone()),
                inst,
                cap: usize::MAX,
                rotten: Vec::new(),
                blocked: Vec::new(),
                checked: Vec::new(),
                suite_ran: false,
            };
            if seed.inst.shape == vitrify_core::model::Shape::Single {
                seed.cap = 1;
            }
            // its static preflight here; its dynamic one, like every lane's,
            // on its integrator's thread when it is admitted
            let mut own: Vec<String> = Vec::new();
            if !super::lane_static(&self.ctx, &seed, &mut own, false)? {
                return Ok(());
            }
            for f in &own {
                diag(&event_line(&slug, &format!("preflight failed: {f}")));
            }
            seed.blocked = own;
            let id = self.lanes.len();
            let lane = Lane::start(
                id,
                seed,
                &self.ctx,
                self.events_tx.clone(),
                self.log.clone(),
                &self.params,
            )?;
            emit(&lane.say("joins the run: its SDD is Active"));
            self.plan.lanes.push(project::Lane {
                slug: lane.graph_slug(),
                root: lane.sdd(),
                records: format!(".vitrify/logs/{}/{}/", self.ctx.job_id, lane.ml.slug),
                state: lane.state,
                units: 0,
                verified: 0,
                regenerate: 0,
                adopt: 0,
                recheck: 0,
            });
            self.lanes.push(lane);
            self.lanes[id].derive()?;
            self.recover_lane(id);
            self.record_lanes(false);
            self.record(None);
        }
        Ok(())
    }

    /// Read the whole repository at the head the first lane derived from —
    /// once per head: its graph and closures, which Verified units a change
    /// reached since their last recorded check, and the priorities.
    fn read_repo(&mut self) {
        let Some(first) = self.lanes.first() else {
            return;
        };
        let head = first.head.clone();
        let git = first.ml.primary.at(&first.head_wt);
        // (Every head is read again: a commit of protocol state alone — a
        // re-check's record, an integration's journal — moves the anchors
        // the dirty set is computed from.)
        if self.repo.as_ref().is_some_and(|r| r.head == head) {
            return;
        }
        // an unreadable SDD is left out, and so are the SDDs on a cycle:
        // the rest still cascade, re-check and rank
        let (snapshot, cycles) = match Snapshot::load_resolvable(&git, &first.head_wt, &head) {
            Ok(s) => s,
            Err(e) => {
                diag(&format!(
                    "note: the repository could not be read whole ({e})"
                ));
                self.repo = None;
                return;
            }
        };
        let dirty = match snapshot.dirty_with(&git, Mode::Changes, &mut self.diffs) {
            Ok(Cascade::Resolved(set)) => Some(set),
            _ => None,
        };
        let priorities = snapshot
            .resolved
            .as_ref()
            .ok()
            .map(|r| plan::priorities(&snapshot.graph, r, self.weights.as_ref()));
        let known = self.repo.as_ref().and_then(|r| r.cycles.as_ref());
        if let Some(c) = &cycles
            && known.is_none_or(|k| k.cycles != c.cycles)
        {
            for path in &c.cycles {
                diag(&format!(
                    "a dependency cycle on mainline: {} — nothing on it or behind it is dispatched",
                    unit_graph::render_path(path)
                ));
            }
        }
        self.repo = Some(Repo {
            head,
            snapshot,
            dirty,
            priorities,
            cycles,
        });
    }

    /// The units Verified at the repository's head: finished, for what
    /// depends on them.
    fn finished(&self, n: &Node) -> bool {
        self.repo
            .as_ref()
            .is_some_and(|r| plan::finished(&r.snapshot.graph, n))
    }

    /// Whether lane `l` can take work now: admitted under `run.lane_cap`
    /// and its checkout ready. A lane admitted for the first time has its
    /// dynamic preflight started on its integrator's thread — it takes work
    /// once that passes; one admitted again has its checkout made at its
    /// first use.
    fn ready_for_work(&mut self, l: LaneId, picked: &[(LaneId, String, bool)]) -> bool {
        if !self.admitted_with(l, picked) {
            return false;
        }
        let lane = &mut self.lanes[l];
        match lane.checkout {
            Checkout::Ready => true,
            Checkout::Preparing | Checkout::Releasing => false,
            Checkout::Absent if lane.prepared => {
                lane.checkout = Checkout::Ready;
                true
            }
            Checkout::Absent => {
                lane.checkout = Checkout::Preparing;
                let _ = lane.jobs.send(Job::Prepare);
                false
            }
        }
    }

    /// With `run.lane_cap`, a lane with nothing to do — no claim, no
    /// re-check, no culprit search, nothing ready or due — gives its
    /// checkouts back, so the next lane by priority can be admitted.
    fn release_idle(&mut self, ready: &BTreeSet<LaneId>, due: &BTreeSet<LaneId>) {
        if self.ctx.cfg.run.lane_limit().is_none() || !self.ctx.project {
            return;
        }
        for l in 0..self.lanes.len() {
            let lane = &mut self.lanes[l];
            if lane.checkout == Checkout::Ready
                && lane.active.is_empty()
                && lane.rechecking.is_none()
                && lane.searches == 0
                && !ready.contains(&l)
                && !due.contains(&l)
            {
                lane.checkout = Checkout::Releasing;
                let _ = lane.jobs.send(Job::Release);
            }
        }
    }

    /// Whether lane `l` may start more work under `run.lane_cap`, counting
    /// as active too the lanes of `picked`, units this pass has picked and
    /// not yet claimed.
    fn admitted_with(&self, l: LaneId, picked: &[(LaneId, String, bool)]) -> bool {
        // a lane counts while it holds checkouts, or has work in flight
        let active = |x: LaneId| {
            let lane = &self.lanes[x];
            lane.checkout != Checkout::Absent
                || !lane.active.is_empty()
                || lane.rechecking.is_some()
                || picked.iter().any(|p| p.0 == x)
        };
        if active(l) {
            return true;
        }
        let n = (0..self.lanes.len()).filter(|&x| active(x)).count();
        self.ctx.cfg.run.lane_limit().is_none_or(|cap| n < cap)
    }

    /// Lane `l`'s dirty Verified units due a re-check: a change reached them
    /// since their last recorded check, and nothing they depend on has work
    /// pending — unfinished, in flight, dirty or rotten. Re-checks follow
    /// dependency order this way: upstream first.
    fn due_rechecks(&self, l: LaneId) -> Vec<String> {
        let lane = &self.lanes[l];
        let Some(repo) = &self.repo else {
            return Vec::new();
        };
        let Some(dirty) = &repo.dirty else {
            return Vec::new();
        };
        if lane.head != repo.head || !lane.blocked.is_empty() {
            return Vec::new();
        }
        let rotten = self.rotten_nodes();
        let in_flight: BTreeSet<Node> = self
            .lanes
            .iter()
            .flat_map(|x| x.active.keys().map(|u| x.node(u)))
            .collect();
        let slug = lane.graph_slug();
        // dirty, or — under --recheck — Verified and not checked this run
        let mut wanted: Vec<String> = dirty.of_sdd(&slug).map(|d| d.node.unit.clone()).collect();
        if self.ctx.recheck {
            for v in lane.views.iter().filter(|v| v.status == "Verified") {
                if !lane.checked.contains(&v.unit) && !wanted.contains(&v.unit) {
                    wanted.push(v.unit.clone());
                }
            }
            wanted.sort();
        }
        wanted
            .into_iter()
            .filter(|unit| {
                !lane.rotten.contains_key(unit)
                    && !lane.active.contains_key(unit)
                    && lane.rechecked_at.get(unit) != Some(&lane.head)
                    // what it depends on in another SDD is settled first;
                    // what it depends on in its own is re-checked with it
                    && !lane.closure(unit).iter().any(|n| {
                        !self.finished(n)
                            || in_flight.contains(n)
                            || rotten.contains(n)
                            || (n.slug != slug && self.needs_check(n, dirty))
                    })
            })
            .collect()
    }

    /// Whether `n` must be checked again before what depends on it is: a
    /// change reached it, or — under `--recheck` — it is a lane's Verified
    /// unit this run has not checked yet.
    fn needs_check(&self, n: &Node, dirty: &DirtySet) -> bool {
        if dirty.is_dirty(n) {
            return true;
        }
        self.ctx.recheck
            && self.lanes.iter().any(|l| {
                l.graph_slug() == n.slug
                    && !l.checked.contains(&n.unit)
                    && l.views
                        .iter()
                        .any(|v| v.unit == n.unit && v.status == "Verified")
            })
    }

    /// Under `--recheck`, whether lane `l`'s §5 Test command is still to run
    /// this run while nothing else will run it: every unit of the lane is
    /// built, and no unit of it is due a re-check.
    fn suite_due(&self, l: LaneId) -> bool {
        let lane = &self.lanes[l];
        let wanted = |v: &&vitrify_core::model::UnitView| {
            v.status == "Verified"
                && self
                    .repo
                    .as_ref()
                    .and_then(|r| r.dirty.as_ref())
                    .is_some_and(|d| self.needs_check(&lane.node(&v.unit), d))
        };
        self.ctx.recheck
            && !lane.suite_ran
            && lane.blocked.is_empty()
            && lane.active.is_empty()
            && lane.rechecked_at.get("") != Some(&lane.head)
            && !lane.views.is_empty()
            && lane
                .views
                .iter()
                .all(|v| matches!(v.status.as_str(), "Verified" | "Retired"))
            // a batch of its units would run the suite anyway
            && !lane.views.iter().any(|v| wanted(&v))
    }

    /// Start a re-check batch in every lane with units due one and nothing
    /// of its own at the gate (a gate re-checks what is dirty anyway).
    /// Returns how many batches started.
    fn start_rechecks(&mut self) -> usize {
        let mut started = 0;
        for l in 0..self.lanes.len() {
            let lane = &self.lanes[l];
            if lane.rechecking.is_some()
                || lane.active.values().any(|a| a.phase == Phase::Integrating)
                || signals::interrupted()
                || self.abort.is_some()
            {
                continue;
            }
            let due = self.due_rechecks(l);
            if (due.is_empty() && !self.suite_due(l)) || !self.ready_for_work(l, &[]) {
                continue;
            }
            let line = if due.is_empty() {
                "re-check the suite".to_string()
            } else {
                format!("re-check {}", due.join(", "))
            };
            emit(&self.say(l, &line));
            self.lane_event(
                l,
                RunEvent::new(UtcTime::now(), "recheck", None, None, line).with(
                    "units",
                    J::Arr(due.iter().map(|u| J::s(u.clone())).collect()),
                ),
            );
            let lane = &mut self.lanes[l];
            lane.rechecking = Some(due.clone());
            let _ = lane.jobs.send(Job::Recheck(due));
            started += 1;
        }
        started
    }

    /// Lane `l`'s dynamic preflight ended: it takes work now, or it is
    /// blocked — its units, and every unit that depends on them, wait.
    fn on_prepared(
        &mut self,
        l: LaneId,
        result: Result<Option<Vec<String>>, CoreError>,
    ) -> Result<(), CoreError> {
        let Some(failures) = result? else {
            // interrupted: the run is ending
            self.lanes[l].checkout = Checkout::Absent;
            return Ok(());
        };
        self.dirty = true;
        let lane = &mut self.lanes[l];
        lane.prepared = true;
        lane.checkout = Checkout::Ready;
        if !failures.is_empty() {
            for f in &failures {
                diag(&lane.say(&format!("preflight failed: {f}")));
            }
            lane.blocked = failures;
            lane.state = LaneState::Blocked;
        }
        Ok(())
    }

    /// A re-check batch of lane `l` ended: its passes are recorded, its
    /// failures are rotten.
    fn on_rechecked(&mut self, l: LaneId, result: Result<Rechecked, CoreError>) {
        let units = self.lanes[l].rechecking.take().unwrap_or_default();
        let head = self.lanes[l].head.clone();
        // ("" stands for the suite: a batch of no unit is the suite's)
        if units.is_empty() {
            self.lanes[l]
                .rechecked_at
                .insert(String::new(), head.clone());
        }
        for u in &units {
            self.lanes[l].rechecked_at.insert(u.clone(), head.clone());
        }
        let r = match result {
            Ok(r) => r,
            Err(e) => {
                self.failure = Some(e.to_string());
                return;
            }
        };
        if r.interrupted {
            return;
        }
        let lane = &mut self.lanes[l];
        lane.suite_ran |= r.suite_ran;
        if r.broken.is_none() {
            lane.checked.extend(units.iter().cloned());
        }
        if let Some((cmd, output)) = &r.broken {
            diag(&self.say(
                l,
                &format!(
                    "re-check could not run: `{cmd}` failed: {}",
                    tail(output, 3)
                ),
            ));
            return;
        }
        if !r.passed.is_empty() {
            self.lanes[l].rechecked += r.passed.len();
            let line = format!("re-checked {} at {}", r.passed.join(", "), short(&r.commit));
            emit(&self.say(l, &line));
            self.lane_event(
                l,
                RunEvent::new(UtcTime::now(), "rechecked", None, None, line)
                    .with("recorded", J::Bool(r.recorded)),
            );
        }
        if !r.rotten.is_empty() {
            self.on_rotten(l, r.rotten);
        }
    }

    fn integrators_alive(&self) -> Result<(), CoreError> {
        self.lanes.iter().try_for_each(Lane::integrator_alive)
    }

    /// Start a worker for each claimable unit while slots are free.
    /// A built attempt of a claimable unit goes to the gate with no worker
    /// and takes no slot; every other claimable unit gets a worker while
    /// slots are free. A unit that depends on a rotten one, in any SDD, is
    /// not built while it fails.
    fn fill_slots(&mut self) -> Result<usize, CoreError> {
        let mut dispatched = 0;
        let rotten = self.rotten_nodes();
        let mut candidates: Vec<(LaneId, String, bool, Node)> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.blocked.is_empty())
            .flat_map(|(l, lane)| {
                let rotten = &rotten;
                lane.views
                    .iter()
                    .filter(move |v| {
                        v.claimable
                            && !lane.active.contains_key(&v.unit)
                            && !lane.parked.contains_key(&v.unit)
                            && lane.rotten_dependency(&v.unit, rotten).is_none()
                    })
                    .map(move |v| (l, v.unit.clone(), v.status == "Stale", lane.node(&v.unit)))
            })
            .collect();
        // nothing on a cycle, or behind one, is dispatched
        if let Some(repo) = &self.repo {
            candidates.retain(|c| !repo.cyclic(&c.3));
        }
        // critical path first: priority descending, then slug, then unit id
        match self.repo.as_ref().and_then(|r| r.priorities.as_ref()) {
            Some(p) => candidates.sort_by(|a, b| p.cmp(&a.3, &b.3)),
            None => candidates.sort_by(|a, b| a.3.cmp(&b.3)),
        }
        self.set_lane_priorities();
        let ready: BTreeSet<LaneId> = candidates.iter().map(|c| c.0).collect();
        let mut waiting = 0;
        let mut picks: Vec<(LaneId, String, bool)> = Vec::new();
        for (l, unit, was_stale, _) in candidates {
            if signals::interrupted() || self.abort.is_some() {
                break;
            }
            if !self.ready_for_work(l, &picks) {
                continue;
            }
            if let Some(adopted) = self.adopt(l, &unit)? {
                dispatched += usize::from(adopted);
                continue;
            }
            // the cap bounds running workers; a unit waiting for the gate
            // holds a lease, not a slot
            let picked_in = |l: LaneId| {
                picks
                    .iter()
                    .filter(|p: &&(LaneId, String, bool)| p.0 == l)
                    .count()
            };
            if self.working() + picks.len() >= self.ctx.cap {
                waiting += 1;
                continue;
            }
            if self.lanes[l].working() + picked_in(l) >= self.lanes[l].cap {
                continue;
            }
            picks.push((l, unit, was_stale));
        }
        dispatched += self.dispatch(picks)?;
        let due: BTreeSet<LaneId> = (0..self.lanes.len())
            .filter(|&l| !self.due_rechecks(l).is_empty())
            .collect();
        self.update_states(&ready, &due);
        self.release_idle(&ready, &due);
        self.record_pass(waiting);
        Ok(dispatched)
    }

    /// Record a pass in the event log when it differs from the last: the
    /// workers running, the slots idle, and the ready units left waiting
    /// for a slot. A slot is idle only when nothing ready waits for one.
    fn record_pass(&mut self, waiting: usize) {
        if !self.ctx.project {
            return;
        }
        let working = self.working();
        let idle = self.ctx.cap.saturating_sub(working);
        let pass = (working, idle, waiting);
        if self.last_pass == Some(pass) {
            return;
        }
        self.last_pass = Some(pass);
        self.event(
            RunEvent::new(
                UtcTime::now(),
                "pass",
                None,
                None,
                format!("{working} working · {idle} idle · {waiting} waiting"),
            )
            .with("cap", J::Int(self.ctx.cap as i64))
            .with("idle", J::Int(idle as i64))
            .with("waiting", J::Int(waiting as i64))
            .with("working", J::Int(working as i64)),
        );
    }

    /// Each lane's gate commands wait for the gate pool at the priority of
    /// its highest-priority unfinished unit.
    fn set_lane_priorities(&self) {
        let Some(p) = self.repo.as_ref().and_then(|r| r.priorities.as_ref()) else {
            return;
        };
        for lane in &self.lanes {
            let best = lane
                .views
                .iter()
                .filter_map(|v| p.of_weighted(&lane.node(&v.unit)))
                .max()
                .unwrap_or(0);
            lane.priority
                .store(i64::try_from(best).unwrap_or(i64::MAX), Ordering::Relaxed);
        }
    }

    /// Adopt `unit`'s built attempt, if it has one that can go to the gate
    /// as it is: claim the unit, make the attempt the claim's branch, and
    /// queue it for the gate. `None` = nothing to adopt (the unit goes to a
    /// worker); `Some(true)` = adopted; `Some(false)` = an adoptable attempt
    /// that did not get through (the claim was lost, or given back).
    fn adopt(&mut self, l: LaneId, unit: &str) -> Result<Option<bool>, CoreError> {
        let lane = &self.lanes[l];
        let Some(inst) = &lane.inst else {
            return Ok(None);
        };
        if lane.not_adoptable.contains(unit) {
            return Ok(None);
        }
        let attempt = match adopt::adoptable(&lane.ml.primary, inst, &lane.head_wt, unit) {
            // Only an attempt a run set aside — the run that made it, as it
            // ended, or the next one, recovering a run that died — is known
            // to have no worker left. One whose lease merely lapsed may
            // still have its worker writing: it goes to a worker of its own.
            Ok(Ok(attempt)) if set_aside(&lane.ml.primary, &attempt.commit) => attempt,
            other => {
                if let Err(e) = other {
                    diag(&format!(
                        "note: {unit}: its last attempt could not be looked at ({e}); it goes to a worker"
                    ));
                }
                self.lanes[l].not_adoptable.insert(unit.to_string());
                return Ok(None);
            }
        };
        self.adopt_n += 1;
        let holder = format!("{}-a{}", self.ctx.job_id, self.adopt_n);
        let lease = self.lease_minutes();
        let outcome = match lane.ml.claim(Op::Claim, unit, &holder, lease) {
            Ok(o) => o,
            Err(e) if matches!(e.exit_code(), 4 | 5) => {
                diag(&format!("claim lost for {unit}: {e}"));
                if e.exit_code() == 5 {
                    self.races_lost += 1;
                }
                return Ok(Some(false));
            }
            Err(e) => return Err(e),
        };
        self.races_lost = 0;
        let seq = outcome.claim_seq;
        let slug = lane.ml.slug.clone();
        let wt = lane.ml.layout.unit_worktree(&slug, unit, seq);
        let branch =
            match adopt::relabel(&lane.ml.primary, &slug, &attempt, seq, &outcome.commit, &wt) {
                Ok(branch) => branch,
                Err(e) => {
                    let why = e.to_string();
                    let why = why.strip_prefix("error: ").unwrap_or(&why).to_string();
                    diag(&format!("{unit} (c{seq}): {why}"));
                    self.lanes[l].release(unit, &holder, lease);
                    self.lanes[l]
                        .attempts
                        .push((unit.to_string(), "not-adopted".to_string(), why));
                    self.lanes[l].not_adoptable.insert(unit.to_string());
                    self.count_failure(l, unit, "not-adopted", false)?;
                    self.churned = true;
                    return Ok(Some(false));
                }
            };
        let expires = outcome
            .claim_expires
            .as_deref()
            .and_then(UtcTime::parse)
            .unwrap_or_else(|| UtcTime::now().add_minutes(lease));
        let started = UtcTime::now();
        self.state.workers.push(WorkerRec {
            lane: slug.clone(),
            holder: holder.clone(),
            pid: 0,
            unit: unit.to_string(),
            claim_seq: seq,
            branch: branch.clone(),
            worktree: wt.display().to_string(),
            started_at: started.format(),
            ended_at: None,
            outcome: "integrating".to_string(),
            retry: false,
            detail: None,
        });
        let rec = self.state.workers.len() - 1;
        let lane = &mut self.lanes[l];
        lane.keeper.hold(unit, &holder, expires);
        lane.active.insert(
            unit.to_string(),
            Active {
                holder: holder.clone(),
                seq,
                branch,
                wt,
                pid: None,
                out_path: PathBuf::new(),
                _tmp: None,
                trace: attempt.trace.clone(),
                phase: Phase::Integrating,
                rec,
                killed: None,
            },
        );
        emit(&adopted_line(&slug, unit, attempt.seq));
        self.lane_event(
            l,
            RunEvent::new(
                started,
                "adopted",
                Some(unit),
                Some(seq),
                format!("built attempt c{} → gate", attempt.seq),
            )
            .with("attempt", J::Int(attempt.seq))
            .with("holder", J::s(holder.clone())),
        );
        self.record(None);
        let _ = self.lanes[l].jobs.send(Job::Integrate(IntegrateRequest {
            unit: unit.to_string(),
            token_seq: seq,
            builder_holder: holder,
            integrator_id: format!("{}-integrator", self.ctx.job_id),
        }));
        Ok(Some(true))
    }

    /// Set the attempt of `unit`'s claim aside on its branch, with why: its
    /// worktree's files committed and the worktree removed. An adopted
    /// attempt has no worktree; its branch gets the note alone, so a later
    /// run sees the verdict and does not adopt it again.
    fn abandon(&self, l: LaneId, unit: &str, label: &str, detail: &str) {
        let lane = &self.lanes[l];
        let Some(a) = lane.active.get(unit) else {
            return;
        };
        let primary = &lane.ml.primary;
        if a.pid.is_none()
            && !a.wt.exists()
            && primary.branch_exists(&a.branch).unwrap_or(false)
            && primary.worktree_add(&a.wt, &a.branch, false).is_err()
        {
            return;
        }
        retry::abandon(primary, &a.wt, unit, a.seq, label, detail);
    }

    // -------------------------------------------------------- dispatch

    /// Claim `unit` of lane `l`, give it a worktree, start its worker.
    fn dispatch(&mut self, picks: Vec<(LaneId, String, bool)>) -> Result<usize, CoreError> {
        if picks.is_empty() {
            return Ok(0);
        }
        // every unit this pass picked, claimed in one mainline transaction:
        // one commit for them all, each with its own token and event
        let lease = self.lease_minutes();
        let mut holders: Vec<(u32, String)> = Vec::new();
        let mut batch: Vec<claim::BatchClaim> = Vec::new();
        for (l, unit, _) in &picks {
            self.worker_n += 1;
            let holder = format!("{}-w{}", self.ctx.job_id, self.worker_n);
            holders.push((self.worker_n, holder.clone()));
            batch.push(claim::BatchClaim {
                sdd_rel: self.lanes[*l].ml.sdd_rel.clone(),
                unit: unit.clone(),
                holder,
            });
        }
        let results = claim::claim_many(
            &self.ctx.primary,
            self.ctx.push,
            &self.ctx.branch,
            lease,
            &batch,
        )?;
        // a pass that lost the race for mainline counts once, however many
        // units it had picked; one that claimed anything resets the count
        let busy = results
            .iter()
            .any(|r| r.as_ref().is_err_and(|e| e.exit_code() == 5));
        let claimed = results.iter().any(Result::is_ok);
        if claimed {
            self.races_lost = 0;
        } else if busy {
            self.races_lost += 1; // mainline was busy, not the units taken
        }
        let mut started = 0;
        for (((l, unit, was_stale), (n, holder)), result) in
            picks.into_iter().zip(holders).zip(results)
        {
            let outcome = match result {
                Ok(o) => o,
                Err(e) if matches!(e.exit_code(), 4 | 5) => {
                    diag(&format!("claim lost for {unit}: {e}"));
                    continue;
                }
                Err(e) => return Err(e),
            };
            if self.start_claimed(l, &unit, was_stale, n, holder, outcome)? {
                started += 1;
            }
        }
        Ok(started)
    }

    /// Start the worker of a unit this run has just claimed: its worktree,
    /// what the last attempt left, the prompt, the process.
    fn start_claimed(
        &mut self,
        l: LaneId,
        unit: &str,
        was_stale: bool,
        n: u32,
        holder: String,
        outcome: claim::ClaimOutcome,
    ) -> Result<bool, CoreError> {
        let lease = self.lease_minutes();
        let lane = &self.lanes[l];
        let seq = outcome.claim_seq;
        let slug = lane.ml.slug.clone();
        let branch = layout::unit_branch(&slug, unit, seq);
        let wt = lane.ml.layout.unit_worktree(&slug, unit, seq);
        let primary = lane.ml.primary.clone();

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
            self.lanes[l].release(unit, &holder, lease);
            self.lanes[l]
                .attempts
                .push((unit.to_string(), "name-taken".to_string(), why));
            self.count_failure(l, unit, "name-taken", false)?;
            // mainline has moved (a claim and its release): what this pass
            // derived is no longer the state to conclude on
            self.churned = true;
            return Ok(false);
        }

        let expires = outcome
            .claim_expires
            .as_deref()
            .and_then(UtcTime::parse)
            .unwrap_or_else(|| UtcTime::now().add_minutes(lease));
        match self.launch(
            l,
            unit,
            &holder,
            seq,
            &branch,
            &wt,
            &outcome.commit,
            was_stale,
        ) {
            Ok((handle, tmp, trace, retried)) => {
                let pid = handle.pid;
                let out_path = handle.out_path.clone();
                let started = UtcTime::now();
                self.state.workers.push(WorkerRec {
                    lane: slug,
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
                self.monitor(l, handle, unit, &holder, expires);
                self.lanes[l].active.insert(
                    unit.to_string(),
                    Active {
                        holder: holder.clone(),
                        seq,
                        branch,
                        wt,
                        pid: Some(pid),
                        out_path,
                        _tmp: Some(tmp),
                        trace,
                        phase: Phase::Working,
                        rec,
                        killed: None,
                    },
                );
                emit(&self.say(
                    l,
                    &format!(
                        "dispatch {unit} (c{seq}) → w{}{}",
                        n,
                        if retried { "  [retry]" } else { "" }
                    ),
                ));
                self.lane_event(
                    l,
                    RunEvent::new(
                        started,
                        "dispatched",
                        Some(unit),
                        Some(seq),
                        format!("→ w{}{}", n, if retried { " (retry)" } else { "" }),
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
                self.lanes[l].release(unit, &holder, lease);
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
                self.lanes[l].attempts.push((
                    unit.to_string(),
                    "not-started".to_string(),
                    why.clone(),
                ));
                self.lanes[l].park(unit, format!("its reading set does not resolve: {why}"))?;
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
        l: LaneId,
        unit: &str,
        holder: &str,
        seq: i64,
        branch: &str,
        wt: &Path,
        claim_commit: &str,
        was_stale: bool,
    ) -> Result<(worker::WorkerHandle, gate::TmpSlot, TraceMap, bool), CoreError> {
        let lane = &self.lanes[l];
        let ml = &lane.ml;
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
        let sdd = lane.sdd();
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
            lane.logs_dir.join(format!("worker-{holder}.prompt.md")),
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
            &lane.logs_dir,
            holder,
            unit,
            Some(&tmp.dir),
        )?;
        Ok((handle, tmp, trace_at_claim, prev.is_some()))
    }

    /// One thread per worker: renew its lease, enforce the hard budget,
    /// report its exit.
    fn monitor(
        &self,
        l: LaneId,
        handle: worker::WorkerHandle,
        unit: &str,
        holder: &str,
        expires: UtcTime,
    ) {
        let tx = self.events_tx.clone();
        let params = self.params.clone();
        let backend = RealLease {
            ml: self.lanes[l].ml.clone(),
            lease_minutes: self.lease_minutes(),
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
                lane: l,
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
                lane,
                unit,
                verdict,
                expires,
            } => self.on_exit(lane, &unit, verdict, expires),
            Event::Integrated {
                lane,
                unit,
                outcome,
            } => self.on_integrated(lane, &unit, outcome),
            Event::IntegratorFailed { lane, error } => {
                // the run ends in an error; the lane did not finish
                self.lanes[lane].state = LaneState::Blocked;
                self.failure = Some(error);
                Ok(())
            }
            // only `conclude` asks for one, and it takes the answer itself
            Event::HeadChecked { .. } => Ok(()),
            Event::Rotten { lane, rows } => {
                self.on_rotten(lane, rows);
                Ok(())
            }
            Event::Rechecked { lane, result } => {
                self.on_rechecked(lane, *result);
                Ok(())
            }
            Event::Prepared { lane, result } => self.on_prepared(lane, *result),
            Event::Released { lane } => {
                self.lanes[lane].checkout = Checkout::Absent;
                self.dirty = true;
                Ok(())
            }
            Event::Culprit { lane, unit, report } => {
                self.on_culprit(lane, &unit, report);
                Ok(())
            }
        }
    }

    /// Rows of lane `l` that fail on their gate's base too: their units are
    /// rotten. Each row's culprit is searched for on a thread of its own.
    fn on_rotten(&mut self, l: LaneId, rows: Vec<RottenRow>) {
        let slug = self.lanes[l].ml.slug.clone();
        for r in rows {
            let entry = self.lanes[l].rotten.entry(r.unit.clone()).or_default();
            entry
                .rows
                .push(format!("row {}: `{}`", r.row_number, r.row));
            // the event carries the unit; the line on stdout names it
            let detail = format!(
                "row {} fails on mainline at {} — finding the culprit",
                r.row_number,
                short(&r.base)
            );
            emit(&event_line(&slug, &format!("{} rotten: {detail}", r.unit)));
            self.lane_event(
                l,
                RunEvent::new(UtcTime::now(), "rotten", Some(&r.unit), None, detail)
                    .with("row", J::s(r.row.clone()))
                    .with("base", J::s(r.base.clone())),
            );
            self.search_culprit(l, r);
        }
        self.record(None);
    }

    /// Start the culprit search for one rotten row: in the lane's probe
    /// checkout (one search at a time per lane), its commands under the
    /// gate pool, off the critical path.
    fn search_culprit(&mut self, l: LaneId, r: RottenRow) {
        let lane = &self.lanes[l];
        let node = lane.node(&r.unit);
        let found = (|| -> Result<_, CoreError> {
            let inst = lane
                .inst
                .as_ref()
                .ok_or_else(|| CoreError::refusal(1, "error: the lane has no view of its SDD"))?;
            let (events, _) = vitrify_core::journal::read(&inst.handle())?;
            let anchor = Anchors::from_events(&events)
                .unit(&r.unit)
                .unwrap_or(&r.base)
                .to_string();
            let ownership = Ownership::build(
                &lane.head_wt,
                lane.sdds.iter().map(|i| (unit_graph::slug_of(i), i)),
            )?;
            let cmds = integrate::sdd_commands(&model::read_text(&inst.file)?);
            let setup: Vec<String> = [cmds.bootstrap, cmds.build].into_iter().flatten().collect();
            Ok((anchor, ownership, setup))
        })();
        let (anchor, ownership, setup) = match found {
            Ok(found) => found,
            Err(e) => {
                let finding = format!("row {}: culprit not searched: {e}", r.row_number);
                if let Some(entry) = self.lanes[l].rotten.get_mut(&r.unit) {
                    entry.findings.push(finding.clone());
                }
                diag(&format!("warning: {node} {finding}"));
                return;
            }
        };
        let closures: Closures = [(node.clone(), lane.closure(&r.unit))].into();
        let git = lane.ml.primary.clone();
        let prepare = PristineCheckout::new(
            git.clone(),
            culprit::probe_checkout(&lane.ml.layout.wt_root, &lane.ml.slug),
        );
        let turn = Arc::clone(&lane.probe_turn);
        let tx = self.events_tx.clone();
        self.lanes[l].searches += 1;
        std::thread::spawn(move || {
            let _turn = turn.lock().unwrap_or_else(|e| e.into_inner());
            let rotten = culprit::Rotten {
                node: &node,
                row_number: r.row_number,
                row: &r.row,
                anchor: &anchor,
                base: &r.base,
            };
            let probe = culprit::Probe {
                git: &git,
                ownership: &ownership,
                closures: &closures,
                setup: &setup,
                prepare: &prepare,
                runner: &ShellRunner,
                config: facf::Config::default(),
            };
            let report = culprit::find(&rotten, &probe).map_err(|e| e.to_string());
            let _ = tx.send(Event::Culprit {
                lane: l,
                unit: r.unit,
                report,
            });
        });
    }

    /// A culprit search of lane `l` ended: its finding goes to the output,
    /// the event log and the final report. A flaky row is not held against
    /// its unit, which is checked again.
    fn on_culprit(&mut self, l: LaneId, unit: &str, report: Result<culprit::Report, String>) {
        let lane = &mut self.lanes[l];
        lane.searches = lane.searches.saturating_sub(1);
        let slug = lane.ml.slug.clone();
        // the finding without the unit: the event and the final report
        // carry the unit already
        let (finding, flaky) = match &report {
            Ok(r) if r.finding == culprit::Finding::Interrupted => return,
            Ok(r) => (
                r.finding_line(),
                matches!(r.finding, culprit::Finding::Flake { .. }),
            ),
            Err(e) => (format!("culprit search failed: {e}"), false),
        };
        if flaky {
            lane.unrot(unit);
        } else if let Some(entry) = lane.rotten.get_mut(unit) {
            entry.findings.push(finding.clone());
        }
        emit(&event_line(&slug, &format!("{unit} {finding}")));
        self.lane_event(
            l,
            RunEvent::new(UtcTime::now(), "culprit", Some(unit), None, finding)
                .with("flaky", J::Bool(flaky)),
        );
    }

    fn on_exit(
        &mut self,
        l: LaneId,
        unit: &str,
        verdict: MonitorVerdict,
        expires: UtcTime,
    ) -> Result<(), CoreError> {
        let Some(a) = self.lanes[l].active.get(unit) else {
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
                    self.ctx.branch
                ),
                _ => "the run ended before the worker finished".to_string(),
            };
            return self.fail(l, unit, why, &detail, Counts::No);
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
                    l,
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
                    l,
                    unit,
                    "lease-lost",
                    "the lease could not be renewed and expired",
                    Counts::Quietly,
                )
            }
            MonitorVerdict::Exited => match report.outcome.as_str() {
                "built" if parsed => {
                    let lane = &mut self.lanes[l];
                    commit_handoff_if_dirty(&lane.ml, &wt, unit, seq);
                    // the lease is the orchestrator's to keep until the
                    // gate has decided
                    lane.keeper.hold(unit, &holder, expires);
                    if let Some(a) = lane.active.get_mut(unit) {
                        a.phase = Phase::Integrating;
                        if let Some(rec) = self.state.workers.get_mut(a.rec) {
                            rec.outcome = "integrating".to_string();
                        }
                    }
                    self.lane_event(
                        l,
                        RunEvent::new(
                            UtcTime::now(),
                            "integrating",
                            Some(unit),
                            Some(seq),
                            report.notes.clone(),
                        ),
                    );
                    self.record(None);
                    let _ = self.lanes[l].jobs.send(Job::Integrate(IntegrateRequest {
                        unit: unit.to_string(),
                        token_seq: seq,
                        builder_holder: holder,
                        integrator_id: format!("{}-integrator", self.ctx.job_id),
                    }));
                    Ok(())
                }
                "blocked" if parsed => {
                    let q = report.q.clone().unwrap_or_else(|| "Q-???".to_string());
                    self.lanes[l].record_question(&q, unit, &holder, &report.notes)?;
                    self.lanes[l].park(unit, format!("open {q}: {}", report.notes))?;
                    self.fail(
                        l,
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
                    self.fail(l, unit, "released", &detail, Counts::Work)
                }
            },
        }
    }

    fn on_integrated(&mut self, l: LaneId, unit: &str, outcome: Outcome) -> Result<(), CoreError> {
        self.lanes[l].keeper.drop_unit(unit);
        let Some(a) = self.lanes[l].active.get(unit) else {
            return Ok(());
        };
        let (seq, branch, holder) = (a.seq, a.branch.clone(), a.holder.clone());
        match outcome {
            Outcome::Verified { .. } => {
                emit(&self.say(l, &format!("integrated {unit} (c{seq})")));
                self.landed(l, unit, holder.contains("-a"));
                self.lanes[l].fails.remove(unit);
                self.lanes[l].attempts.push((
                    unit.to_string(),
                    "verified".to_string(),
                    String::new(),
                ));
                self.close(l, unit, "verified", "");
                Ok(())
            }
            Outcome::RejectedFencing { reason } => {
                diag(&format!("fencing rejection for {unit}: {reason}"));
                self.fail(l, unit, "rejected-fencing", &reason, Counts::Quietly)
            }
            Outcome::SkippedLeaseExpired => {
                diag(&format!(
                    "{unit}: token current but lease expired — back to the pool"
                ));
                self.fail(
                    l,
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
                self.lanes[l].gate_failures.push((
                    unit.to_string(),
                    "write-set check".to_string(),
                    detail.clone(),
                ));
                self.fail(
                    l,
                    unit,
                    "rejected-write-set",
                    &format!("the work changed paths outside the unit's write-set:\n{detail}"),
                    Counts::Work,
                )
            }
            Outcome::RejectedNotBuilt { reason } => {
                diag(&format!("{unit}: {reason}"));
                self.lanes[l].gate_failures.push((
                    unit.to_string(),
                    "Built stamp".to_string(),
                    reason.clone(),
                ));
                self.fail(l, unit, "rejected-not-built", &reason, Counts::Work)
            }
            Outcome::RejectedStaleInputs { keys } => {
                let detail = format!(
                    "built from text that has since changed on {}: {}",
                    self.ctx.branch,
                    keys.join(", ")
                );
                diag(&format!(
                    "{unit}: {detail} — it is rebuilt from the current text"
                ));
                self.fail(l, unit, "stale-inputs", &detail, Counts::No)
            }
            Outcome::RejectedGate { command, output } => {
                diag(&format!("gate red integrating {unit}: {command}"));
                self.lanes[l].gate_failures.push((
                    unit.to_string(),
                    command.clone(),
                    output.clone(),
                ));
                self.fail(
                    l,
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
                    l,
                    unit,
                    &branch,
                    "merge conflict on paths this unit may write — mainline also changed them after the branch forked",
                    &paths.join("\n"),
                )?;
                let detail = format!(
                    "merge conflict with {} on: {}",
                    self.ctx.branch,
                    paths.join(", ")
                );
                self.enforce(l, unit, &holder, verdict)?;
                self.fail(l, unit, "ownership-conflict", &detail, Counts::Quietly)
            }
            Outcome::LostRace => {
                diag(&format!(
                    "integration of {unit} lost the publish race after retries"
                ));
                self.fail(
                    l,
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
    fn close(&mut self, l: LaneId, unit: &str, outcome: &str, detail: &str) {
        let detail = first_line_clipped(detail);
        if let Some(a) = self.lanes[l].active.remove(unit)
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
            self.lane_event(l, e);
        }
        self.record(None);
    }

    /// An attempt that did not land: its work goes to its branch with the
    /// reason, its worktree is removed, its claim released.
    fn fail(
        &mut self,
        l: LaneId,
        unit: &str,
        label: &str,
        detail: &str,
        counts: Counts,
    ) -> Result<(), CoreError> {
        let lease = self.lease_minutes();
        self.abandon(l, unit, label, detail);
        // the attempt just set aside may be adoptable
        self.lanes[l].not_adoptable.remove(unit);
        let lane = &mut self.lanes[l];
        if let Some(a) = lane.active.get(unit) {
            let holder = a.holder.clone();
            lane.release(unit, &holder, lease);
        }
        lane.attempts
            .push((unit.to_string(), label.to_string(), detail.to_string()));
        self.close(l, unit, label, detail);
        match counts {
            Counts::No => Ok(()),
            Counts::Quietly => self.count_failure(l, unit, label, false),
            Counts::Work => self.count_failure(l, unit, label, true),
        }
    }

    /// Count a failed attempt. At the limit, work failures get one
    /// escalation call; at twice the limit the unit is set aside.
    fn count_failure(
        &mut self,
        l: LaneId,
        unit: &str,
        label: &str,
        work: bool,
    ) -> Result<(), CoreError> {
        let limit = self.limit();
        let lane = &mut self.lanes[l];
        let (all, ask) = {
            let f = lane.fails.entry(unit.to_string()).or_default();
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
            let (command, output) = lane
                .gate_failures
                .iter()
                .rev()
                .find(|(u, _, _)| u == unit)
                .map(|(_, c, o)| (c.clone(), o.clone()))
                .unwrap_or_else(|| {
                    let last = lane
                        .attempts
                        .iter()
                        .rev()
                        .find(|(u, _, _)| u == unit)
                        .map(|(_, l, d)| format!("{l}: {d}"))
                        .unwrap_or_default();
                    (format!("worker attempt ({label})"), last)
                });
            let verdict = self.escalate(l, unit, "-", &command, &output)?;
            self.enforce(l, unit, "escalation", verdict)?;
        }
        if all >= limit * 2 && !self.lanes[l].parked.contains_key(unit) && self.abort.is_none() {
            diag(&format!(
                "{unit}: {all} attempts did not land (last: {label}) — set aside for this run"
            ));
            return self.lanes[l].park(
                unit,
                format!("set aside after {all} attempts that did not land; last outcome {label}"),
            );
        }
        Ok(())
    }

    // ------------------------------------------------------ escalation

    fn escalate(
        &mut self,
        l: LaneId,
        unit: &str,
        branch: &str,
        command: &str,
        output: &str,
    ) -> Result<escalate::Verdict, CoreError> {
        let lane = &self.lanes[l];
        let (journal_tail, fencing) = match &lane.inst {
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
            &lane.logs_dir,
            lane.escalation_index,
        )?;
        self.lanes[l].escalation_index += 1;
        self.state.escalations.push((
            evidence_path.display().to_string(),
            verdict.verdict.as_str().to_string(),
        ));
        emit(&format!(
            "escalation verdict: {} ({})",
            verdict.verdict.as_str(),
            verdict.reason
        ));
        self.lane_event(
            l,
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
        l: LaneId,
        unit: &str,
        reporter: &str,
        verdict: escalate::Verdict,
    ) -> Result<(), CoreError> {
        match verdict.verdict {
            escalate::VerdictKind::Retry | escalate::VerdictKind::Release => Ok(()),
            escalate::VerdictKind::Block => {
                let q = verdict.q.clone().unwrap_or_else(|| "Q-???".into());
                let target = verdict.unit.clone().unwrap_or_else(|| unit.to_string());
                let lane = &mut self.lanes[l];
                lane.record_question(&q, &target, reporter, &verdict.reason)?;
                lane.park(&target, format!("open {q}: {}", verdict.reason))
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
        for l in 0..self.lanes.len() {
            self.recover_lane(l);
        }
        Ok(())
    }

    /// Release lane `l`'s claims held by runs that are gone, saving what
    /// their workers wrote, and sweep what those runs left.
    fn recover_lane(&mut self, l: LaneId) {
        let lease = self.lease_minutes();
        let mut jobs: Vec<String> = Vec::new();
        {
            let lane = &self.lanes[l];
            let Some(inst) = &lane.inst else {
                return;
            };
            for (unit, holder, job, seq) in lane::dead_claims(&self.ctx, inst) {
                emit(&lane.say(&format!(
                    "recovered {unit} (c{seq}): its holder {holder} is gone — claim released"
                )));
                self.lane_event(
                    l,
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
                let wt = lane.ml.layout.unit_worktree(&lane.ml.slug, &unit, seq);
                retry::abandon(
                    &lane.ml.primary,
                    &wt,
                    &unit,
                    seq,
                    "interrupted",
                    &format!("the run that held this claim ({job}) ended without releasing it"),
                );
                lane.release(&unit, &holder, lease);
                if !jobs.contains(&job) {
                    jobs.push(job);
                }
            }
        }
        for job in &jobs {
            self.sweep_dead_job(job);
        }
        if !jobs.is_empty() {
            self.dirty = true;
        }
    }

    /// What a dead run left in `.vitrify/`: its lock, its transaction
    /// worktrees — and possibly workers that outlived it.
    fn sweep_dead_job(&self, job: &str) {
        let scratch = &self.ctx.layout.scratch;
        let _ = std::fs::remove_file(scratch.join("jobs").join(format!("{job}.lock")));
        if let Some((_, pid)) = job.rsplit_once("-p")
            && let Ok(entries) = std::fs::read_dir(scratch)
        {
            let prefix = format!("tx-{pid}-");
            let mut dead: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .map(|e| e.path())
                .collect();
            dead.sort();
            for path in dead {
                let _ = self.ctx.primary.worktree_remove(&path);
            }
        }
        // a run killed outright cannot stop its workers; say so
        let record = self.ctx.layout.job_logs(job).join("run.json");
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
    /// is the end, and which one: Complete when every lane is, else the
    /// first other end a lane reaches.
    fn conclude(&mut self) -> Result<Option<End>, CoreError> {
        if self.ctx.project {
            return self.conclude_project();
        }
        let mut end = End::Complete;
        for l in 0..self.lanes.len() {
            match self.conclude_lane(l)? {
                None => return Ok(None),
                Some(End::Complete) => {}
                Some(other) => {
                    if matches!(end, End::Complete) {
                        end = other;
                    }
                }
            }
        }
        Ok(Some(end))
    }

    /// Ask lane `l`'s integrator for the state of the head. `None` = the
    /// run was interrupted meanwhile.
    fn head_check(&mut self, l: LaneId, want_suite: bool) -> Result<Option<HeadCheck>, CoreError> {
        // under a lane cap the head checks take turns, each lane's checkout
        // made for its check and removed after it
        let release = self.ctx.project && self.ctx.cfg.run.lane_limit().is_some();
        let _ = self.lanes[l].jobs.send(Job::CheckHead {
            want_suite,
            release,
        });
        if release {
            self.lanes[l].checkout = Checkout::Absent;
        }
        let mut asked_at = Instant::now();
        let check = loop {
            match self.events.recv_timeout(TICK) {
                Ok(Event::HeadChecked { lane, check }) if lane == l => break (*check)?,
                Ok(other) => self.handle(other)?,
                Err(RecvTimeoutError::Timeout) => {
                    self.lanes[l].integrator_alive()?;
                    if signals::pending().is_none() {
                        asked_at = Instant::now();
                    } else if asked_at.elapsed() > Duration::from_secs(10) {
                        return Ok(None);
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
        if check.interrupted || signals::pending().is_some() {
            return Ok(None);
        }
        Ok(Some(check))
    }

    /// Under a lane cap, every lane gives its checkouts back before the
    /// head checks, which then take turns.
    fn release_all(&mut self) -> Result<(), CoreError> {
        if self.ctx.cfg.run.lane_limit().is_none() {
            return Ok(());
        }
        for lane in &mut self.lanes {
            if lane.checkout == Checkout::Ready && lane.active.is_empty() && lane.searches == 0 {
                lane.checkout = Checkout::Releasing;
                let _ = lane.jobs.send(Job::Release);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while self
            .lanes
            .iter()
            .any(|l| matches!(l.checkout, Checkout::Releasing | Checkout::Preparing))
            && Instant::now() < deadline
            && signals::pending().is_none()
        {
            match self.events.recv_timeout(TICK) {
                Ok(event) => self.handle(event)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    /// The end of a project run: each lane's head checked, its outcome
    /// decided — Complete, Up to date, or Blocked with why — and the report
    /// printed: `Complete.` only when no lane is blocked.
    fn conclude_project(&mut self) -> Result<Option<End>, CoreError> {
        if self.ctx.wait
            && self
                .lanes
                .iter()
                .all(|l| l.checked_head.as_deref() == Some(l.head.as_str()))
        {
            return Ok(None); // already reported on these heads
        }
        self.release_all()?;
        let rotten = self.rotten_nodes();
        let slugs = self.lane_slugs();
        let mut reports: Vec<LaneReport> = Vec::new();
        let mut details: Vec<String> = Vec::new();
        for l in 0..self.lanes.len() {
            let slug = self.lanes[l].graph_slug();
            if let Some(first) = self.lanes[l].blocked.first() {
                let first = first.lines().next().unwrap_or("").to_string();
                reports.push(LaneReport {
                    slug,
                    outcome: LaneOutcome::Blocked {
                        detail: format!("preflight failed: {first}"),
                    },
                });
                continue;
            }
            let Some(inst) = self.lanes[l].inst.clone() else {
                continue;
            };
            let all_verified = inst
                .units
                .iter()
                .all(|u| matches!(u.meta.get_str_or("status", "Draft"), "Verified" | "Retired"));
            let Some(check) = self.head_check(l, all_verified)? else {
                return Ok(signals::pending().map(End::Interrupted));
            };
            let lane = &self.lanes[l];
            let (events, _) = journal::read(&inst.handle()).unwrap_or_default();
            let proj = journal::replay(&events);
            let replay_clean =
                proj.order_findings.is_empty() && journal::verify(&proj, &inst.units).is_empty();
            let suite = check.suite.as_ref().map(|(_, passed, _)| *passed);
            let complete = all_verified
                && lane.rotten.is_empty()
                && replay_clean
                && !events.is_empty()
                && suite != Some(false)
                && check.findings.is_empty()
                && check.broken.is_none();
            let total = inst
                .units
                .iter()
                .filter(|u| u.meta.get_str_or("status", "Draft") != "Retired")
                .count();
            let verified = inst
                .units
                .iter()
                .filter(|u| u.meta.get_str_or("status", "Draft") == "Verified")
                .count();
            let outcome = if complete && lane.built + lane.adopted + lane.rechecked == 0 {
                // since the newest recorded check: a unit's verification or
                // re-check, or the suite's pass
                let since = events
                    .iter()
                    .rev()
                    .filter(|e| e.event == "verified" || e.is_suite_pass())
                    .find_map(|e| e.commit.clone())
                    .unwrap_or_else(|| short(&check.head).to_string());
                LaneOutcome::UpToDate { since }
            } else if complete {
                let model_calls = self
                    .state
                    .workers
                    .iter()
                    .filter(|w| w.lane == lane.ml.slug && w.pid != 0)
                    .count();
                LaneOutcome::Complete {
                    verified,
                    total,
                    built: lane.built + lane.adopted,
                    rechecked: lane.rechecked,
                    changed: Vec::new(),
                    suite,
                    model_calls,
                }
            } else {
                let mut lines: Vec<String> = Vec::new();
                for v in &lane.views {
                    if let Some(r) = lane.rotten.get(&v.unit) {
                        let found =
                            r.findings.first().cloned().unwrap_or_else(|| {
                                format!("{} fails on mainline", r.rows.join(", "))
                            });
                        lines.push(format!("{} rotten: {found}", v.unit));
                    } else if !matches!(v.status.as_str(), "Verified" | "Retired") {
                        lines.push(format!("{} {}", v.unit, lane.blocker(v, &rotten, &slugs)));
                    }
                }
                if let Some((cmd, output)) = &check.broken {
                    lines.push(format!(
                        "§5 command failing at the head: `{cmd}`: {}",
                        tail(output, 1)
                    ));
                }
                for f in &check.findings {
                    lines.push(format!(
                        "audit [check {}] {}: {}",
                        f.check, f.subject, f.cause
                    ));
                }
                if suite == Some(false) {
                    lines.push("§5 Test failing at the head".to_string());
                }
                if !replay_clean {
                    lines.push("replay: inconsistent (see `vitrify journal replay`)".to_string());
                }
                let detail = lines
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "the report names each blocker".to_string());
                for line in lines.iter().skip(1) {
                    details.push(format!("    {slug} {line}"));
                }
                LaneOutcome::Blocked { detail }
            };
            reports.push(LaneReport { slug, outcome });
        }
        let blocked = reports.iter().any(|r| r.state() == LaneState::Blocked);
        for r in &reports {
            if let Some(l) = self.lanes.iter().position(|x| x.graph_slug() == r.slug) {
                self.lanes[l].state = r.state();
            }
        }
        let mut lines = project::report(&reports);
        lines.extend(details);
        out::emit("");
        for line in &lines {
            out::emit(line);
        }
        if blocked && self.ctx.wait {
            for lane in &mut self.lanes {
                lane.checked_head = Some(lane.head.clone());
            }
            emit("waiting: the run resumes when the repository changes (interrupt to stop)");
            self.waiting(
                "waiting: the run resumes when the repository changes (interrupt to stop)",
            );
            return Ok(None);
        }
        self.keep_report(lines);
        Ok(Some(if blocked { End::Blocked } else { End::Complete }))
    }

    /// Whether lane `l` has reached its end, and which one.
    fn conclude_lane(&mut self, l: LaneId) -> Result<Option<End>, CoreError> {
        let Some(inst) = self.lanes[l].inst.clone() else {
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
            let lane = &self.lanes[l];
            if lane.checked_head.as_deref() == Some(lane.head.as_str()) {
                return Ok(None); // already reported on this head
            }
            if !all_verified {
                self.lanes[l].checked_head = Some(self.lanes[l].head.clone());
                self.report(l, &inst, None, false, true);
                emit("waiting: the run resumes when the spec changes (interrupt to stop)");
                self.waiting("waiting: the run resumes when the spec changes (interrupt to stop)");
                return Ok(None);
            }
        }

        // the head itself: audit, and — when everything is Verified — the
        // whole suite. The lane's integrator owns its checkout; ask it.
        let _ = self.lanes[l].jobs.send(Job::CheckHead {
            want_suite: all_verified,
            release: false,
        });
        let mut asked_at = Instant::now();
        let check = loop {
            match self.events.recv_timeout(TICK) {
                Ok(Event::HeadChecked { lane, check }) if lane == l => break (*check)?,
                Ok(other) => self.handle(other)?,
                Err(RecvTimeoutError::Timeout) => {
                    self.lanes[l].integrator_alive()?;
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
        let (complete, lines) = self.report(l, &inst, Some(&check), all_verified, false);
        if complete {
            self.lanes[l].state = LaneState::Complete;
            self.keep_report(lines);
            return Ok(Some(End::Complete));
        }
        if self.ctx.wait {
            self.lanes[l].checked_head = Some(self.lanes[l].head.clone());
            emit("waiting: the run resumes when the repository changes (interrupt to stop)");
            self.waiting(
                "waiting: the run resumes when the repository changes (interrupt to stop)",
            );
            return Ok(None);
        }
        self.lanes[l].state = LaneState::Blocked;
        self.keep_report(lines);
        Ok(Some(End::Blocked))
    }

    /// Print lane `l`'s termination report; returns whether its
    /// done-condition holds, and the lines printed from the status line
    /// on. `check` is the state of the head (absent when the run is ending
    /// without one: interrupted, aborted, or still waiting).
    fn report(
        &self,
        l: LaneId,
        inst: &Instance,
        check: Option<&HeadCheck>,
        all_verified: bool,
        interim: bool,
    ) -> (bool, Vec<String>) {
        let lane = &self.lanes[l];
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
            && lane.rotten.is_empty()
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
                        self.ctx.branch,
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
                        self.ctx.branch,
                        tail(output, 3)
                    ));
                }
            }
            if !replay_clean {
                emit("  replay: inconsistent (see `vitrify journal replay`)");
            }
            let (rotten, lanes) = (self.rotten_nodes(), self.lane_slugs());
            for v in &lane.views {
                if let Some(r) = lane.rotten.get(&v.unit) {
                    let found = if r.findings.is_empty() {
                        "no culprit found".to_string()
                    } else {
                        r.findings.join("; ")
                    };
                    emit(&format!(
                        "  {}: rotten — {} fails on mainline too; {found}",
                        v.unit,
                        r.rows.join(", ")
                    ));
                    continue;
                }
                if v.status == "Verified" || v.status == "Retired" {
                    continue;
                }
                emit(&format!(
                    "  {}: {}",
                    v.unit,
                    lane.blocker(v, &rotten, &lanes)
                ));
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

    /// Keep a final report: `report.txt` beside the job's logs, and the
    /// run record's `report` (written with the record's next update). A
    /// lane's report follows those of the lanes before it.
    fn keep_report(&mut self, lines: Vec<String>) {
        let report = self.state.report.get_or_insert_with(Vec::new);
        report.extend(lines);
        let path = self.ctx.logs_dir.join("report.txt");
        let mut text = report.join("\n");
        text.push('\n');
        if let Err(e) =
            std::fs::create_dir_all(&self.ctx.logs_dir).and_then(|_| std::fs::write(&path, text))
        {
            diag(&format!("warning: {} not written: {e}", path.display()));
        }
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
        for lane in &mut self.lanes {
            lane.stop_threads();
        }
        if unfinished {
            for lane in &mut self.lanes {
                if lane.state != LaneState::Complete && lane.state != LaneState::UpToDate {
                    lane.state = LaneState::Blocked;
                }
            }
        }
        // whatever the integrators finished before they stopped
        while let Ok(event) = self.events.try_recv() {
            self.absorb(event);
        }
        if unfinished {
            self.hand_back(cut_short);
        }

        if let Ok(End::Aborted(reason)) = &end {
            emit(&format!("escalation verdict abort — stopping: {reason}"));
            for l in 0..self.lanes.len() {
                self.lanes[l].state = LaneState::Blocked;
                if self.derive_quietly(l)
                    && let Some(inst) = self.lanes[l].inst.clone()
                {
                    let (_, lines) = self.report(l, &inst, None, false, false);
                    self.keep_report(lines);
                }
            }
        }
        self.record(Some((label, &detail)));
        self.record_lanes(true);
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
            claim::catch_up_primary(&self.ctx.primary, &self.ctx.branch)
        };
        if let Some(warning) = behind {
            diag(&warning);
        }

        // this run's own scratch
        for lane in &self.lanes {
            lane.remove_head_checkout();
        }
        drop(self.job_lock.take());
        let _ = std::fs::remove_file(
            self.ctx
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

    /// Stop every worker and the gates, and wait for them to say so.
    fn stop_work(&mut self, why: &'static str) {
        signals::request_stop();
        let mut waiting = 0;
        for lane in &mut self.lanes {
            for a in lane.active.values_mut() {
                if a.phase == Phase::Working
                    && let Some(pid) = a.pid
                {
                    if a.killed.is_none() {
                        a.killed = Some(why);
                    }
                    gate::kill_group(pid);
                    waiting += 1;
                }
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
                lane: l,
                unit,
                outcome: Outcome::Verified { .. },
            } => {
                self.lanes[l].keeper.drop_unit(&unit);
                if let Some(a) = self.lanes[l].active.get(&unit) {
                    let adopted = a.holder.contains("-a");
                    emit(&self.say(l, &format!("integrated {unit} (c{})", a.seq)));
                    self.landed(l, &unit, adopted);
                    self.lanes[l].attempts.push((
                        unit.clone(),
                        "verified".to_string(),
                        String::new(),
                    ));
                    self.close(l, &unit, "verified", "");
                }
            }
            // a verdict the gate reached before the run ended is the
            // attempt's, whatever is ending the run: set aside under its own
            // label, so a later run judges it as the gate did
            Event::Integrated {
                lane: l,
                unit,
                outcome,
            } => {
                self.lanes[l].keeper.drop_unit(&unit);
                if let Some((label, detail)) = verdict_label(&outcome)
                    && self.lanes[l].active.contains_key(&unit)
                {
                    let _ = self.fail(l, &unit, label, &detail, Counts::No);
                }
            }
            _ => {}
        }
    }

    /// Every claim still held goes back: the work to its branch, the unit
    /// to the pool.
    fn hand_back(&mut self, why: &'static str) {
        let lease = self.lease_minutes();
        for l in 0..self.lanes.len() {
            let units: Vec<String> = self.lanes[l].active.keys().cloned().collect();
            for unit in units {
                let Some(a) = self.lanes[l].active.get(&unit) else {
                    continue;
                };
                let detail = match a.phase {
                    Phase::Working => "the run ended before the worker finished",
                    Phase::Integrating => {
                        "the work was reported built; the run ended before the gate had decided"
                    }
                };
                self.abandon(l, &unit, why, detail);
                let lane = &mut self.lanes[l];
                lane.keeper.drop_unit(&unit);
                let Some(a) = lane.active.get(&unit) else {
                    continue;
                };
                let holder = a.holder.clone();
                lane.release(&unit, &holder, lease);
                lane.attempts
                    .push((unit.clone(), why.to_string(), detail.to_string()));
                self.close(l, &unit, why, detail);
            }
        }
    }

    /// A last look at lane `l`'s head for the final report; never fails
    /// the exit.
    fn derive_quietly(&mut self, l: LaneId) -> bool {
        match self.lanes[l].derive() {
            Ok(_) => true,
            Err(e) => {
                diag(&format!("warning: final state not derived: {e}"));
                self.lanes[l].inst.is_some()
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
        for lane in &self.lanes {
            for a in lane.active.values() {
                if a.phase == Phase::Working
                    && let Some(pid) = a.pid
                {
                    gate::kill_group(pid);
                }
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

/// The label and reason an attempt is set aside under for a gate's
/// verdict; `None` for one that is no verdict on it (landed, interrupted).
fn verdict_label(outcome: &Outcome) -> Option<(&'static str, String)> {
    Some(match outcome {
        Outcome::Verified { .. } | Outcome::Interrupted => return None,
        Outcome::RejectedFencing { reason } => ("rejected-fencing", reason.clone()),
        Outcome::SkippedLeaseExpired => (
            "skipped",
            "the lease had expired when the gate started".to_string(),
        ),
        Outcome::RejectedWriteSet { detail, .. } => ("rejected-write-set", detail.clone()),
        Outcome::RejectedNotBuilt { reason } => ("rejected-not-built", reason.clone()),
        Outcome::RejectedStaleInputs { keys } => (
            "stale-inputs",
            format!(
                "built from text that has since changed: {}",
                keys.join(", ")
            ),
        ),
        Outcome::RejectedGate { command, output } => (
            "rejected-gate",
            format!(
                "failing gate: `{command}`\n{}",
                retry::clip(output, 40, 4000)
            ),
        ),
        Outcome::OwnershipConflict { paths } => (
            "ownership-conflict",
            format!("merge conflict on: {}", paths.join(", ")),
        ),
        Outcome::LostRace => (
            "lost-race",
            "mainline kept moving; the integration was not published".to_string(),
        ),
    })
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

/// Whether `commit` is a run's note setting an attempt aside.
fn set_aside(git: &crate::git::Git, commit: &str) -> bool {
    git.commit_message(commit)
        .is_ok_and(|m| m.starts_with("sdd: abandoned attempt "))
}

impl Scheduler {
    /// A unit of lane `l` landed: count it, and follow the critical path.
    fn landed(&mut self, l: LaneId, unit: &str, adopted: bool) {
        self.lanes[l].unrot(unit);
        // its gate checked it, and ran the §5 Test command if there is one
        self.lanes[l].checked.insert(unit.to_string());
        self.lanes[l].suite_ran = true;
        if adopted {
            self.lanes[l].adopted += 1;
        } else {
            self.lanes[l].built += 1;
        }
        let node = self.lanes[l].node(unit);
        if self.plan.critical_path.contains(&node) && !self.critical_ran.contains(&node) {
            self.critical_ran.push(node);
            self.record_lanes(false);
        }
    }
}

/// The first 7 characters of a commit id.
fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
