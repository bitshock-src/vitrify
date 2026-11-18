//! The serial integrator: every Built unit that is ready, as one batch, in
//! the dedicated integration checkout. Fencing check first; then the
//! write-set check (the branch changed only what the unit may write); a
//! `--no-ff` merge per unit; the input check (the unit was built from the
//! spec text mainline holds now); one run of the full integration gate over
//! the merged tree; one integration commit carrying stamps, journal events,
//! ledger, and a clean audit; CAS/push publish with lost-race redo; never
//! leaves mainline failing. A red batch is integrated again one unit at a
//! time, so one bad unit never holds back the others.
//!
//! The gate: the SDD's §5 Bootstrap and Build, the Verification table of
//! each unit being integrated and of every unit already Verified (each
//! distinct command once, `run.gate_jobs` at a time), the §5 Lint command,
//! then the §5 Test command — the whole suite, which a merge must not turn
//! from passing to failing — and audit checks 1–3.

use std::path::{Path, PathBuf};

use vitrify_core::audit;
use vitrify_core::journal::Event;
use vitrify_core::model::{self, Instance, Shape, UtcTime};
use vitrify_core::writeset::{Violation, WriteSet};
use vitrify_core::{CoreError, ledger, trace, yamlish};

use super::mainline::{Mainline, append_event};
use super::progress::GateBoard;
use crate::cmd::{self, claim};
use crate::gate::{self, Ran, ShRunner};
use crate::git::Git;
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
    let mut out = SddCommands::default();
    let mut in_section = false;
    for line in core_text.lines() {
        if line.starts_with('#') {
            in_section = line.trim_start_matches('#').trim() == "Commands";
            continue;
        }
        if !in_section {
            continue;
        }
        let Some(rest) = line.trim_start().strip_prefix("- ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(':') else {
            continue;
        };
        let cmd = value
            .split('`')
            .nth(1)
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        match name.trim().to_lowercase().as_str() {
            "bootstrap" => out.bootstrap = cmd,
            "build" => out.build = cmd,
            "test" => out.test = cmd,
            "lint" => out.lint = cmd,
            _ => {}
        }
    }
    out
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
    /// A mainline commit at which every Verified unit's Verification table
    /// passed (a green gate, or a clean full audit).
    pub tables_green_at: Option<String>,
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
    Green,
    Red { command: String, output: String },
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

/// A held lock on the integration checkout: one integration per repository
/// at a time, across processes.
struct CheckoutLock(std::fs::File);

impl Drop for CheckoutLock {
    /// Unlock before the file closes, as a temp slot does (`gate::TmpSlot`):
    /// left to the close, the lock can outlive the integration while a child
    /// forked meanwhile (a worker, say) has yet to exec — and the next batch
    /// wait on it, told that another process is integrating.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Puts the board back to `idle` however the integration it covers ends:
/// an error, an early return or a panic leaves no stale step behind.
struct IdleOnDrop<'a>(&'a GateBoard);

impl Drop for IdleOnDrop<'_> {
    fn drop(&mut self) {
        self.0.idle();
    }
}

pub struct Integrator {
    pub ml: Mainline,
    /// Verification rows run at once.
    pub gate_jobs: usize,
    /// Where the integrator says which step and commands it is running.
    pub board: GateBoard,
}

impl Integrator {
    pub fn new(ml: Mainline, gate_jobs: usize) -> Integrator {
        Integrator {
            ml,
            gate_jobs: gate_jobs.max(1),
            board: GateBoard::off(),
        }
    }

    /// The integrator, reporting its progress to `board`.
    pub fn with_board(self, board: GateBoard) -> Integrator {
        Integrator { board, ..self }
    }

    fn checkout(&self) -> PathBuf {
        self.ml.layout.mainline()
    }

    /// Take the integration checkout. `None` = interrupted while waiting.
    fn lock_checkout(&self) -> Result<Option<CheckoutLock>, CoreError> {
        let path = self.ml.layout.mainline_lock();
        let io = |source| CoreError::Io {
            path: path.clone(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io)?;
        let mut told = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(CheckoutLock(file))),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if signals::interrupted() {
                        return Ok(None);
                    }
                    if !told {
                        self.board.step("lock");
                        diag(
                            "note: another vitrify process is integrating in this repository — waiting for the integration checkout",
                        );
                        told = true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(io(e)),
            }
        }
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
                this.ml.checkout_at(&mlwt, &base)?;
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
            match self.gate(&mlwt, &merged, reqs)? {
                Gate::Green => {}
                Gate::Red { command, output } => return red(self, out, memo, command, output),
                Gate::Interrupted => {
                    let _ = self.ml.checkout_at(&mlwt, &base);
                    interrupt(out, &members);
                    return Ok(());
                }
            }
            let mut rebuilt = false;
            let suite = match self.suite(&mlwt, &base, memo, &mut rebuilt)? {
                Suite::Fine(result) => result,
                Suite::Regressed { command, output } => {
                    return red(self, out, memo, command, output);
                }
                Suite::Interrupted => {
                    let _ = self.ml.checkout_at(&mlwt, &base);
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
                reqs,
                out,
                memo,
                before_publish,
            )? {
                Final::Published => return Ok(()),
                Final::Red { command, output } => return red(self, out, memo, command, output),
                Final::Interrupted => {
                    let _ = self.ml.checkout_at(&mlwt, &base);
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
    /// tree.
    fn gate(
        &self,
        mlwt: &Path,
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
        // the batch's own tables, then every Verified unit's — where
        // semantic conflicts between units surface; each distinct command
        // runs once
        let mut rows: Vec<String> = Vec::new();
        let batch: Vec<&str> = merged.iter().map(|m| reqs[m.i].unit.as_str()).collect();
        let mut add = |unit: &model::Unit| -> Result<(), CoreError> {
            for cmd in gate::verification_table(&inst, unit)? {
                if !rows.contains(&cmd) {
                    rows.push(cmd);
                }
            }
            Ok(())
        };
        for uid in &batch {
            if let Some(unit) = inst.unit(uid) {
                add(unit)?;
            }
        }
        for unit in &inst.units {
            if !batch.contains(&unit.unit.as_str())
                && unit.meta.get_str_or("status", "Draft") == "Verified"
            {
                add(unit)?;
            }
        }
        self.board.step("rows");
        self.board.rows(rows.len());
        let results = gate::run_rows_watched(&rows, mlwt, self.gate_jobs, true, &self.board);
        for (cmd, ran) in rows.iter().zip(results) {
            if let Some(ran) = ran
                && !ran.ok()
            {
                return Ok(verdict(cmd, ran));
            }
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
        Ok(Gate::Green)
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
                self.ml.checkout_at(mlwt, base)?;
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
        reqs: &[IntegrateRequest],
        out: &mut [Option<Outcome>],
        memo: &mut GateMemo,
        before_publish: Option<&dyn Fn()>,
    ) -> Result<Final, CoreError> {
        let mut base = gated_base.to_string();
        let mut heals_left = 2;
        for _ in 0..6 {
            self.board.step("publish");
            if signals::interrupted() {
                return Ok(Final::Interrupted);
            }
            // this process's other mainline writers wait their turn from here
            let turn = claim::tx_lock();
            let head = self.ml.head()?;
            if head != base {
                if !self.benign(gated_base, &head)? {
                    return Ok(Final::Redo(merged.iter().map(|m| m.i).collect()));
                }
                rebuild = true;
            }
            if rebuild {
                self.ml.checkout_at(mlwt, &head)?;
                let before: Vec<usize> = merged.iter().map(|m| m.i).collect();
                merged = self.assemble(ml, mlwt, &head, &before, reqs, out)?;
                if merged.len() != before.len() {
                    // a member dropped out: the gated tree is not this tree
                    self.ml.checkout_at(mlwt, &head)?;
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
                drop(turn);
                if heals_left == 0 {
                    self.ml.checkout_at(mlwt, &base)?;
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
                drop(turn);
                self.ml.checkout_at(mlwt, &base)?;
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

            // publish every merge and the verify commit as one move
            if let Some(landed) = self.ml.publish(&verify_sha, &base)? {
                self.ml.follow(&landed)?;
                drop(turn);
                memo.tables_green_at = Some(verify_sha.clone());
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

    /// The mainline head as the termination report needs it: the audit
    /// (check 4 skipped when `memo` shows every Verified table green on
    /// this tree) and, when `want_suite`, the §5 Test command.
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

        let tables_known = match &memo.tables_green_at {
            Some(at) => self.benign(at, &head)?,
            None => false,
        };
        let suite_known = self.suite_known(memo, &head)?;
        let run_suite = want_suite && cmds.test.is_some() && suite_known.is_none();
        // commands are about to run here: the environment first
        if !tables_known || run_suite {
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
        let options = |skip_rotten| {
            cmd::audit_options(
                audit::Options {
                    skip_rotten,
                    ..Default::default()
                },
                &mlwt,
                &inst,
            )
        };
        let (findings, _) = if tables_known {
            audit::run(&inst, &mlwt, &options(true), &ShRunner, UtcTime::now())?
        } else {
            self.board.rows(gate::verified_commands(&inst)?.len());
            let runner =
                gate::MemoRunner::for_verified_watched(&inst, &mlwt, self.gate_jobs, &self.board)?;
            audit::run(&inst, &mlwt, &options(false), &runner, UtcTime::now())?
        };
        if signals::interrupted() {
            check.interrupted = true;
            return Ok(check);
        }
        if findings.is_empty() {
            memo.tables_green_at = Some(head.clone());
        }
        check.findings = findings;

        if want_suite && let Some(test) = &cmds.test {
            check.suite = Some(match suite_known {
                Some(passed) => (test.clone(), passed, String::new()),
                None => {
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
}

/// Changed paths between two commits, each with whether it is protocol
/// state only: a journal, a ledger, a decision or change log, or a unit
/// entry that differs in nothing but its vitrify-managed fields.
fn classify_changes(ml: &Mainline, old: &str, new: &str) -> Result<Vec<(String, bool)>, CoreError> {
    let git = &ml.primary;
    // a single-file SDD is its own path, whatever the file is called
    let own = cmd::sdd_arg(&ml.sdd_rel);
    let mut bundles: Vec<(String, bool)> = Vec::new();
    let mut is_bundle = |dir: &str| -> bool {
        if let Some((_, known)) = bundles.iter().find(|(d, _)| d == dir) {
            return *known;
        }
        let core = if dir.is_empty() {
            "sdd.md".to_string()
        } else {
            format!("{dir}/sdd.md")
        };
        let known = git
            .show_text(new, &core)
            .ok()
            .flatten()
            .and_then(|t| model::front_matter(&t))
            .is_some_and(|m| m.has("sdd"));
        bundles.push((dir.to_string(), known));
        known
    };
    let both = |path: &str| -> Option<(String, String)> {
        Some((
            git.show_text(old, path).ok().flatten()?,
            git.show_text(new, path).ok().flatten()?,
        ))
    };
    let mut out = Vec::new();
    for path in git.changed_paths(old, new).map_err(CoreError::from)? {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        let state_only = if matches!(
            name,
            "journal.ndjson" | "ledger.md" | "decisions.md" | "changelog.md"
        ) {
            is_bundle(dir)
        } else if name.ends_with(".sdd.md") || path == own {
            both(&path).is_some_and(|(a, b)| single_state_masked(&a) == single_state_masked(&b))
        } else if (dir == "units" || dir.ends_with("/units"))
            && name.starts_with("U-")
            && name.ends_with(".md")
            && is_bundle(dir.trim_end_matches("units").trim_end_matches('/'))
        {
            both(&path).is_some_and(|(a, b)| {
                trace::mask_unit_fields(Shape::Bundle, &a, "", &trace::VOLATILE)
                    == trace::mask_unit_fields(Shape::Bundle, &b, "", &trace::VOLATILE)
            })
        } else {
            false
        };
        out.push((path, state_only));
    }
    Ok(out)
}

/// Whether `old..new` moved nothing but protocol state: journals, ledgers,
/// decision and change logs, and the vitrify-managed fields of unit
/// entries. Such commits — claims, renewals, releases, recorded questions —
/// change nothing a build, a test or a lint reads, so what a gate
/// established at `old` holds at `new`.
pub fn benign(ml: &Mainline, old: &str, new: &str) -> Result<bool, CoreError> {
    if old == new {
        return Ok(true);
    }
    if !ml.primary.is_ancestor(old, new).map_err(CoreError::from)? {
        return Ok(false);
    }
    Ok(classify_changes(ml, old, new)?
        .iter()
        .all(|(_, state_only)| *state_only))
}

/// Whether `old..new` changed the SDD's own text — anything under its root
/// beyond protocol state. Only then can a recorded trace have gone stale.
pub fn spec_moved(ml: &Mainline, old: &str, new: &str) -> Result<bool, CoreError> {
    if old == new {
        return Ok(false);
    }
    if !ml.primary.is_ancestor(old, new).map_err(CoreError::from)? {
        return Ok(true);
    }
    let root = cmd::sdd_arg(&ml.sdd_rel);
    let under = |p: &str| root == "." || p == root || p.starts_with(&format!("{root}/"));
    Ok(classify_changes(ml, old, new)?
        .iter()
        .any(|(path, state_only)| under(path) && !state_only))
}

fn short(sha: &str) -> &str {
    &sha[..7.min(sha.len())]
}

/// A single-file SDD's text with everything vitrify itself writes removed:
/// each unit's managed fields, the journal, the ledger, the decision and
/// change logs.
fn single_state_masked(text: &str) -> String {
    let mut t = text.to_string();
    let ids: Vec<String> = model::split_h3_regions(&t, "Work Units")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        if let Some(masked) = trace::mask_unit_fields(Shape::Single, &t, &id, &trace::VOLATILE) {
            t = masked;
        }
    }
    for heading in [
        "Journal",
        "State Ledger",
        "Decisions & Open Questions",
        "Changelog",
    ] {
        if let Some((s, e)) = model::section_span(&t, heading) {
            t = format!("{}{}", &t[..s], &t[e..]);
        }
    }
    t
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
        return Err(CoreError::refusal(
            4,
            problems
                .iter()
                .map(|p| format!("error: {p}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
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
