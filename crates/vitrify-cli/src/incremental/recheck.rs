//! Re-check batches: rows of dirty units, then the §5 Test command.
//!
//! A batch runs in a lane checkout made pristine at one commit — `reset
//! --hard` and `clean -ffdx`, so no file a previous command left behind can
//! decide a result — through an injected [`Runner`]: the §5 Bootstrap and
//! Build (a fresh tree has nothing built), the Verification rows of the
//! given units that are Verified at that commit (each distinct command once,
//! [`rows::for_recheck`]), then the §5 Test command. A unit passes when every
//! row of its table passed; the suite is judged on its own.
//!
//! Successes are recorded as they happen ([`Report::edits`], [`apply`]): for
//! each unit that passed, a `verified` journal event with actor
//! [`journal::RECHECK_ACTOR`] and the checked commit — the unit's new
//! anchor — and a fresh `verified:` stamp, a volatile field masked from
//! traces, so no trace moves; and, when the Test command passed,
//! `suite: "pass"` on the `audit` event the edits end with. A failing row
//! records nothing for its unit: failures are recomputed, never trusted. An
//! interrupted batch records nothing at all.

use std::path::Path;

use vitrify_core::audit::{self, Finding};
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{self, Instance, UtcTime};
use vitrify_core::{CoreError, ledger, yamlish};

use super::rows::{self, Commands};
use crate::cmd;
use crate::gate;
use crate::git::Git;

/// One command's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub passed: bool,
    /// Stopped because the run was interrupted, not because it failed.
    pub interrupted: bool,
    pub output: String,
}

/// Runs a batch's commands: one in a directory, or a list of rows. The rows
/// are handed over at once, so a runner with a pool of its own can run them
/// side by side; by default they run one after another.
pub trait Runner {
    fn run(&self, cmd: &str, cwd: &Path) -> CommandResult;

    /// Every row in `cwd`, results index-aligned with `rows`.
    fn run_rows(&self, rows: &[String], cwd: &Path) -> Vec<CommandResult> {
        rows.iter().map(|r| self.run(r, cwd)).collect()
    }
}

impl<F: Fn(&str, &Path) -> CommandResult> Runner for F {
    fn run(&self, cmd: &str, cwd: &Path) -> CommandResult {
        self(cmd, cwd)
    }
}

/// The plain runner: each command through `sh -c`, one at a time.
pub struct ShRunner;

impl Runner for ShRunner {
    fn run(&self, cmd: &str, cwd: &Path) -> CommandResult {
        let ran = gate::run_command(cmd, cwd);
        CommandResult {
            passed: ran.ok(),
            interrupted: ran.interrupted,
            output: ran.output,
        }
    }
}

/// A command that failed, with its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub command: String,
    pub output: String,
}

/// One unit of the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitResult {
    pub unit: String,
    /// Every row of its table passed.
    pub passed: bool,
    /// Its rows that failed, in table order.
    pub failures: Vec<Failure>,
}

/// The §5 Test command's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteResult {
    pub command: String,
    pub passed: bool,
    pub output: String,
}

/// What a batch found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The commit checked, in full.
    pub commit: String,
    /// The units checked, in the order given.
    pub units: Vec<UnitResult>,
    /// Units given that are not Verified at the commit: nothing to re-check.
    pub skipped: Vec<String>,
    /// The §5 Test command; `None` when the SDD has none, or it did not run.
    pub suite: Option<SuiteResult>,
    /// A §5 Bootstrap or Build that failed: no row ran.
    pub broken: Option<Failure>,
    /// The run was interrupted: nothing is recorded.
    pub interrupted: bool,
}

/// The protocol edits a batch earns ([`apply`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edits {
    /// The checked commit, short, as the journal records commits.
    pub commit: String,
    /// The units that passed, in batch order.
    pub verified: Vec<String>,
    /// The §5 Test command passed.
    pub suite_pass: bool,
}

impl Edits {
    /// Whether there is nothing to record.
    pub fn is_empty(&self) -> bool {
        self.verified.is_empty() && !self.suite_pass
    }
}

impl Report {
    /// What to record: every unit that passed, and the suite if it passed.
    /// Nothing when the batch was interrupted.
    pub fn edits(&self) -> Edits {
        if self.interrupted {
            return Edits {
                commit: short(&self.commit).to_string(),
                ..Default::default()
            };
        }
        Edits {
            commit: short(&self.commit).to_string(),
            verified: self
                .units
                .iter()
                .filter(|u| u.passed)
                .map(|u| u.unit.clone())
                .collect(),
            suite_pass: self.suite.as_ref().is_some_and(|s| s.passed),
        }
    }
}

fn short(sha: &str) -> &str {
    &sha[..7.min(sha.len())]
}

/// Re-check `units` of the SDD at `sdd_rel` (relative to the checkout) at
/// `commit`, in the lane checkout `lane`: reset it to `commit` and clean
/// it, run the §5 Bootstrap and Build, the units' rows, then the §5 Test
/// command, each through `runner`. Leaves the checkout holding whatever the
/// commands left; reset it before using it for anything else.
pub fn run(
    lane: &Git,
    commit: &str,
    sdd_rel: &Path,
    units: &[String],
    runner: &dyn Runner,
) -> Result<Report, CoreError> {
    let full = lane
        .resolve_commit(commit)
        .map_err(CoreError::from)?
        .ok_or_else(|| CoreError::refusal(1, format!("error: {commit} names no commit")))?;
    lane.reset_hard(&full).map_err(CoreError::from)?;
    lane.clean_all().map_err(CoreError::from)?;
    let inst = Instance::load(cmd::resolve_sdd_in(&lane.dir, sdd_rel)?)?;
    let tables = rows::tables(&inst)?;
    let commands = Commands::of(&inst)?;
    let selection = rows::for_recheck(&tables, units, commands.test.as_deref());

    let mut report = Report {
        commit: full,
        units: Vec::new(),
        skipped: units
            .iter()
            .filter(|u| !selection.units.contains(u))
            .cloned()
            .collect(),
        suite: None,
        broken: None,
        interrupted: false,
    };
    let failed_all = |report: &mut Report| {
        report.units = selection
            .units
            .iter()
            .map(|u| UnitResult {
                unit: u.clone(),
                passed: false,
                failures: Vec::new(),
            })
            .collect();
    };
    if selection.units.is_empty() && selection.test.is_none() {
        return Ok(report);
    }

    // a pristine tree has nothing built
    for cmd in [&commands.bootstrap, &commands.build].into_iter().flatten() {
        let ran = runner.run(cmd, &lane.dir);
        if ran.interrupted {
            report.interrupted = true;
            failed_all(&mut report);
            return Ok(report);
        }
        if !ran.passed {
            report.broken = Some(Failure {
                command: cmd.clone(),
                output: ran.output,
            });
            failed_all(&mut report);
            return Ok(report);
        }
    }

    let results = runner.run_rows(&selection.rows, &lane.dir);
    let result_of = |row: &str| -> Option<&CommandResult> {
        let i = selection.rows.iter().position(|r| r == row)?;
        results.get(i)
    };
    report.interrupted = results.iter().any(|r| r.interrupted);
    for unit in &selection.units {
        let table = tables
            .iter()
            .find(|t| &t.unit == unit)
            .map(|t| t.rows.as_slice())
            .unwrap_or_default();
        let mut failures = Vec::new();
        let mut passed = true;
        for row in table {
            match result_of(row) {
                Some(r) if r.passed => {}
                Some(r) => {
                    passed = false;
                    failures.push(Failure {
                        command: row.clone(),
                        output: r.output.clone(),
                    });
                }
                // a row the runner gave no result for is no pass
                None => passed = false,
            }
        }
        report.units.push(UnitResult {
            unit: unit.clone(),
            passed,
            failures,
        });
    }
    if report.interrupted {
        return Ok(report);
    }

    if let Some(test) = &selection.test {
        let ran = runner.run(test, &lane.dir);
        if ran.interrupted {
            report.interrupted = true;
            return Ok(report);
        }
        report.suite = Some(SuiteResult {
            command: test.clone(),
            passed: ran.passed,
            output: ran.output,
        });
    }
    Ok(report)
}

/// Append one event to `inst`'s journal, numbered after the last.
fn append_event(inst: &Instance, mut event: Event) -> Result<(), CoreError> {
    let handle = inst.handle();
    let (events, _) = journal::read(&handle)?;
    event.seq = events.last().map(|e| e.seq).unwrap_or(0) + 1;
    if event.at.is_empty() {
        event.at = UtcTime::now().format();
    }
    if event.sdd.is_empty() {
        event.sdd = inst.meta.get_str_or("version", "0.0.0").to_string();
    }
    journal::append_line(&handle, &journal::render_line(&event))?;
    Ok(())
}

/// Record `edits` in the SDD `inst`, loaded from the checkout at `root`
/// (one holding the checked commit's spec files, or a descendant whose
/// changes since are protocol state only): for each unit that passed and is
/// still Verified, a fresh `verified:` stamp and a `verified` event with
/// actor `recheck` and the checked commit; then the ledger, regenerated;
/// then the audit (check 4 skipped), which must be clean; then an `audit`
/// event carrying `suite: "pass"` when the Test command passed. Returns the
/// audit's findings: when there are any, the closing `audit` event is not
/// written, and the checkout should be reset rather than committed. Nothing
/// is written when `edits` is empty. Committing and publishing are the
/// caller's.
pub fn apply(inst: &Instance, root: &Path, edits: &Edits) -> Result<Vec<Finding>, CoreError> {
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    let stamp = model::render_stamp(
        &UtcTime::now().format_date(),
        inst.meta.get_str_or("version", "0.0.0"),
        &edits.commit,
    );
    let handle = inst.handle();
    for uid in &edits.verified {
        let Some(unit) = inst.unit(uid) else {
            continue;
        };
        if unit.meta.get_str_or("status", "Draft") != "Verified" {
            continue;
        }
        model::rewrite_unit_meta(&handle, unit, |inner| {
            yamlish::set_scalar_in_place(inner, "verified", &stamp)
        })?;
        append_event(
            inst,
            Event {
                event: "verified".to_string(),
                unit: Some(uid.clone()),
                actor: Some(journal::RECHECK_ACTOR.to_string()),
                commit: Some(edits.commit.clone()),
                ..Default::default()
            },
        )?;
    }
    let stamped = Instance::load(handle)?;
    ledger::regen(&stamped, &UtcTime::now().format_date())?;
    let (findings, _) = audit::run(
        &stamped,
        root,
        &cmd::audit_options(
            audit::Options {
                skip_rotten: true,
                ..Default::default()
            },
            root,
            &stamped,
        ),
        &gate::ShRunner,
        UtcTime::now(),
    )?;
    if !findings.is_empty() {
        return Ok(findings);
    }
    append_event(
        &stamped,
        Event {
            event: "audit".to_string(),
            actor: Some("audit".to_string()),
            findings: Some(0),
            suite: edits.suite_pass.then(|| journal::SUITE_PASS.to_string()),
            commit: Some(edits.commit.clone()),
            ..Default::default()
        },
    )?;
    Ok(Vec::new())
}
