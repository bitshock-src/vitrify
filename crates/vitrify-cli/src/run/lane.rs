//! A lane: one SDD's part of a run.
//!
//! Each lane has what building one SDD needs for itself: its pinned SDD
//! and the [`Mainline`] handle that names it, its own integration
//! checkout (under its own lock) and the integrator thread that owns it,
//! its lease keeper, its head checkout, and the state the scheduler keeps
//! for it — the claims it holds, the units it has set aside, the failures
//! it has counted, the view it last derived from mainline. Its records are
//! under `.vitrify/logs/<job>/<slug>/`.
//!
//! The lane decides nothing on its own: the scheduler owns every decision,
//! and the lane's threads — its integrator, its workers' monitors, its
//! culprit searches — only report, each message naming the lane it belongs
//! to.
//!
//! A lane reads the whole repository's unit graph at the head it derives
//! from, so a unit that depends on another SDD's units (through a pin) is
//! claimable only once they are built, whichever SDDs the run builds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use vitrify_core::CoreError;
use vitrify_core::claims::Op;
use vitrify_core::graph::{self as unit_graph, Node, UnitGraph};
use vitrify_core::model::{self, Instance, UnitView, UtcTime};
use vitrify_core::trace::{self, TraceMap};

use super::events::{EventLog, RunEvent};
use super::integrate::{
    self, GateMemo, HeadCheck, IntegrateRequest, Integrator, Outcome, Rechecked, RottenRow,
};
use super::lease::{self, Keeper, MonitorParams, MonitorVerdict};
use super::mainline::Mainline;
use super::progress::GateBoard;
use super::project::{self, LaneState, outside_blocker};
use super::{Ctx, LaneSeed, tail};
use crate::cmd::{self, claim};
use crate::gate;
use crate::incremental::culprit;
use crate::incremental::dirty::{self, SpecFiles};
use crate::out::{diag, emit};

/// A lane's place in the scheduler's list of lanes.
pub(super) type LaneId = usize;

/// What a lane's threads tell the scheduler, each naming its lane.
pub(super) enum Event {
    /// A worker's process ended (or was ended).
    Exited {
        lane: LaneId,
        unit: String,
        verdict: MonitorVerdict,
        expires: UtcTime,
    },
    Integrated {
        lane: LaneId,
        unit: String,
        outcome: Outcome,
    },
    /// The integrator hit an error that is not a verdict on any unit.
    IntegratorFailed { lane: LaneId, error: String },
    HeadChecked {
        lane: LaneId,
        check: Box<Result<HeadCheck, CoreError>>,
    },
    /// The integrator found rows that fail on their gate's base too.
    Rotten { lane: LaneId, rows: Vec<RottenRow> },
    /// The lane's dynamic preflight ended: the failures, or `None` when it
    /// was interrupted.
    Prepared {
        lane: LaneId,
        result: Box<Result<Option<Vec<String>>, CoreError>>,
    },
    /// The lane's checkouts are gone (`run.lane_cap`).
    Released { lane: LaneId },
    /// A re-check batch ended.
    Rechecked {
        lane: LaneId,
        result: Box<Result<Rechecked, CoreError>>,
    },
    /// A culprit search for a rotten unit's row ended.
    Culprit {
        lane: LaneId,
        unit: String,
        report: Result<culprit::Report, String>,
    },
}

/// What the scheduler asks of a lane's integrator.
pub(super) enum Job {
    Integrate(IntegrateRequest),
    /// Check the head; then, with `release`, give the checkouts back.
    CheckHead {
        want_suite: bool,
        release: bool,
    },
    /// Make the lane's checkout and run its §5 Bootstrap, Build and Lint
    /// there: its dynamic preflight, at its first build or re-check.
    Prepare,
    /// Remove the lane's checkouts, so another lane can be admitted.
    Release,
    /// Check the unit's rows at the lane's gates again: it is no longer
    /// held rotten.
    Forget(String),
    /// Re-check these dirty Verified units: one batch, no worker.
    Recheck(Vec<String>),
    Stop,
}

#[derive(PartialEq)]
pub(super) enum Phase {
    /// The worker is running.
    Working,
    /// Reported `built`; waiting for, or in, the gate.
    Integrating,
}

/// One claim this run holds.
pub(super) struct Active {
    pub holder: String,
    pub seq: i64,
    pub branch: String,
    pub wt: PathBuf,
    /// The worker's process group; `None` for an adopted attempt, which
    /// has no worker.
    pub pid: Option<u32>,
    pub out_path: PathBuf,
    /// The worker's temp directory: this claim's alone until it is closed.
    pub _tmp: Option<gate::TmpSlot>,
    /// What the unit's inputs hashed to when it was claimed.
    pub trace: TraceMap,
    pub phase: Phase,
    /// Index of this attempt in `RunState::workers`.
    pub rec: usize,
    /// Why the scheduler ended the worker itself, if it did.
    pub killed: Option<&'static str>,
}

/// A unit the run has stopped dispatching: an open question, or too many
/// failed attempts. It returns to the pool when its reading set changes —
/// the only thing that can make another attempt go differently.
pub(super) struct Parked {
    pub reason: String,
    pub trace: TraceMap,
}

/// Where a lane's checkouts stand. All but `Absent` count against
/// `run.lane_cap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Checkout {
    /// None on disk; the lane is not admitted.
    Absent,
    /// Its dynamic preflight is running, in a checkout made for it.
    Preparing,
    /// On disk and usable.
    Ready,
    /// Being removed.
    Releasing,
}

/// A unit whose rows fail on the base of the gate that ran them: something
/// already on mainline broke it, or the rows are flaky. It is reported,
/// never rebuilt; it and the units that depend on it count as not built
/// while it fails.
#[derive(Default)]
pub(super) struct Rotten {
    /// Its failing rows, as `row <n>: \`<command>\``.
    pub rows: Vec<String>,
    /// The culprit searches' report lines, as they ended.
    pub findings: Vec<String>,
}

#[derive(Default)]
pub(super) struct Fails {
    pub all: u32,
    pub work: u32,
    pub escalated: bool,
}

pub(super) struct Lane {
    /// The SDD, pinned: path, slug, and how mainline is reached.
    pub ml: Mainline,
    /// `.vitrify/logs/<job>/<slug>/`: worker logs, prompts, the gate
    /// board, escalations.
    pub logs_dir: PathBuf,
    /// Workers at once in this lane (the run's cap bounds them all).
    pub cap: usize,
    /// Where the lane stands now; at the end, how it ended.
    pub state: LaneState,
    /// A project run: the lane's lines on stdout name it.
    pub project: bool,
    /// Its checkouts, as `run.lane_cap` counts them.
    pub checkout: Checkout,
    /// Its dynamic preflight has run (once per run).
    pub prepared: bool,
    /// Why its preflight failed (a project run): the lane is blocked.
    pub blocked: Vec<String>,
    /// The priority its gate commands wait for the gate pool at: its
    /// highest-priority unfinished unit's.
    pub priority: Arc<AtomicI64>,
    /// A re-check batch is running, of these units.
    pub rechecking: Option<Vec<String>>,
    /// Units already re-checked at a head, so a batch whose passes could
    /// not be recorded is not run again on the same tree.
    pub rechecked_at: BTreeMap<String, String>,
    /// This run: units built by workers and landed; units adopted and
    /// landed; units re-checked.
    pub built: usize,
    pub adopted: usize,
    pub rechecked: usize,
    /// Units this run has checked — re-checked, found rotten, or landed:
    /// what `--recheck` no longer distrusts.
    pub checked: BTreeSet<String>,
    /// The §5 Test command has run in this run (a gate or a re-check).
    pub suite_ran: bool,
    /// To the lane's integrator.
    pub jobs: Sender<Job>,
    pub integrator: Option<JoinHandle<()>>,
    pub keeper: Arc<Keeper>,
    pub keeper_thread: Option<JoinHandle<()>>,
    /// A detached checkout of the mainline head, for reading spec state.
    pub head_wt: PathBuf,
    pub active: BTreeMap<String, Active>,
    pub parked: BTreeMap<String, Parked>,
    /// Attempts per unit that did not land, this run: all of them, those
    /// where the work itself failed, and whether the judge has been asked.
    pub fails: BTreeMap<String, Fails>,
    /// (unit, command, output) of each gate that turned a unit away.
    pub gate_failures: Vec<(String, String, String)>,
    /// (unit, final outcome label, detail) of each attempt that ended.
    pub attempts: Vec<(String, String, String)>,
    pub escalation_index: u32,
    pub inst: Option<Instance>,
    pub views: Vec<UnitView>,
    pub head: String,
    /// The head up to which staleness has been applied.
    pub healed_at: String,
    /// The head the last termination check ran on (`--wait`).
    pub checked_head: Option<String>,
    pub audit_notes: Vec<String>,
    /// Every SDD of the repository at `graph_at`, and their unit graph
    /// (`None` when it could not be built).
    pub sdds: Vec<Instance>,
    pub graph: Option<UnitGraph>,
    graph_at: String,
    /// Units held rotten, by id.
    pub rotten: BTreeMap<String, Rotten>,
    /// Culprit searches running for this lane.
    pub searches: usize,
    /// Held by the culprit search probing the lane's probe checkout.
    pub probe_turn: Arc<Mutex<()>>,
    /// Units found not adoptable since the spec last moved.
    pub not_adoptable: BTreeSet<String>,
    /// Rows the preflight re-check found failing, for the scheduler to take
    /// up once the lane has a view.
    pub preflight_rotten: Vec<RottenRow>,
    /// The job's event log.
    log: EventLog,
}

impl Lane {
    /// Start lane `id` from what preflight established: its records
    /// directory, its integrator thread, its lease keeper.
    pub fn start(
        id: LaneId,
        seed: LaneSeed,
        ctx: &Ctx,
        events: Sender<Event>,
        log: EventLog,
        params: &MonitorParams,
    ) -> Result<Lane, CoreError> {
        let ml = seed.ml;
        let logs_dir = ctx.layout.lane_logs(&ctx.job_id, &ml.slug);
        std::fs::create_dir_all(&logs_dir).map_err(|source| CoreError::Io {
            path: logs_dir.clone(),
            source,
        })?;
        let (jobs, jobs_rx) = mpsc::channel::<Job>();
        let rotten_events = events.clone();
        let integrator = Integrator::new(ml.clone(), ctx.cfg.run.rows_at_once())
            .with_board(
                GateBoard::to(logs_dir.join("gate.json"))
                    .with_events(log.clone())
                    .for_lane(&ml.slug),
            )
            .with_rotten_sink(move |rows| {
                let _ = rotten_events.send(Event::Rotten { lane: id, rows });
            });
        let memo = GateMemo::default();
        let preflight_rotten = seed.rotten;
        let priority = Arc::new(AtomicI64::new(0));
        let lane_priority = Arc::clone(&priority);
        // rows preflight found failing are not run again while they fail
        for r in &preflight_rotten {
            integrator.hold_rotten(&r.unit);
        }
        let integrator = std::thread::spawn(move || {
            integrator_loop(id, integrator, jobs_rx, events, memo, &lane_priority)
        });

        let keeper = Arc::new(Keeper::default());
        let keeper_thread = {
            let keeper = Arc::clone(&keeper);
            let params = params.clone();
            let backend = RealLease {
                ml: ml.clone(),
                lease_minutes: ctx.cfg.run.lease_minutes,
            };
            std::thread::spawn(move || keeper.run(&params, &lease::RealClock, &backend))
        };

        Ok(Lane {
            // a project run's lanes read one head checkout, made once per
            // head: they all derive from the same mainline commit
            head_wt: if ctx.project {
                ctx.layout.head_checkout("project")
            } else {
                ctx.layout.head_checkout(&ml.slug)
            },
            ml,
            logs_dir,
            cap: seed.cap,
            state: if seed.blocked.is_empty() {
                LaneState::Build
            } else {
                LaneState::Blocked
            },
            project: ctx.project,
            // one SDD's run made its checkout at preflight; a project run's
            // lanes make theirs when admitted
            checkout: if ctx.project {
                Checkout::Absent
            } else {
                Checkout::Ready
            },
            prepared: !ctx.project,
            blocked: seed.blocked,
            priority,
            rechecking: None,
            rechecked_at: BTreeMap::new(),
            built: 0,
            adopted: 0,
            rechecked: 0,
            checked: seed.checked.iter().cloned().collect(),
            suite_ran: seed.suite_ran,
            jobs,
            integrator: Some(integrator),
            keeper,
            keeper_thread: Some(keeper_thread),
            active: BTreeMap::new(),
            parked: BTreeMap::new(),
            fails: BTreeMap::new(),
            gate_failures: Vec::new(),
            attempts: Vec::new(),
            escalation_index: 0,
            inst: None,
            views: Vec::new(),
            head: String::new(),
            healed_at: String::new(),
            checked_head: None,
            audit_notes: Vec::new(),
            sdds: Vec::new(),
            graph: None,
            graph_at: String::new(),
            rotten: BTreeMap::new(),
            searches: 0,
            probe_turn: Arc::new(Mutex::new(())),
            not_adoptable: BTreeSet::new(),
            preflight_rotten,
            log,
        })
    }

    /// The SDD's path from the repository root, as `--sdd` takes it.
    pub fn sdd(&self) -> String {
        crate::cmd::sdd_arg(&self.ml.sdd_rel)
    }

    /// Workers of this lane running now (a unit waiting for the gate holds
    /// a lease, not a slot).
    pub fn working(&self) -> usize {
        self.active
            .values()
            .filter(|a| a.phase == Phase::Working)
            .count()
    }

    fn event(&self, e: RunEvent) {
        self.log
            .write(&e.with("lane", crate::out::J::s(self.ml.slug.clone())));
    }

    /// `unit` is no longer held rotten — it was rebuilt, landed Verified, or
    /// its row was found flaky: its rows run at the lane's gates again, it
    /// may be re-checked again, and what depends on it is no longer held.
    pub fn unrot(&mut self, unit: &str) {
        if self.rotten.remove(unit).is_some() {
            let _ = self.jobs.send(Job::Forget(unit.to_string()));
        }
        self.rechecked_at.remove(unit);
    }

    /// A line of this lane's for stdout: in a project run it names the lane.
    pub fn say(&self, line: &str) -> String {
        if self.project {
            project::event_line(&self.ml.slug, line)
        } else {
            line.to_string()
        }
    }

    /// The SDD's slug as its `sdd:` front matter names it: how the unit
    /// graph names its units.
    pub fn graph_slug(&self) -> String {
        self.inst
            .as_ref()
            .map(|i| unit_graph::slug_of(i).to_string())
            .unwrap_or_else(|| self.ml.slug.clone())
    }

    /// `unit` of this lane, as the unit graph names it.
    pub fn node(&self, unit: &str) -> Node {
        Node::new(self.graph_slug(), unit)
    }

    /// The units `unit` depends on, directly or transitively, in any SDD:
    /// from the unit graph, else from its own SDD's `depends_on`.
    pub fn closure(&self, unit: &str) -> BTreeSet<Node> {
        let mut out: BTreeSet<Node> = BTreeSet::new();
        let mut todo = vec![self.node(unit)];
        while let Some(n) = todo.pop() {
            let deps: Vec<Node> = match &self.graph {
                Some(g) => g.dependencies(&n).into_iter().cloned().collect(),
                None if n.slug == self.graph_slug() => self
                    .views
                    .iter()
                    .find(|v| v.unit == n.unit)
                    .map(|v| {
                        v.depends_on
                            .iter()
                            .map(|(d, _)| Node::new(n.slug.clone(), d.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            for d in deps {
                if out.insert(d.clone()) {
                    todo.push(d);
                }
            }
        }
        out
    }

    /// The first unit, in any SDD, that `unit` depends on and that is
    /// rotten (`rotten` holds every lane's).
    pub fn rotten_dependency(&self, unit: &str, rotten: &BTreeSet<Node>) -> Option<Node> {
        if rotten.is_empty() {
            return None;
        }
        self.closure(unit).into_iter().find(|n| rotten.contains(n))
    }

    /// Each unit's view of `inst`, claimability through the unit graph.
    fn compute_views(&mut self, inst: &Instance) {
        let now = UtcTime::now();
        let slug = unit_graph::slug_of(inst).to_string();
        self.views = inst
            .units
            .iter()
            .map(|u| match &self.graph {
                Some(g) => {
                    let node = Node::new(slug.clone(), u.unit.clone());
                    model::unit_view_with(u, &inst.units, now, &g.resolver(&node))
                }
                None => model::unit_view(u, &inst.units, now),
            })
            .collect();
        self.views.sort_by(|a, b| a.unit.cmp(&b.unit));
    }

    /// Take the repository's SDDs and unit graph from `other`, read at a
    /// head this lane has not read yet: one read per head serves every
    /// lane.
    pub fn share_graph(&mut self, other: &Lane) {
        if !other.graph_at.is_empty() && other.graph_at != self.graph_at {
            self.sdds = other.sdds.clone();
            self.graph = other.graph.clone();
            self.graph_at = other.graph_at.clone();
        }
    }

    /// Every SDD of the repository at `head`, as the head checkout holds
    /// it, and their unit graph: built again only when the head moved.
    fn read_graph(&mut self, head: &str) {
        if self.graph_at == head {
            return;
        }
        // claims, renewals and releases move the head and nothing the graph
        // is built from: no need to read it all again
        if !self.graph_at.is_empty()
            && self.graph.is_some()
            && dirty::benign(
                &self.ml.primary,
                &self.graph_at,
                head,
                &SpecFiles::of(&self.head_wt, &self.sdds),
            )
            .unwrap_or(false)
        {
            self.graph_at = head.to_string();
            return;
        }
        self.sdds = cmd::discover(&self.head_wt)
            .into_iter()
            .filter_map(|h| Instance::load(h).ok())
            .collect();
        self.graph = match UnitGraph::build(&self.sdds) {
            Ok(g) => Some(g),
            Err(e) => {
                diag(&format!(
                    "note: the repository's unit graph could not be read ({e}); units that depend on other SDDs wait"
                ));
                None
            }
        };
        self.graph_at = head.to_string();
    }

    /// Read the mainline head: apply staleness where the spec moved, load
    /// the SDD, compute the views, return parked units whose reading set
    /// changed to the pool, and stop workers building from text that is no
    /// longer current. `false` = mainline was too busy to be put right;
    /// the view was left as it was.
    pub fn derive(&mut self) -> Result<bool, CoreError> {
        let mut head = self.ml.head()?;
        // the same head: only the clock has moved (a lease may have lapsed)
        if head == self.head
            && let Some(inst) = self.inst.take()
        {
            self.compute_views(&inst);
            self.inst = Some(inst);
            return Ok(true);
        }
        if self.healed_at.is_empty() {
            self.healed_at = head.clone(); // preflight left the audit clean
        }
        if head != self.healed_at {
            if integrate::spec_moved(&self.ml, &self.healed_at, &head)? {
                // what was not adoptable may be now, and the other way round
                self.not_adoptable.clear();
                let healed = match self.ml.heal(false, true) {
                    Ok(healed) => healed,
                    Err(e) if e.exit_code() == 5 => {
                        // mainline too busy to publish it now: next pass
                        diag(&format!("note: {e}"));
                        return Ok(false);
                    }
                    Err(e) => return Err(e),
                };
                for (unit, cause) in &healed.stale {
                    self.unrot(unit);
                    emit(&self.say(&format!("stale {unit}: {cause} — it will be rebuilt")));
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
                    head = self.ml.head()?;
                }
            }
            self.healed_at = head.clone();
        }
        {
            // adding the head checkout is one more worktree: its turn like
            // any other; resetting it touches nothing another writer uses
            let made = self.head_wt.join(".git").exists();
            // another lane of the run brought it to this head already
            if !made || !head_checkout_at(&self.head_wt, &head) {
                let _turn = (!made).then(claim::tx_lock);
                self.ml.pristine(&self.head_wt, &head)?;
                head_checkout_made(&self.head_wt, &head);
            }
        }
        let inst = self.ml.sdd_in(&self.head_wt)?;
        self.read_graph(&head);
        self.compute_views(&inst);

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
            emit(&self.say(&format!(
                "{unit}: its reading set changed — back in the pool"
            )));
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
                && let Some(pid) = a.pid
                && let Some(u) = inst.unit(unit)
                && trace::compute(&inst, u)?.0 != a.trace
            {
                diag(&format!(
                    "{unit}: its inputs changed on {} while it was being built — stopping the worker; the unit is rebuilt from the current text",
                    self.ml.branch
                ));
                a.killed = Some("stale-inputs");
                stale.push(pid);
            }
        }
        for pid in stale {
            gate::kill_group(pid);
        }

        self.inst = Some(inst);
        self.head = head;
        Ok(true)
    }

    /// The integrator thread ends only when told to. If it is gone, units
    /// waiting for the gate would wait forever with their leases renewed.
    pub fn integrator_alive(&self) -> Result<(), CoreError> {
        if self.integrator.as_ref().is_some_and(|t| t.is_finished()) {
            return Err(CoreError::refusal(
                1,
                "error: the integrator thread ended unexpectedly",
            ));
        }
        Ok(())
    }

    /// Stop dispatching `unit` until its reading set changes.
    pub fn park(&mut self, unit: &str, reason: String) -> Result<(), CoreError> {
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
    pub fn record_question(
        &self,
        q: &str,
        unit: &str,
        reporter: &str,
        notes: &str,
    ) -> Result<(), CoreError> {
        match self.ml.land_q_entry(q, unit, reporter, notes) {
            Err(e) if e.exit_code() == 5 => {
                diag(&format!(
                    "warning: {q} for {unit} was not recorded in the Decision Log ({e}); record it by hand: {notes}"
                ));
                Ok(())
            }
            other => other,
        }
    }

    /// Give `holder`'s claim on `unit` back.
    pub fn release(&self, unit: &str, holder: &str, lease_minutes: i64) {
        // a release that only lost the race for the ref is tried again: a
        // claim left standing keeps the unit out of the pool for its lease
        for attempt in 0..3 {
            match self.ml.claim(Op::Release, unit, holder, lease_minutes) {
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

    /// Why a unit is not Verified, from the most specific cause down.
    /// `rotten` holds every lane's rotten units; `lanes` the slugs the run
    /// builds.
    pub fn blocker(&self, v: &UnitView, rotten: &BTreeSet<Node>, lanes: &[String]) -> String {
        let attempts = || {
            self.attempts
                .iter()
                .filter(|(u, _, _)| u == &v.unit)
                .count()
        };
        if let Some(parked) = self.parked.get(&v.unit) {
            return parked.reason.clone();
        }
        if let Some(r) = self.rotten_dependency(&v.unit, rotten) {
            let own = r.slug == self.graph_slug();
            return format!(
                "waits on {} (rotten)",
                if own { r.unit.clone() } else { r.to_string() }
            );
        }
        if let Some((_, command, output)) = self
            .gate_failures
            .iter()
            .rev()
            .find(|(u, _, _)| u == &v.unit)
        {
            return format!("failing gate `{command}`: {}", tail(output, 3));
        }
        // the workers' own accounts: last attempt's outcome + notes
        if let Some((_, outcome, notes)) = self.attempts.iter().rev().find(|(u, _, _)| u == &v.unit)
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
        // waiting on units of other SDDs, through a pin
        let waits_outside = v.through.iter().any(|t| {
            v.depends_on
                .iter()
                .any(|(d, s)| d == &t.entry && s != "Built" && s != "Verified")
        });
        if waits_outside && let Some(g) = &self.graph {
            let node = self.node(&v.unit);
            let status = |n: &Node| g.status(n).map(str::to_string);
            if let Some(line) = outside_blocker(g, lanes, &node, &status) {
                let line = line
                    .strip_prefix(&format!("{} ", v.unit))
                    .unwrap_or(&line)
                    .to_string();
                return line;
            }
            let pending: Vec<String> = v
                .through
                .iter()
                .flat_map(|t| t.units.iter())
                .filter(|(_, s)| s != "Built" && s != "Verified")
                .map(|(u, s)| format!("{u} ({s})"))
                .collect();
            return format!("waits on {}", pending.join(", "));
        }
        // a link of the chain: a unit with its status, or a pin entry as
        // what it resolved to — the chain ends there
        let link = |x: &UnitView, dep: &str, status: &str| -> (String, bool) {
            let Some(t) = x.through.iter().find(|t| t.entry == dep) else {
                return (format!("{dep} ({status})"), false);
            };
            if let Some(g) = &self.graph {
                let node = self.node(&x.unit);
                let status = |n: &Node| g.status(n).map(str::to_string);
                if let Some(line) = outside_blocker(g, lanes, &node, &status)
                    && let Some(rest) = line.strip_prefix(&format!("{} waits on ", x.unit))
                {
                    return (format!("{dep} → {rest}"), true);
                }
            }
            let pending: Vec<String> = t
                .units
                .iter()
                .filter(|(_, s)| s != "Built" && s != "Verified")
                .map(|(u, s)| format!("{u} ({s})"))
                .collect();
            (format!("{dep} → {}", pending.join(", ")), true)
        };
        if let Some((dep, status)) = unmet(v) {
            let (first, mut ended) = link(v, &dep, &status);
            let mut chain = vec![first];
            let mut cur = dep;
            while !ended
                && let Some(x) = self.views.iter().find(|x| x.unit == cur)
                && let Some((next, status)) = unmet(x)
            {
                if chain.len() > self.views.len() {
                    break; // a cycle lint would have refused; never loop on one
                }
                let (text, pin) = link(x, &next, &status);
                chain.push(text);
                ended = pin;
                cur = next;
            }
            return format!("dependency chain blocked: {}", chain.join(" ← "));
        }
        format!("not attempted this run (status {})", v.status)
    }

    /// End the lane's threads: the integrator after whatever it is doing,
    /// the keeper at once.
    pub fn stop_threads(&mut self) {
        let _ = self.jobs.send(Job::Stop);
        self.keeper.stop();
        if let Some(t) = self.integrator.take() {
            let _ = t.join();
        }
        if let Some(t) = self.keeper_thread.take() {
            let _ = t.join();
        }
    }

    /// Remove the lane's head checkout: this run's own scratch.
    pub fn remove_head_checkout(&self) {
        if self.head_wt.exists() {
            let _ = self.ml.primary.worktree_remove(&self.head_wt);
        }
    }
}

/// The head each head checkout was last made pristine at, in this process.
static HEAD_CHECKOUTS: std::sync::Mutex<Vec<(PathBuf, String)>> = std::sync::Mutex::new(Vec::new());

/// Whether the head checkout at `path` was made pristine at `head` last.
fn head_checkout_at(path: &Path, head: &str) -> bool {
    HEAD_CHECKOUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(p, h)| p == path && h == head)
}

fn head_checkout_made(path: &Path, head: &str) {
    let mut made = HEAD_CHECKOUTS.lock().unwrap_or_else(|e| e.into_inner());
    made.retain(|(p, _)| p != path);
    made.push((path.to_path_buf(), head.to_string()));
}

// ---------------------------------------------------------- dead runs

/// `run-<stamp>-p<pid>-w<n>` (a worker's claim) or `…-a<n>` (an adopted
/// attempt's) → (`run-<stamp>-p<pid>`, pid).
fn run_holder(holder: &str) -> Option<(String, u32)> {
    let (job, w) = holder
        .rsplit_once("-w")
        .or_else(|| holder.rsplit_once("-a"))?;
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

/// Claims on `inst`'s units held by runs that are gone: (unit, holder,
/// job, claim_seq).
pub(super) fn dead_claims(ctx: &Ctx, inst: &Instance) -> Vec<(String, String, String, i64)> {
    let mut out = Vec::new();
    for u in &inst.units {
        if u.meta.get_str_or("status", "Draft") != "Claimed" {
            continue;
        }
        let holder = u.meta.get_str_or("claimed_by", "").to_string();
        let Some((job, pid)) = run_holder(&holder) else {
            continue;
        };
        if job != ctx.job_id && liveness(&ctx.layout.scratch, &job, pid) == Liveness::Dead {
            out.push((u.unit.clone(), holder, job, u.meta.get_int("claim_seq")));
        }
    }
    out
}

/// `--dry-run`: what the run would release before dispatching.
pub(super) fn dead_run_notes(ctx: &Ctx, inst: &Instance) -> Vec<String> {
    dead_claims(ctx, inst)
        .into_iter()
        .map(|(unit, holder, _, seq)| {
            format!(
                "Recovery: would release {unit} (c{seq}), claimed by {holder} — that run is gone"
            )
        })
        .collect()
}

// ---------------------------------------------------------- threads

/// The lease backend: claim transactions on one lane's SDD.
pub(super) struct RealLease {
    pub ml: Mainline,
    pub lease_minutes: i64,
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

/// The integrator thread of lane `lane`: every built unit that is waiting,
/// as one batch.
fn integrator_loop(
    lane: LaneId,
    integrator: Integrator,
    jobs: Receiver<Job>,
    events: Sender<Event>,
    memo: GateMemo,
    priority: &AtomicI64,
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
            Job::Forget(unit) => integrator.forget_rotten(&unit),
            Job::Prepare => {
                let mut failures = Vec::new();
                let result = gate::at_priority(priority.load(Ordering::Relaxed), || {
                    super::dynamic_checks(&integrator.ml, &mut failures, &|| {
                        diag(
                            "note: another vitrify process is integrating this SDD — waiting for its integration checkout",
                        )
                    })
                })
                .map(|done| done.map(|_| failures));
                let _ = events.send(Event::Prepared {
                    lane,
                    result: Box::new(result),
                });
            }
            Job::Release => {
                integrator.ml.remove_checkouts();
                let _ = events.send(Event::Released { lane });
            }
            Job::Recheck(units) => {
                let result = gate::at_priority(priority.load(Ordering::Relaxed), || {
                    integrator.recheck(&units)
                });
                let _ = events.send(Event::Rechecked {
                    lane,
                    result: Box::new(result),
                });
            }
            Job::CheckHead {
                want_suite,
                release,
            } => {
                let check = gate::at_priority(priority.load(Ordering::Relaxed), || {
                    integrator.check_head(&mut memo, want_suite)
                });
                if release {
                    integrator.ml.remove_checkouts();
                }
                let _ = events.send(Event::HeadChecked {
                    lane,
                    check: Box::new(check),
                });
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
                let done = gate::at_priority(priority.load(Ordering::Relaxed), || {
                    integrator.integrate_batch(&batch, &mut memo, None)
                });
                match done {
                    Ok(outcomes) => {
                        // what the gate found rotten, before what it decided
                        let rows = integrator.take_rotten();
                        if !rows.is_empty() {
                            let _ = events.send(Event::Rotten { lane, rows });
                        }
                        for (req, outcome) in batch.iter().zip(outcomes) {
                            let _ = events.send(Event::Integrated {
                                lane,
                                unit: req.unit.clone(),
                                outcome,
                            });
                        }
                    }
                    Err(e) => {
                        let _ = events.send(Event::IntegratorFailed {
                            lane,
                            error: e.to_string(),
                        });
                    }
                }
            }
        }
    }
}
