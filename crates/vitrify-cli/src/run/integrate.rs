//! A lane's serial integrator: every Built unit of its SDD that is ready, as
//! one batch, in the lane's own integration checkout. Fencing check first;
//! then the write-set check (the branch changed only what the unit may write); a
//! `--no-ff` merge per unit; the input check (the unit was built from the
//! spec text mainline holds now); one run of the full integration gate over
//! the merged tree; one integration commit carrying stamps, journal events,
//! ledger, and a clean audit; CAS/push publish with lost-race redo; never
//! leaves mainline failing. A red batch is integrated again one unit at a
//! time, so one bad unit never holds back the others.
//!
//! The gate: the SDD's §5 Bootstrap and Build; the Verification tables of
//! the units being integrated and of the SDD's **dirty** Verified units —
//! those a path they are checked against changed since their last recorded
//! check, every dependent of the merged units among them — each distinct
//! command once, `run.gate_jobs` at a time ([`rows::for_gate`]); the §5
//! Lint command; then the §5 Test command — the whole suite, which a merge
//! must not turn from passing to failing — and audit checks 1–3. Rows of
//! clean units are not run: their recorded checks hold. An SDD without a
//! Test command has no backstop for what its declarations miss, so its gate
//! runs every Verified unit's rows. The integration commit records the
//! re-checks the gate made (a `verified` event with actor `recheck` and a
//! fresh stamp for each dirty unit whose rows passed) and `suite: "pass"`
//! when the Test command passed, so a later run trusts them.
//!
//! A lane that loses the publish race does not always gate again. The
//! commits it lost to are replayed over when, beyond protocol state, they
//! changed nothing its gate checked or read ([`Footprint`]): no file of a
//! unit whose rows ran or of anything those units depend on, none of its
//! SDD's unmanaged files, no path outside every scope the SDD takes as
//! input, and — when the gate ran the §5 Test command — nothing in its
//! SDD's scope at all. The merges are then replayed on the new head, the
//! fencing, write-set and input checks made again there, and the result
//! published without another gate.
//!
//! No new failures: a failing row of a unit already Verified (and not in
//! the batch) is run again on the base, in the lane's own pristine base
//! checkout ([`blame::judge`]). A row that passes there was broken by the
//! batch, which is rejected. A row that fails there too was broken before
//! the batch: its unit is **rotten**, the batch is judged on its other rows,
//! and the rotten unit's rows are left out of the lane's later gates until
//! the scheduler says otherwise ([`Integrator::forget_rotten`]). A failing
//! row of the batch's own units is always the batch's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use vitrify_core::audit;
use vitrify_core::cascade::{Owner, Ownership};
use vitrify_core::graph::{self as unit_graph, Node};
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{self, Instance, Shape, UtcTime};
use vitrify_core::writeset::{Violation, WriteSet};
use vitrify_core::{CoreError, ledger, trace, yamlish};

use super::mainline::{LaneLock, Mainline, append_event};
use super::progress::GateBoard;
use crate::cmd::{self, claim};
use crate::gate::{self, Ran, ShRunner};
use crate::git::Git;
use crate::incremental::blame::{self, Judged, PristineCheckout, ShellRunner};
use crate::incremental::dirty::{self, Cascade, DiffCache, Mode, Snapshot, SpecFiles};
use crate::incremental::{recheck, rows};
use crate::layout;
use crate::out::diag;
use crate::signals;

pub struct IntegrateRequest {
    pub unit: String,
    pub token_seq: i64,
    pub builder_holder: String,
    pub integrator_id: String,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Verified {
        merge_commit: String,
        verify_commit: String,
    },
    /// Superseded token or wrong state — rejected outright, tests not run.
    RejectedFencing { reason: String },
    /// Token current but the lease merely expired at gate start: not an
    /// error — the unit returns to the pool.
    SkippedLeaseExpired,
    /// The branch changed paths outside the unit's write-set (schema §13.3):
    /// files the unit does not own (`unmanaged` ones included), other files
    /// of the SDD, or its own spec text beyond the handoff fields. Rejected
    /// before merging — nothing merged, nothing published. `detail` holds
    /// one line per violation.
    RejectedWriteSet { paths: Vec<String>, detail: String },
    /// The work does not stamp the unit Built with a recorded trace: there
    /// is nothing to say what it was built from.
    RejectedNotBuilt { reason: String },
    /// The trace the unit was built from no longer matches the spec text at
    /// the mainline head: an input (`keys`) changed while it was being
    /// built. Not the work's fault — the unit is rebuilt from current text.
    RejectedStaleInputs { keys: Vec<String> },
    /// Gate red: reverted to the recorded base; nothing published.
    RejectedGate { command: String, output: String },
    /// Merge conflict. The write-set check already passed, so every
    /// conflicted path is one this unit may write that mainline also changed
    /// after the branch forked — treated as an ownership violation: escalate.
    OwnershipConflict { paths: Vec<String> },
    /// CAS/push lost after the retry budget.
    LostRace,
    /// The run was interrupted before this unit's integration finished;
    /// nothing was published.
    Interrupted,
}

/// The SDD §5 Commands block (`### Commands` in the core file). `vitrify run`
/// preflight runs bootstrap, build and lint; the integration gate runs all
/// four — `test` as a regression check (see [`Integrator`]).
#[derive(Debug, Clone, Default)]
pub struct SddCommands {
    pub bootstrap: Option<String>,
    pub build: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
}

pub fn sdd_commands(core_text: &str) -> SddCommands {
    // one parser for §5 Commands: what the gate runs is what a re-check runs
    let c = crate::incremental::rows::Commands::parse(core_text);
    SddCommands {
        bootstrap: c.bootstrap,
        build: c.build,
        test: c.test,
        lint: c.lint,
    }
}

/// Run one gate command via `sh -c` (the sanctioned shell use).
pub fn run_gate_command(cmd: &str, cwd: &Path) -> (bool, String) {
    let ran = gate::run_command(cmd, cwd);
    (ran.ok(), ran.output)
}

/// What a gate established, carried from one integration to the next so
/// that nothing is run twice on the same tree.
#[derive(Debug, Clone, Default)]
pub struct GateMemo {
    /// The §5 Test command's result at a mainline commit.
    pub suite: Option<(String, bool)>,
}

/// The state of the mainline head, for the termination report.
#[derive(Debug, Clone)]
pub struct HeadCheck {
    pub head: String,
    pub findings: Vec<audit::Finding>,
    /// A §5 Bootstrap or Build that failed at the head: (command, output).
    pub broken: Option<(String, String)>,
    /// The §5 Test command at the head: (command, passed, output). `None`
    /// when the SDD has none or it was not asked for.
    pub suite: Option<(String, bool, String)>,
    pub interrupted: bool,
}

/// One merged member of a batch.
struct Merged {
    /// Index into the batch's requests.
    i: usize,
    branch: String,
    merge_sha: String,
}

enum Gate {
    /// Every selected row passed (or was found rotten): the dirty Verified
    /// units whose rows passed, to be recorded as re-checked, and what the
    /// gate checked or read.
    Green {
        rechecked: Vec<String>,
        footprint: Option<Footprint>,
    },
    Red {
        command: String,
        output: String,
    },
    Interrupted,
}

enum Suite {
    /// No Test command, or it passed.
    Fine(Option<bool>),
    /// It failed after the merges and had passed before them.
    Regressed {
        command: String,
        output: String,
    },
    Interrupted,
}

enum Final {
    Published,
    /// Mainline moved in substance (or a member dropped out): gate again.
    Redo(Vec<usize>),
    Red {
        command: String,
        output: String,
    },
    Interrupted,
}

/// Puts the board back to `idle` however the integration it covers ends:
/// an error, an early return or a panic leaves no stale step behind.
struct IdleOnDrop<'a>(&'a GateBoard);

impl Drop for IdleOnDrop<'_> {
    fn drop(&mut self) {
        self.0.idle();
    }
}

/// How many times a publish is built and tried before the batch is gated
/// again: a lost race costs a replay, not a gate, while the commits that
/// won it leave the verdict standing.
const PUBLISH_TRIES: usize = 12;

/// A Verification row that failed at a gate and on its base too: its unit
/// is rotten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RottenRow {
    pub unit: String,
    /// The row's 1-based position in the unit's Verification table.
    pub row_number: usize,
    pub row: String,
    /// The commit the batch was merged onto, where the row failed too.
    pub base: String,
    /// The tail of the row's output at the gate.
    pub output: String,
}

pub struct Integrator {
    pub ml: Mainline,
    /// Verification rows run at once.
    pub gate_jobs: usize,
    /// Where the integrator says which step and commands it is running.
    pub board: GateBoard,
    /// Rotten rows found since the last [`Integrator::take_rotten`], when
    /// no sink takes them as they are found.
    found: Mutex<Vec<RottenRow>>,
    /// Told of rotten rows the moment they are found — before anything is
    /// published — so the units that depend on them are never dispatched
    /// against a head that already holds the verdict's batch.
    rotten_sink: Option<Box<dyn Fn(Vec<RottenRow>) + Send>>,
    /// Units held rotten: their rows are left out of later gates.
    held: Mutex<BTreeSet<String>>,
    /// What the dirty sets of earlier gates found, for the next one's.
    diffs: Mutex<DiffCache>,
}

impl Integrator {
    pub fn new(ml: Mainline, gate_jobs: usize) -> Integrator {
        Integrator {
            ml,
            gate_jobs: gate_jobs.max(1),
            board: GateBoard::off(),
            found: Mutex::new(Vec::new()),
            held: Mutex::new(BTreeSet::new()),
            diffs: Mutex::new(DiffCache::default()),
            rotten_sink: None,
        }
    }

    /// The integrator, telling `sink` of rotten rows as soon as it finds
    /// them.
    pub fn with_rotten_sink(self, sink: impl Fn(Vec<RottenRow>) + Send + 'static) -> Integrator {
        Integrator {
            rotten_sink: Some(Box::new(sink)),
            ..self
        }
    }

    /// Hold the units of `rows` rotten, and pass the rows on: to the sink at
    /// once, else to [`Integrator::take_rotten`].
    fn report_rotten(&self, rows: Vec<RottenRow>) {
        if rows.is_empty() {
            return;
        }
        for r in &rows {
            self.hold_rotten(&r.unit);
        }
        match &self.rotten_sink {
            Some(sink) => sink(rows),
            None => self
                .found
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(rows),
        }
    }

    /// The rows found rotten since the last call, in the order found.
    pub fn take_rotten(&self) -> Vec<RottenRow> {
        std::mem::take(&mut *self.found.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Leave `unit`'s rows out of the lane's gates: it is rotten.
    pub fn hold_rotten(&self, unit: &str) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(unit.to_string());
    }

    /// Run `unit`'s rows at the lane's gates again: it is no longer held
    /// rotten (its culprit search found the row flaky, or it was rebuilt).
    pub fn forget_rotten(&self, unit: &str) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(unit);
    }

    /// The units held rotten now.
    fn held_rotten(&self) -> BTreeSet<String> {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Where this lane runs failing rows on their base.
    fn base_checkout(&self) -> PristineCheckout {
        PristineCheckout::new(
            self.ml.primary.clone(),
            self.ml.layout.base_checkout(&self.ml.slug),
        )
    }

    /// The integrator, reporting its progress to `board`.
    pub fn with_board(self, board: GateBoard) -> Integrator {
        Integrator { board, ..self }
    }

    fn checkout(&self) -> PathBuf {
        self.ml.checkout()
    }

    /// Take the lane's integration checkout. `None` = interrupted while
    /// waiting.
    fn lock_checkout(&self) -> Result<Option<LaneLock>, CoreError> {
        self.ml.lock_checkout(&|| {
            self.board.step("lock");
            diag(
                "note: another vitrify process is integrating this SDD — waiting for its integration checkout",
            );
        })
    }

    /// Integrate one Built unit. `before_publish` is a test seam fired after
    /// the gate and before the CAS/push (lost-race rehearsals).
    pub fn integrate(
        &self,
        req: &IntegrateRequest,
        memo: &mut GateMemo,
        before_publish: Option<&dyn Fn()>,
    ) -> Result<Outcome, CoreError> {
        let mut out = self.integrate_batch(std::slice::from_ref(req), memo, before_publish)?;
        Ok(out.pop().unwrap_or(Outcome::LostRace))
    }

    /// Integrate every request as one batch: one gate run for all of them.
    /// Outcomes are index-aligned with `reqs`. A red batch of several is
    /// integrated again one at a time.
    pub fn integrate_batch(
        &self,
        reqs: &[IntegrateRequest],
        memo: &mut GateMemo,
        before_publish: Option<&dyn Fn()>,
    ) -> Result<Vec<Outcome>, CoreError> {
        let _idle = IdleOnDrop(&self.board);
        self.board
            .batch(reqs.iter().map(|r| r.unit.clone()).collect());
        let mut out: Vec<Option<Outcome>> = vec![None; reqs.len()];
        match self.lock_checkout()? {
            Some(_held) => {
                let all: Vec<usize> = (0..reqs.len()).collect();
                self.run_batch(&all, reqs, &mut out, memo, before_publish)?;
            }
            None => out.fill(Some(Outcome::Interrupted)),
        }
        Ok(out
            .into_iter()
            .map(|o| o.unwrap_or(Outcome::LostRace))
            .collect())
    }

    fn run_batch(
        &self,
        idx: &[usize],
        reqs: &[IntegrateRequest],
        out: &mut [Option<Outcome>],
        memo: &mut GateMemo,
        before_publish: Option<&dyn Fn()>,
    ) -> Result<(), CoreError> {
        let mlwt = self.checkout();
        let ml = self.ml.primary.at(&mlwt);
        let mut pending: Vec<usize> = idx.to_vec();
        let units = |which: &[usize]| -> Vec<String> {
            which.iter().map(|&i| reqs[i].unit.clone()).collect()
        };
        let interrupt = |out: &mut [Option<Outcome>], which: &[usize]| {
            for &i in which {
                if out[i].is_none() {
                    out[i] = Some(Outcome::Interrupted);
                }
            }
        };

        for _attempt in 0..3 {
            if signals::interrupted() {
                interrupt(out, &pending);
                return Ok(());
            }
            self.board.batch(units(&pending));
            self.board.step("sync");
            // 1. sync; record the CAS base. Mainline is first put right
            // where the audit can do it mechanically, so that a lapsed lease
            // or a spec change elsewhere is never held against this batch.
            // A unit of the batch whose own lease has lapsed is decided
            // before that: skipped (back to the pool), not rejected.
            self.ml.sync_checkout(&mlwt)?;
            let at_head = self.ml.sdd_in(&mlwt)?;
            pending.retain(|&i| {
                let lapsed = at_head.unit(&reqs[i].unit).is_some_and(|u| {
                    u.meta.get_int("claim_seq") == reqs[i].token_seq
                        && u.meta.get_str_or("status", "Draft") == "Claimed"
                        && !UtcTime::parse(u.meta.get_str_or("claim_expires", ""))
                            .is_some_and(|e| e > UtcTime::now())
                });
                if lapsed {
                    out[i] = Some(Outcome::SkippedLeaseExpired);
                }
                !lapsed
            });
            if pending.is_empty() {
                return Ok(());
            }
            match self.ml.heal(true, true) {
                Ok(_) => {}
                // mainline too busy to publish the housekeeping: the final
                // audit decides whether it mattered
                Err(e) if e.exit_code() == 5 => diag(&format!("note: {e}")),
                Err(e) => return Err(e),
            }
            let base = self.ml.sync_checkout(&mlwt)?;

            // 2–3. fencing, write-set, merge, input check — per unit
            self.board.batch(units(&pending));
            self.board.step("merge");
            let merged = self.assemble(&ml, &mlwt, &base, &pending, reqs, out)?;
            if merged.is_empty() {
                return Ok(());
            }
            let members: Vec<usize> = merged.iter().map(|m| m.i).collect();
            self.board.batch(units(&members));

            // 4. the full integration gate, once for the batch
            let red = |this: &Integrator,
                       out: &mut [Option<Outcome>],
                       memo: &mut GateMemo,
                       command: String,
                       output: String|
             -> Result<(), CoreError> {
                this.ml.pristine(&mlwt, &base)?;
                if members.len() > 1 {
                    diag(&format!(
                        "note: gate red for a batch of {} ({command}); integrating them one at a time",
                        members.len()
                    ));
                    for &i in &members {
                        this.run_batch(&[i], reqs, out, memo, before_publish)?;
                    }
                } else {
                    out[members[0]] = Some(Outcome::RejectedGate { command, output });
                }
                Ok(())
            };
            let (rechecked, footprint) = match self.gate(&mlwt, &base, &merged, reqs)? {
                Gate::Green {
                    rechecked,
                    footprint,
                } => (rechecked, footprint),
                Gate::Red { command, output } => return red(self, out, memo, command, output),
                Gate::Interrupted => {
                    let _ = self.ml.pristine(&mlwt, &base);
                    interrupt(out, &members);
                    return Ok(());
                }
            };
            let mut rebuilt = false;
            let suite = match self.suite(&mlwt, &base, memo, &mut rebuilt)? {
                Suite::Fine(result) => result,
                Suite::Regressed { command, output } => {
                    return red(self, out, memo, command, output);
                }
                Suite::Interrupted => {
                    let _ = self.ml.pristine(&mlwt, &base);
                    interrupt(out, &members);
                    return Ok(());
                }
            };

            // 5. stamp, journal, ledger, audit, publish
            match self.finalize(
                &ml,
                &mlwt,
                &base,
                merged,
                rebuilt,
                suite,
                &rechecked,
                footprint.as_ref(),
                reqs,
                out,
                memo,
                before_publish,
            )? {
                Final::Published => return Ok(()),
                Final::Red { command, output } => return red(self, out, memo, command, output),
                Final::Interrupted => {
                    let _ = self.ml.pristine(&mlwt, &base);
                    interrupt(out, &members);
                    return Ok(());
                }
                Final::Redo(remaining) => {
                    if remaining.is_empty() {
                        return Ok(());
                    }
                    diag(&format!(
                        "note: {} changed in substance during the gate; integrating again from the new head",
                        self.ml.branch
                    ));
                    pending = remaining;
                }
            }
        }
        for i in pending {
            if out[i].is_none() {
                out[i] = Some(Outcome::LostRace);
            }
        }
        Ok(())
    }

    /// Fencing check, write-set check, merge and input check for each of
    /// `which`, on the checkout at `base`. Rejections are recorded in `out`;
    /// the units that merged are returned in order.
    fn assemble(
        &self,
        ml: &Git,
        mlwt: &Path,
        base: &str,
        which: &[usize],
        reqs: &[IntegrateRequest],
        out: &mut [Option<Outcome>],
    ) -> Result<Vec<Merged>, CoreError> {
        let inst = self.ml.sdd_in(mlwt)?;
        let slug = layout::sdd_slug(&inst);
        let mut candidates: Vec<(usize, String)> = Vec::new();
        for &i in which {
            let req = &reqs[i];
            let branch = layout::unit_branch(&slug, &req.unit, req.token_seq);
            // fencing check FIRST, against front matter at this head
            let Some(unit) = inst.unit(&req.unit) else {
                out[i] = Some(Outcome::RejectedFencing {
                    reason: format!("unit {} not found at the mainline head", req.unit),
                });
                continue;
            };
            let current_seq = unit.meta.get_int("claim_seq");
            if req.token_seq != current_seq {
                out[i] = Some(Outcome::RejectedFencing {
                    reason: format!(
                        "superseded token: branch carries c{}, current claim_seq is {}",
                        req.token_seq, current_seq
                    ),
                });
                continue;
            }
            match unit.meta.get_str_or("status", "Draft") {
                "Built" => {}
                "Claimed" => {
                    let expires = UtcTime::parse(unit.meta.get_str_or("claim_expires", ""));
                    if !expires.is_some_and(|e| e > UtcTime::now()) {
                        out[i] = Some(Outcome::SkippedLeaseExpired);
                        continue;
                    }
                }
                other => {
                    out[i] = Some(Outcome::RejectedFencing {
                        reason: format!("unit {} is {other}, not Built/Claimed", req.unit),
                    });
                    continue;
                }
            }
            if !ml.branch_exists(&branch).map_err(CoreError::from)? {
                out[i] = Some(Outcome::RejectedFencing {
                    reason: format!("work branch {branch} does not exist"),
                });
                continue;
            }
            // write-set check, before anything merges: every path the branch
            // changed since it left mainline is owned by the unit (never a
            // file of the SDD), or a handoff field of its own spec entry
            let violations = write_set_violations(ml, &inst, unit, mlwt, base, &branch)?;
            if !violations.is_empty() {
                out[i] = Some(Outcome::RejectedWriteSet {
                    paths: violations.iter().map(|v| v.path().to_string()).collect(),
                    detail: violations
                        .iter()
                        .map(|v| v.describe(&req.unit))
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
                continue;
            }
            // input check, still before anything merges: what the branch
            // says the unit was built from — its Built stamp's trace — must
            // be what mainline holds now. (It is checked again on the merged
            // tree; here it also keeps a change to the unit's own entry on
            // mainline from surfacing as a merge conflict in that file, when
            // it is simply text the unit has to be rebuilt from.)
            if let Some(outcome) = stamped_for_current_inputs(ml, &inst, unit, mlwt, &branch)? {
                out[i] = Some(outcome);
                continue;
            }
            candidates.push((i, branch));
        }

        let mut merged: Vec<Merged> = Vec::new();
        for (i, branch) in candidates {
            let req = &reqs[i];
            let before = ml.rev_parse("HEAD").map_err(CoreError::from)?;
            // merge --no-ff; never squash
            if let Err(e) = ml.merge_no_ff(
                &branch,
                &format!("sdd: integrate {} (c{})", req.unit, req.token_seq),
            ) {
                // conflict: list conflicted paths, abort, classify
                let conflicted = ml.conflicted_paths();
                ml.merge_abort().map_err(CoreError::from)?;
                ml.reset_hard(&before).map_err(CoreError::from)?;
                if conflicted.is_empty() {
                    return Err(e.into());
                }
                out[i] = Some(Outcome::OwnershipConflict { paths: conflicted });
                continue;
            }
            // input check: the work must say what it was built from (the
            // Built stamp's trace), and that must be what mainline holds now
            let merged_inst = self.ml.sdd_in(mlwt)?;
            let rejected = match merged_inst.unit(&req.unit) {
                None => Some(Outcome::RejectedFencing {
                    reason: format!("unit {} missing after merge", req.unit),
                }),
                Some(unit) => built_from_current_inputs(&merged_inst, unit)?,
            };
            if let Some(outcome) = rejected {
                ml.reset_hard(&before).map_err(CoreError::from)?;
                out[i] = Some(outcome);
                continue;
            }
            merged.push(Merged {
                i,
                branch,
                merge_sha: ml.rev_parse("HEAD").map_err(CoreError::from)?,
            });
        }
        Ok(merged)
    }

    /// Bootstrap, Build, the Verification tables and Lint, on the merged
    /// tree; failing rows of units outside the batch are judged on `base`
    /// (see the module doc).
    fn gate(
        &self,
        mlwt: &Path,
        base: &str,
        merged: &[Merged],
        reqs: &[IntegrateRequest],
    ) -> Result<Gate, CoreError> {
        let inst = self.ml.sdd_in(mlwt)?;
        let cmds = sdd_commands(&model::read_text(&inst.file)?);
        let verdict = |command: &str, ran: Ran| {
            if ran.interrupted || signals::interrupted() {
                Gate::Interrupted
            } else {
                Gate::Red {
                    command: command.to_string(),
                    output: ran.output,
                }
            }
        };
        for (step, cmd) in [("bootstrap", &cmds.bootstrap), ("build", &cmds.build)] {
            let Some(cmd) = cmd else { continue };
            self.board.step(step);
            let ran = gate::run_command_watched(cmd, mlwt, &self.board);
            if !ran.ok() {
                return Ok(verdict(cmd, ran));
            }
        }
        // the batch's own tables, then the dirty Verified units' — where
        // semantic conflicts between units surface; each distinct command
        // runs once. A unit held rotten is not checked again while it fails.
        let held = self.held_rotten();
        let slug = unit_graph::slug_of(&inst).to_string();
        let batch: Vec<String> = merged.iter().map(|m| reqs[m.i].unit.clone()).collect();
        let mut tables = rows::tables(&inst)?;
        for t in &mut tables {
            if held.contains(&t.unit) {
                t.verified = false;
            }
        }
        let (dirty, snapshot) = self.read_dirty(mlwt, &slug);
        let selection = rows::for_gate(&tables, &batch, &dirty, cmds.test.as_deref());
        let footprint = snapshot
            .and_then(|snap| Footprint::of(&snap, &slug, &selection.units, cmds.test.is_some()));
        let rows = selection.rows.clone();
        let own: BTreeSet<String> = tables
            .iter()
            .filter(|t| batch.contains(&t.unit))
            .flat_map(|t| t.rows.iter().cloned())
            .collect();
        let verified: BTreeMap<Node, Vec<String>> = tables
            .iter()
            .filter(|t| selection.units.contains(&t.unit) && !batch.contains(&t.unit))
            .map(|t| (Node::new(slug.clone(), t.unit.clone()), t.rows.clone()))
            .collect();
        self.board.step("rows");
        self.board.rows(rows.len());
        let results = gate::run_rows_watched(&rows, mlwt, self.gate_jobs, false, &self.board);
        let failed: Vec<(&String, Ran)> = rows
            .iter()
            .zip(results)
            .filter_map(|(cmd, ran)| ran.filter(|r| !r.ok()).map(|r| (cmd, r)))
            .collect();
        if let Some((cmd, ran)) = failed.iter().find(|(_, r)| r.interrupted) {
            return Ok(verdict(cmd, ran.clone()));
        }
        // a failing row of the batch's own units is the batch's
        if let Some((cmd, ran)) = failed.iter().find(|(cmd, _)| own.contains(*cmd)) {
            return Ok(verdict(cmd, ran.clone()));
        }
        if !failed.is_empty() {
            // the rest: run them on the base, and blame the batch only for
            // what it broke
            let commands: Vec<String> = failed.iter().map(|(c, _)| (*c).clone()).collect();
            let setup: Vec<String> = [&cmds.bootstrap, &cmds.build]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            self.board.step("rows-base");
            let judged = blame::judge(
                &commands,
                &verified,
                base,
                &setup,
                &self.base_checkout(),
                &ShellRunner,
            )?;
            let Judged::Done { verdict: v, .. } = judged else {
                return Ok(Gate::Interrupted);
            };
            if let Some(why) = &v.base_unusable {
                diag(&format!(
                    "note: failing rows could not be judged on the base ({why}); the batch is blamed for them"
                ));
            }
            if let Some((cmd, ran)) = failed.iter().find(|(c, _)| v.broken.contains(*c)) {
                return Ok(verdict(cmd, ran.clone()));
            }
            // every failing row fails on the base too: their units are
            // rotten, and the batch is judged on the rest
            let mut found: Vec<RottenRow> = Vec::new();
            for (node, unit_rows) in &v.rotten {
                let Some(table) = verified.get(node) else {
                    continue;
                };
                for row in unit_rows {
                    let output = failed
                        .iter()
                        .find(|(c, _)| *c == row)
                        .map(|(_, r)| r.output.clone())
                        .unwrap_or_default();
                    found.push(RottenRow {
                        unit: node.unit.clone(),
                        row_number: table.iter().position(|c| c == row).map_or(0, |i| i + 1),
                        row: row.clone(),
                        base: base.to_string(),
                        output,
                    });
                }
            }
            self.report_rotten(found);
        }
        if signals::interrupted() {
            return Ok(Gate::Interrupted);
        }
        if let Some(cmd) = &cmds.lint {
            self.board.step("lint");
            let ran = gate::run_command_watched(cmd, mlwt, &self.board);
            if !ran.ok() {
                return Ok(verdict(cmd, ran));
            }
        }
        let held = self.held_rotten();
        Ok(Gate::Green {
            rechecked: selection
                .units
                .into_iter()
                .filter(|u| !batch.contains(u) && !held.contains(u))
                .collect(),
            footprint,
        })
    }

    /// The Verified units of the SDD `slug` that must be checked again at
    /// the tree `checkout` holds: a path they are checked against changed
    /// since their last recorded check. When the repository cannot be read
    /// as a whole, or its graph has a cycle, every Verified unit is.
    fn dirty_units(&self, checkout: &Path, slug: &str) -> Vec<String> {
        self.read_dirty(checkout, slug).0
    }

    /// [`Integrator::dirty_units`], with the repository as it was read —
    /// `None` when it could not be read whole.
    fn read_dirty(&self, checkout: &Path, slug: &str) -> (Vec<String>, Option<Snapshot>) {
        let git = self.ml.primary.at(checkout);
        let everything = || match self.ml.sdd_in(checkout) {
            Ok(inst) => inst
                .units
                .iter()
                .filter(|u| u.meta.get_str_or("status", "Draft") == "Verified")
                .map(|u| u.unit.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        let snapshot = match Snapshot::load_resolvable(&git, checkout, "HEAD") {
            Ok((snap, _)) => snap,
            Err(e) => {
                diag(&format!(
                    "note: which units a change reaches could not be told ({e}); every Verified unit is checked"
                ));
                return (everything(), None);
            }
        };
        let cached = snapshot.dirty_with(
            &git,
            Mode::Changes,
            &mut self.diffs.lock().unwrap_or_else(|e| e.into_inner()),
        );
        let units = match cached {
            Ok(Cascade::Resolved(set)) => set.of_sdd(slug).map(|d| d.node.unit.clone()).collect(),
            Ok(Cascade::Cycles(_)) => everything(),
            Err(e) => {
                diag(&format!(
                    "note: which units a change reaches could not be told ({e}); every Verified unit is checked"
                ));
                everything()
            }
        };
        (units, Some(snapshot))
    }

    /// The §5 Test command on the merged tree: the whole suite, which
    /// Verification rows — each selecting a few tests — cannot stand in for.
    /// It gates as a regression check: a merge must not turn it from passing
    /// to failing. While it already fails at the head (no tests yet, tests
    /// awaiting their code, an earlier breakage) no merge is judged by it.
    /// Its state before the merges is taken from `memo`, or measured at
    /// `base` — which resets the checkout (`rebuilt`).
    fn suite(
        &self,
        mlwt: &Path,
        base: &str,
        memo: &mut GateMemo,
        rebuilt: &mut bool,
    ) -> Result<Suite, CoreError> {
        let inst = self.ml.sdd_in(mlwt)?;
        let cmds = sdd_commands(&model::read_text(&inst.file)?);
        let Some(test) = cmds.test.clone() else {
            return Ok(Suite::Fine(None));
        };
        self.board.step("test");
        let after = gate::run_command_watched(&test, mlwt, &self.board);
        if after.interrupted || signals::interrupted() {
            return Ok(Suite::Interrupted);
        }
        if after.ok() {
            return Ok(Suite::Fine(Some(true)));
        }
        let before = match self.suite_known(memo, base)? {
            Some(passed) => passed,
            None => {
                // measure at the base: the tree as it was before the merges
                self.board.step("test-base");
                self.ml.pristine(mlwt, base)?;
                *rebuilt = true;
                let mut passed = true;
                for cmd in [&cmds.bootstrap, &cmds.build, &Some(test.clone())]
                    .into_iter()
                    .flatten()
                {
                    let ran = gate::run_command_watched(cmd, mlwt, &self.board);
                    if ran.interrupted || signals::interrupted() {
                        return Ok(Suite::Interrupted);
                    }
                    if !ran.ok() {
                        passed = false;
                        break;
                    }
                }
                memo.suite = Some((base.to_string(), passed));
                passed
            }
        };
        if before {
            Ok(Suite::Regressed {
                command: test,
                output: format!(
                    "{}\n(the §5 Test command passed before this merge and fails after it)",
                    after.output
                ),
            })
        } else {
            diag(&format!(
                "note: the §5 Test command already fails at the {} head — integrations are not judged by it until it passes: {test}",
                self.ml.branch
            ));
            Ok(Suite::Fine(Some(false)))
        }
    }

    /// The Test command's result at `commit`, if `memo` holds one that
    /// still applies.
    fn suite_known(&self, memo: &GateMemo, commit: &str) -> Result<Option<bool>, CoreError> {
        match &memo.suite {
            Some((at, passed)) if benign(&self.ml, at, commit)? => Ok(Some(*passed)),
            _ => Ok(None),
        }
    }

    /// Steps 5–6: on the gated tree, stamp every unit Verified, append the
    /// journal events, regenerate the ledger, require a clean audit, commit
    /// and publish. If mainline moved since the gate started, the gate's
    /// verdict still stands when the commits in between moved nothing but
    /// protocol state (claims, renewals, releases, recorded questions): the
    /// merges are then replayed on the new head and published from there.
    #[allow(clippy::too_many_arguments)]
    fn finalize(
        &self,
        ml: &Git,
        mlwt: &Path,
        gated_base: &str,
        mut merged: Vec<Merged>,
        mut rebuild: bool,
        suite: Option<bool>,
        rechecked: &[String],
        footprint: Option<&Footprint>,
        reqs: &[IntegrateRequest],
        out: &mut [Option<Outcome>],
        memo: &mut GateMemo,
        before_publish: Option<&dyn Fn()>,
    ) -> Result<Final, CoreError> {
        let mut base = gated_base.to_string();
        let mut heals_left = 2;
        // The merges, stamps and audit are made in the lane's own checkout,
        // with no other writer waiting on them; only the move of mainline
        // itself takes this process's mainline turn. A publish that loses
        // the race to another lane's replays them on the new head — while
        // the commits in between leave the gate's verdict standing.
        for _ in 0..PUBLISH_TRIES {
            self.board.step("publish");
            if signals::interrupted() {
                return Ok(Final::Interrupted);
            }
            let head = self.ml.head()?;
            if head != base {
                // protocol state alone, or changes the gate never looked
                // at: the verdict stands, and the merges are replayed
                let untouched = self.benign(gated_base, &head)?
                    || footprint.is_some_and(|f| f.untouched(&self.ml.primary, gated_base, &head));
                if !untouched {
                    return Ok(Final::Redo(merged.iter().map(|m| m.i).collect()));
                }
                rebuild = true;
            }
            if rebuild {
                self.ml.pristine(mlwt, &head)?;
                let before: Vec<usize> = merged.iter().map(|m| m.i).collect();
                merged = self.assemble(ml, mlwt, &head, &before, reqs, out)?;
                if merged.len() != before.len() {
                    // a member dropped out: the gated tree is not this tree
                    self.ml.pristine(mlwt, &head)?;
                    return Ok(Final::Redo(merged.iter().map(|m| m.i).collect()));
                }
                base = head.clone();
            }
            // What is stamped, audited and published is the merged tree and
            // nothing else. Whatever the gate's commands left in the
            // checkout — files they wrote, tracked files they rewrote — goes
            // here, so the result never depends on whether the merges had
            // to be replayed.
            ml.reset_hard("HEAD").map_err(CoreError::from)?;
            ml.clean_fdq().map_err(CoreError::from)?;

            let inst = self.ml.sdd_in(mlwt)?;
            for m in &merged {
                let req = &reqs[m.i];
                let Some(unit) = inst.unit(&req.unit) else {
                    continue;
                };
                stamp_verified(&inst, unit, short(&m.merge_sha))?;
                let branch_sha = ml.rev_parse(&m.branch).map_err(CoreError::from)?;
                for (event, actor, commit) in [
                    ("built", &req.builder_holder, short(&branch_sha)),
                    ("verified", &req.integrator_id, short(&m.merge_sha)),
                ] {
                    append_event(
                        &inst,
                        Event {
                            event: event.to_string(),
                            unit: Some(req.unit.clone()),
                            actor: Some(actor.clone()),
                            claim_seq: Some(req.token_seq),
                            commit: Some(commit.to_string()),
                            ..Default::default()
                        },
                    )?;
                }
            }
            // the re-checks the gate made, on the tree it gated
            let gated = short(&ml.rev_parse("HEAD").map_err(CoreError::from)?).to_string();
            let held = self.held_rotten();
            let stamp = model::render_stamp(
                &UtcTime::now().format_date(),
                inst.meta.get_str_or("version", "0.0.0"),
                &gated,
            );
            for uid in rechecked.iter().filter(|u| !held.contains(*u)) {
                let Some(unit) = inst.unit(uid) else {
                    continue;
                };
                if unit.meta.get_str_or("status", "Draft") != "Verified" {
                    continue;
                }
                model::rewrite_unit_meta(&inst.handle(), unit, |inner| {
                    yamlish::set_scalar_in_place(inner, "verified", &stamp)
                })?;
                append_event(
                    &inst,
                    Event {
                        event: "verified".to_string(),
                        unit: Some(uid.clone()),
                        actor: Some(journal::RECHECK_ACTOR.to_string()),
                        commit: Some(gated.clone()),
                        ..Default::default()
                    },
                )?;
            }
            let stamped = self.ml.sdd_in(mlwt)?;
            ledger::regen(&stamped, &UtcTime::now().format_date())?;
            // audit --skip-rotten --journal: must be clean, run last on the
            // stamped tree (checks 1–3 on what the merges brought in; 5–7 on
            // the state about to be published)
            let (findings, _) = audit::run(
                &stamped,
                mlwt,
                &cmd::audit_options(
                    audit::Options {
                        skip_rotten: true,
                        ..Default::default()
                    },
                    mlwt,
                    &stamped,
                ),
                &ShRunner,
                UtcTime::now(),
            )?;
            // A lease this run does not hold that lapsed since the sync, or
            // a trace elsewhere that a change on mainline staled: mainline's
            // own state, and mechanically put right — not a verdict on this
            // batch. Put it right and publish from the new head.
            let batch: Vec<&str> = merged.iter().map(|m| reqs[m.i].unit.as_str()).collect();
            let elsewhere = |f: &audit::Finding| {
                matches!(f.check, 5 | 7) && !batch.contains(&f.subject.as_str())
            };
            if !findings.is_empty() && findings.iter().all(elsewhere) {
                if heals_left == 0 {
                    self.ml.pristine(mlwt, &base)?;
                    return Ok(Final::Redo(merged.iter().map(|m| m.i).collect()));
                }
                heals_left -= 1;
                match self.ml.heal(true, true) {
                    Ok(_) => {}
                    Err(e) if e.exit_code() == 5 => diag(&format!("note: {e}")),
                    Err(e) => return Err(e),
                }
                rebuild = true;
                continue;
            }
            if !findings.is_empty() {
                self.ml.pristine(mlwt, &base)?;
                return Ok(Final::Red {
                    command: "vitrify audit --skip-rotten --journal".to_string(),
                    output: findings
                        .iter()
                        .map(|f| format!("[check {}] {}: {}", f.check, f.subject, f.cause))
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
            }
            append_event(
                &stamped,
                Event {
                    event: "audit".to_string(),
                    actor: Some("audit".to_string()),
                    findings: Some(0),
                    // the suite passed on the gated tree: its new anchor
                    suite: (suite == Some(true)).then(|| journal::SUITE_PASS.to_string()),
                    commit: Some(gated.clone()),
                    ..Default::default()
                },
            )?;
            ml.add_all().map_err(CoreError::from)?;
            let label = merged
                .iter()
                .map(|m| format!("{} (c{})", reqs[m.i].unit, reqs[m.i].token_seq))
                .collect::<Vec<_>>()
                .join(", ");
            ml.commit(&format!("sdd: verify {label}"))
                .map_err(CoreError::from)?;
            let verify_sha = ml.rev_parse("HEAD").map_err(CoreError::from)?;

            if let Some(hook) = before_publish {
                hook();
            }
            if signals::interrupted() {
                return Ok(Final::Interrupted);
            }

            // publish every merge and the verify commit as one move; the
            // primary checkout follows in the same turn
            let turn = claim::tx_lock();
            if let Some(landed) = self.ml.publish(&verify_sha, &base)? {
                self.ml.follow(&landed)?;
                drop(turn);
                if let Some(passed) = suite {
                    memo.suite = Some((verify_sha.clone(), passed));
                }
                let slug = layout::sdd_slug(&stamped);
                for m in &merged {
                    let req = &reqs[m.i];
                    // the merged attempt's worktree and branch are done
                    let wt = self
                        .ml
                        .layout
                        .unit_worktree(&slug, &req.unit, req.token_seq);
                    if wt.exists() {
                        let _ = self.ml.primary.worktree_remove(&wt);
                    }
                    self.ml
                        .primary
                        .delete_branch(&m.branch)
                        .map_err(CoreError::from)?;
                    out[m.i] = Some(Outcome::Verified {
                        merge_commit: m.merge_sha.clone(),
                        verify_commit: verify_sha.clone(),
                    });
                }
                return Ok(Final::Published);
            }
            drop(turn);
            diag(&format!(
                "note: lost publish race integrating {label} on {}; redoing from sync",
                self.ml.branch
            ));
            rebuild = true;
        }
        Ok(Final::Redo(merged.iter().map(|m| m.i).collect()))
    }

    /// [`benign`] for this integrator's repository.
    pub fn benign(&self, old: &str, new: &str) -> Result<bool, CoreError> {
        benign(&self.ml, old, new)
    }

    /// A re-check batch for `units`, which a change reached and whose
    /// dependencies have no work pending: in the lane's integration checkout
    /// at the mainline head (taken under its lock), their rows then the §5
    /// Test command ([`recheck::run`]). The passes are recorded as one
    /// re-check commit; failing rows make their units rotten, held out of
    /// the lane's later gates. No worker takes part.
    pub fn recheck(&self, units: &[String]) -> Result<Rechecked, CoreError> {
        let mlwt = self.checkout();
        let _idle = IdleOnDrop(&self.board);
        self.board.batch(units.to_vec());
        let mut out = Rechecked::default();
        let Some(_held) = self.lock_checkout()? else {
            out.interrupted = true;
            return Ok(out);
        };
        self.board.step("recheck");
        let head = self.ml.sync_checkout(&mlwt)?;
        let report = recheck::run(
            &self.ml.primary.at(&mlwt),
            &head,
            &self.ml.sdd_rel,
            units,
            &PoolRunner(self.gate_jobs),
        )?;
        out.commit = report.commit.clone();
        if report.interrupted || signals::interrupted() {
            out.interrupted = true;
            return Ok(out);
        }
        if let Some(b) = &report.broken {
            out.broken = Some((b.command.clone(), b.output.clone()));
            return Ok(out);
        }
        out.suite_ran = report.suite.is_some();
        let inst = self.ml.sdd_in(&mlwt)?;
        // the verdict goes out before the passes are published
        self.report_rotten(rotten_of(&inst, &report)?);
        out.rotten = self.take_rotten();
        let edits = report.edits();
        out.passed = edits.verified.clone();
        out.recorded = record_recheck(&self.ml, &edits);
        Ok(out)
    }

    /// The mainline head as the termination report needs it: the audit
    /// (checks 1–3 and 5–7), the rows of the units a change reached since
    /// their last recorded check, and — when `want_suite` — the §5 Test
    /// command, unless its recorded pass still holds. Units held rotten are
    /// not run again.
    pub fn check_head(
        &self,
        memo: &mut GateMemo,
        want_suite: bool,
    ) -> Result<HeadCheck, CoreError> {
        let mlwt = self.checkout();
        let _idle = IdleOnDrop(&self.board);
        self.board.batch(Vec::new());
        let Some(_held) = self.lock_checkout()? else {
            return Ok(HeadCheck {
                head: String::new(),
                findings: Vec::new(),
                broken: None,
                suite: None,
                interrupted: true,
            });
        };
        self.board.step("head-check");
        let head = self.ml.sync_checkout(&mlwt)?;
        let inst = self.ml.sdd_in(&mlwt)?;
        let cmds = sdd_commands(&model::read_text(&inst.file)?);
        let mut check = HeadCheck {
            head: head.clone(),
            findings: Vec::new(),
            broken: None,
            suite: None,
            interrupted: false,
        };
        let slug = unit_graph::slug_of(&inst).to_string();
        let held = self.held_rotten();
        let dirty: Vec<String> = self
            .dirty_units(&mlwt, &slug)
            .into_iter()
            .filter(|u| !held.contains(u))
            .collect();
        let tables = rows::tables(&inst)?;
        let selection = rows::for_preflight(&tables, &dirty);
        let suite_known = match self.suite_known(memo, &head)? {
            Some(passed) => Some(passed),
            None => self.suite_recorded(&mlwt, &slug).then_some(true),
        };
        let run_suite = want_suite && cmds.test.is_some() && suite_known.is_none();
        // commands are about to run here: the environment first
        if !selection.rows.is_empty() || run_suite {
            for cmd in [&cmds.bootstrap, &cmds.build].into_iter().flatten() {
                let ran = gate::run_command_watched(cmd, &mlwt, &self.board);
                if ran.interrupted || signals::interrupted() {
                    check.interrupted = true;
                    return Ok(check);
                }
                if !ran.ok() {
                    check.broken = Some((cmd.clone(), ran.output));
                    return Ok(check);
                }
            }
        }
        let (mut findings, _) = audit::run(
            &inst,
            &mlwt,
            &cmd::audit_options(
                audit::Options {
                    skip_rotten: true,
                    ..Default::default()
                },
                &mlwt,
                &inst,
            ),
            &ShRunner,
            UtcTime::now(),
        )?;
        if !selection.rows.is_empty() {
            self.board.step("rows");
            self.board.rows(selection.rows.len());
            let results =
                gate::run_rows_watched(&selection.rows, &mlwt, self.gate_jobs, false, &self.board);
            if signals::interrupted() {
                check.interrupted = true;
                return Ok(check);
            }
            for t in tables.iter().filter(|t| selection.units.contains(&t.unit)) {
                for (row, ran) in selection.rows.iter().zip(&results) {
                    if t.rows.contains(row)
                        && let Some(ran) = ran
                        && !ran.ok()
                    {
                        findings.push(audit::Finding {
                            check: 4,
                            subject: t.unit.clone(),
                            cause: format!("Verification row fails: `{row}` (exit {})", ran.code),
                            remediation: format!(
                                "run `{row}` in a checkout of the head and fix what it reports"
                            ),
                        });
                    }
                }
            }
        }
        check.findings = findings;

        if want_suite && let Some(test) = &cmds.test {
            check.suite = Some(match suite_known {
                Some(passed) => (test.clone(), passed, String::new()),
                None => {
                    self.board.step("test");
                    let ran = gate::run_command_watched(test, &mlwt, &self.board);
                    if ran.interrupted || signals::interrupted() {
                        check.interrupted = true;
                        return Ok(check);
                    }
                    memo.suite = Some((head.clone(), ran.ok()));
                    (test.clone(), ran.ok(), ran.output)
                }
            });
        }
        Ok(check)
    }

    /// Whether the SDD `slug`'s journal records a §5 Test pass that still
    /// holds at the tree `checkout` holds.
    fn suite_recorded(&self, checkout: &Path, slug: &str) -> bool {
        let git = self.ml.primary.at(checkout);
        match Snapshot::load_resolvable(&git, checkout, "HEAD").and_then(|(s, _)| {
            s.dirty_with(
                &git,
                Mode::Changes,
                &mut self.diffs.lock().unwrap_or_else(|e| e.into_inner()),
            )
        }) {
            Ok(Cascade::Resolved(set)) => set
                .suite(slug)
                .is_some_and(|s| s.anchor.is_some() && s.dirty.is_none()),
            _ => false,
        }
    }
}

/// What a gate checked or read, beyond protocol state: the units whose
/// rows ran and everything they depend on, the SDD's unmanaged files, the
/// paths outside every scope it takes as input, and — when it ran the §5
/// Test command — the SDD's whole scope. A commit that changed nothing of
/// it cannot change the gate's verdict.
#[derive(Debug, Clone)]
pub struct Footprint {
    /// The gated SDD's slug.
    slug: String,
    /// The units whose rows ran, and every unit they depend on, in any SDD.
    checked: BTreeSet<Node>,
    /// The §5 Test command ran: every file in the SDD's scope was read.
    suite: bool,
    ownership: Ownership,
    spec: SpecFiles,
}

impl Footprint {
    /// The footprint of a gate over `units` (the units whose rows ran) of
    /// the SDD `slug`, read from `snap`. `None` when the graph has no
    /// closures to give (a cycle).
    fn of(snap: &Snapshot, slug: &str, units: &[String], suite: bool) -> Option<Footprint> {
        let resolution = snap.resolved.as_ref().ok()?;
        let mut checked: BTreeSet<Node> = BTreeSet::new();
        for unit in units {
            let node = Node::new(slug, unit.as_str());
            checked.extend(resolution.closure(&node).into_iter().cloned());
            checked.insert(node);
        }
        Some(Footprint {
            slug: slug.to_string(),
            checked,
            suite,
            ownership: snap.ownership.clone(),
            spec: snap.spec.clone(),
        })
    }

    /// Whether `path` is something the gate checked or read.
    fn reads(&self, path: &str) -> bool {
        match self.ownership.classify(path) {
            Owner::Spec(s) => s == self.slug,
            Owner::Unit(n) => self.checked.contains(&n) || (self.suite && n.slug == self.slug),
            Owner::Unmanaged(s) => s == self.slug,
            Owner::Outside => self.ownership.takes_outside(&self.slug, path),
        }
    }

    /// Whether `old..new` changed, beyond protocol state, nothing the gate
    /// checked or read (`new` descending from `old`).
    pub fn untouched(&self, git: &Git, old: &str, new: &str) -> bool {
        if old == new {
            return true;
        }
        if !git.is_ancestor(old, new).unwrap_or(false) {
            return false;
        }
        match dirty::substantive_changes(git, old, new, &self.spec) {
            Ok(paths) => !paths.iter().any(|p| self.reads(p)),
            Err(_) => false,
        }
    }
}

/// What a lane's re-check batch found.
#[derive(Debug, Clone, Default)]
pub struct Rechecked {
    /// The commit checked.
    pub commit: String,
    /// The units whose rows all passed.
    pub passed: Vec<String>,
    /// Their passes reached mainline as a re-check commit.
    pub recorded: bool,
    /// Rows that failed: their units are rotten.
    pub rotten: Vec<RottenRow>,
    /// A §5 Bootstrap or Build that failed, so no row ran: (command, output).
    pub broken: Option<(String, String)>,
    /// The §5 Test command ran.
    pub suite_ran: bool,
    pub interrupted: bool,
}

/// Runs a re-check's commands through the gate pool, its rows side by side.
pub(crate) struct PoolRunner(pub usize);

impl recheck::Runner for PoolRunner {
    fn run(&self, cmd: &str, cwd: &Path) -> recheck::CommandResult {
        recheck::ShRunner.run(cmd, cwd)
    }

    fn run_rows(&self, rows: &[String], cwd: &Path) -> Vec<recheck::CommandResult> {
        gate::run_rows(rows, cwd, self.0, false)
            .into_iter()
            .map(|ran| match ran {
                Some(ran) => recheck::CommandResult {
                    passed: ran.ok(),
                    interrupted: ran.interrupted,
                    output: ran.output,
                },
                None => recheck::CommandResult {
                    passed: false,
                    interrupted: true,
                    output: String::new(),
                },
            })
            .collect()
    }
}

/// The rows a re-check found failing, as rotten rows at the commit it
/// checked: nothing in a batch broke them, so something already on
/// mainline did — or they are flaky, which culprit finding sorts out.
pub(crate) fn rotten_of(
    inst: &Instance,
    report: &recheck::Report,
) -> Result<Vec<RottenRow>, CoreError> {
    let tables = rows::tables(inst)?;
    let mut out = Vec::new();
    for u in report.units.iter().filter(|u| !u.passed) {
        let table = tables
            .iter()
            .find(|t| t.unit == u.unit)
            .map(|t| t.rows.as_slice())
            .unwrap_or_default();
        for f in &u.failures {
            out.push(RottenRow {
                unit: u.unit.clone(),
                row_number: table
                    .iter()
                    .position(|r| r == &f.command)
                    .map_or(0, |i| i + 1),
                row: f.command.clone(),
                base: report.commit.clone(),
                output: f.output.clone(),
            });
        }
    }
    Ok(out)
}

/// Record a re-check's passes on mainline as one commit — fresh stamps,
/// `verified` events by `recheck`, `suite: "pass"` — published like any
/// other. `false` when there was nothing to record or it could not be.
pub(crate) fn record_recheck(ml: &Mainline, edits: &recheck::Edits) -> bool {
    if edits.is_empty() {
        return false;
    }
    let message = if edits.verified.is_empty() {
        format!("sdd: re-check the suite at {}", edits.commit)
    } else {
        format!(
            "sdd: re-check {} at {}",
            edits.verified.join(", "),
            edits.commit
        )
    };
    let published = ml.publish_tx(&message, &|tx| {
        let inst = ml.sdd_in(tx)?;
        let findings = recheck::apply(&inst, tx, edits)?;
        if !findings.is_empty() {
            diag(&format!(
                "note: the re-check was not recorded — the audit found {} finding(s) after it",
                findings.len()
            ));
            return Ok(false);
        }
        Ok(true)
    });
    match published {
        Ok(commit) => commit.is_some(),
        Err(e) => {
            // the checks still passed; only their record is missing
            diag(&format!("note: the re-check was not recorded: {e}"));
            false
        }
    }
}

/// Where the repository's SDDs keep their files, read from the primary
/// checkout: what tells protocol state from everything else. The lane's own
/// SDD is always among them, whatever its file is called.
fn spec_files(ml: &Mainline) -> SpecFiles {
    let (mut sdds, _) = cmd::loaded(&ml.primary.dir, None);
    if let Ok(own) = ml.sdd_in(&ml.primary.dir) {
        sdds.push(own);
    }
    SpecFiles::of(&ml.primary.dir, &sdds)
}

/// Whether `old..new` moved nothing but protocol state — journals,
/// ledgers, decision and change logs, the vitrify-managed fields of unit
/// entries, in any SDD of the repository. Such commits — claims, renewals,
/// releases, recorded questions — change nothing a build, a test or a lint
/// reads, so what a gate established at `old` holds at `new`.
pub fn benign(ml: &Mainline, old: &str, new: &str) -> Result<bool, CoreError> {
    if old == new {
        return Ok(true);
    }
    Ok(substantive(ml, old, new)?.is_some_and(|paths| paths.is_empty()))
}

/// The paths `old..new` changed beyond protocol state
/// ([`dirty::substantive_changes`]), `None` when `new` does not descend
/// from `old`; remembered: every lane asks the same of each new head, and
/// the answer is the same for all of them.
fn substantive(ml: &Mainline, old: &str, new: &str) -> Result<Option<Vec<String>>, CoreError> {
    type Memo = Vec<((String, String), Option<Vec<String>>)>;
    static SEEN: std::sync::Mutex<Memo> = std::sync::Mutex::new(Vec::new());
    let key = (old.to_string(), new.to_string());
    if let Some((_, paths)) = SEEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| *k == key)
    {
        return Ok(paths.clone());
    }
    let paths = if ml.primary.is_ancestor(old, new).map_err(CoreError::from)? {
        Some(dirty::substantive_changes(
            &ml.primary,
            old,
            new,
            &spec_files(ml),
        )?)
    } else {
        None
    };
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    seen.push((key, paths.clone()));
    if seen.len() > 64 {
        seen.remove(0);
    }
    Ok(paths)
}

/// Whether `old..new` changed the SDD's own text — anything under its root
/// beyond protocol state. Only then can a recorded trace have gone stale.
pub fn spec_moved(ml: &Mainline, old: &str, new: &str) -> Result<bool, CoreError> {
    if old == new {
        return Ok(false);
    }
    let root = cmd::sdd_arg(&ml.sdd_rel);
    let under = |p: &str| root == "." || p == root || p.starts_with(&format!("{root}/"));
    Ok(match substantive(ml, old, new)? {
        Some(paths) => paths.iter().any(|p| under(p)),
        None => true,
    })
}

fn short(sha: &str) -> &str {
    &sha[..7.min(sha.len())]
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

/// The input check before the merge: `None` when the unit's entry on
/// `branch` is stamped Built with a trace equal to what its inputs hash to
/// at the head (`inst`, loaded from the checkout at `root`). The write-set
/// check has passed, so the branch changed nothing of that entry but the
/// handoff fields.
fn stamped_for_current_inputs(
    ml: &Git,
    inst: &Instance,
    unit: &model::Unit,
    root: &Path,
    branch: &str,
) -> Result<Option<Outcome>, CoreError> {
    let spec_file = WriteSet::of(inst, unit, root)?.spec_file;
    let stamped = ml
        .show_text(branch, &spec_file)
        .map_err(CoreError::from)?
        .and_then(|text| unit_meta_in(inst.shape, &text, &unit.unit))
        .unwrap_or_default();
    judge_stamp(inst, unit, &stamped)
}

/// The input check: `None` when the merged unit is stamped Built with a
/// trace equal to what its inputs hash to now.
fn built_from_current_inputs(
    merged: &Instance,
    unit: &model::Unit,
) -> Result<Option<Outcome>, CoreError> {
    judge_stamp(merged, unit, &unit.meta)
}

/// `stamped` (a version of `unit`'s front matter) against the unit's inputs
/// as `inst` holds them.
fn judge_stamp(
    inst: &Instance,
    unit: &model::Unit,
    stamped: &model::Meta,
) -> Result<Option<Outcome>, CoreError> {
    let merged = inst;
    let status = stamped.get_str_or("status", "Draft");
    let recorded = stamped.get_map("trace").cloned().unwrap_or_default();
    if status != "Built" || recorded.is_empty() {
        return Ok(Some(Outcome::RejectedNotBuilt {
            reason: format!(
                "the work does not stamp {} Built (status {status}{}) — `vitrify trace record --stamp built` is the handoff",
                unit.unit,
                if recorded.is_empty() {
                    ", no trace"
                } else {
                    ""
                }
            ),
        }));
    }
    let (current, problems) = trace::compute(merged, unit)?;
    if !problems.is_empty() {
        // its reading set no longer resolves (a consumed contract gone, a
        // pin's authority moved): the work was built from what is no more,
        // a verdict on this unit alone
        return Ok(Some(Outcome::RejectedStaleInputs {
            keys: problems.iter().map(|p| p.to_string()).collect(),
        }));
    }
    let mismatches = trace::verify(&recorded, &current);
    if mismatches.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Outcome::RejectedStaleInputs {
            keys: mismatches.into_iter().map(|m| m.key).collect(),
        }))
    }
}

/// The write-set check (schema §13.3): classify every path the branch changed
/// since it forked from `base` against the unit's write-set as defined at
/// `base`. Empty = the work stays inside what the unit may write.
fn write_set_violations(
    ml: &Git,
    inst: &Instance,
    unit: &model::Unit,
    root: &Path,
    base: &str,
    branch: &str,
) -> Result<Vec<Violation>, CoreError> {
    let ws = WriteSet::of(inst, unit, root)?;
    let fork = ml.merge_base(base, branch).map_err(CoreError::from)?;
    let changed = ml.changed_paths(&fork, branch).map_err(CoreError::from)?;
    let spec_file = ws.spec_file.clone();
    let versions = || {
        (
            ml.show_text(&fork, &spec_file).ok().flatten(),
            ml.show_text(branch, &spec_file).ok().flatten(),
        )
    };
    Ok(ws.violations(&changed, versions))
}

/// `trace record --stamp verified --commit <sha>` via core (one hashing
/// implementation; this is wiring, not a second recipe).
fn stamp_verified(inst: &Instance, unit: &model::Unit, commit: &str) -> Result<(), CoreError> {
    let (trace_map, problems) = trace::compute(inst, unit)?;
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
    let stamp = model::render_stamp(
        &UtcTime::now().format_date(),
        inst.meta.get_str_or("version", "0.0.0"),
        commit,
    );
    model::rewrite_unit_meta(&inst.handle(), unit, |inner| {
        let b = yamlish::set_scalar_in_place(inner, "status", "Verified");
        let b = yamlish::set_scalar_in_place(&b, "verified", &stamp);
        let b = yamlish::set_scalar_in_place(&b, "claimed_by", "—");
        let b = yamlish::set_scalar_in_place(&b, "claim_expires", "—");
        yamlish::set_map_in_place(&b, "trace", &trace_map)
    })?;
    Ok(())
}
