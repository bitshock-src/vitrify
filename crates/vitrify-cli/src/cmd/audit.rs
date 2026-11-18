//! `vitrify audit` — the seven checks. Report-only by default;
//! `--fix-leases` / `--apply-stale` edit the current worktree and leave
//! committing to the caller. Check 4 runs the SDD's own Verification
//! commands through [`crate::gate`] — the one sanctioned shell use.

use std::path::Path;

use vitrify_core::audit::{self, Finding, Options, StateEdit};
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{self, Instance, SddHandle, UtcTime};
use vitrify_core::{CoreError, ledger, yamlish};

use crate::gate::MemoRunner;
use crate::out::{J, diag, emit};

pub struct Flags {
    pub json: bool,
    pub fix_leases: bool,
    pub apply_stale: bool,
    pub skip_rotten: bool,
    pub journal: bool,
    pub sdd: Option<String>,
    /// Every SDD of the repository.
    pub all: bool,
}

fn append_event(
    inst: &Instance,
    event: &str,
    unit: Option<&str>,
    claim_seq: Option<i64>,
    cause: Option<&str>,
    findings: Option<i64>,
) -> Result<(), CoreError> {
    let handle = inst.handle();
    let (events, _) = journal::read(&handle)?;
    let e = Event {
        seq: events.last().map(|e| e.seq).unwrap_or(0) + 1,
        at: UtcTime::now().format(),
        event: event.to_string(),
        unit: unit.map(str::to_string),
        actor: Some("audit".to_string()),
        claim_seq,
        cause: cause.map(str::to_string),
        findings,
        sdd: inst.meta.get_str_or("version", "0.0.0").to_string(),
        ..Default::default()
    };
    journal::append_line(&handle, &journal::render_line(&e))?;
    Ok(())
}

/// Apply the audit's state edits to the working tree — lapsed leases first,
/// then Stale marks — appending one journal event each.
/// Returns (leases released, units marked Stale). The caller regenerates the
/// ledger and commits.
pub fn apply_edits(inst: &Instance, edits: &[StateEdit]) -> Result<(usize, usize), CoreError> {
    let handle = inst.handle();
    let mut fixed_leases = 0usize;
    let mut marked_stale = 0usize;
    for edit in edits {
        if let StateEdit::FixLease { unit, claim_seq } = edit {
            let Some(u) = inst.unit(unit).cloned() else {
                continue;
            };
            model::rewrite_unit_meta(&handle, &u, |inner| {
                let b = yamlish::set_scalar_in_place(inner, "status", "Ready");
                let b = yamlish::set_scalar_in_place(&b, "claimed_by", "—");
                yamlish::set_scalar_in_place(&b, "claim_expires", "—")
            })?;
            append_event(inst, "release", Some(unit), Some(*claim_seq), None, None)?;
            fixed_leases += 1;
        }
    }
    for edit in edits {
        if let StateEdit::ApplyStale { unit, cause } = edit {
            let Some(u) = inst.unit(unit).cloned() else {
                continue;
            };
            model::rewrite_unit_meta(&handle, &u, |inner| {
                yamlish::set_scalar_in_place(inner, "status", "Stale")
            })?;
            append_event(inst, "stale", Some(unit), None, Some(cause), None)?;
            marked_stale += 1;
        }
    }
    Ok((fixed_leases, marked_stale))
}

/// Run the seven checks over `inst`, writing nothing: the findings, and the
/// state edits `opts`' fixing flags ask for.
fn examine(
    root: &Path,
    inst: &Instance,
    opts: Options,
) -> Result<(Vec<Finding>, Vec<StateEdit>), CoreError> {
    let skip_rotten = opts.skip_rotten;
    let opts = super::audit_options(opts, root, inst);
    // check 4: every Verified unit's rows, each distinct command once,
    // run.gate_jobs at a time (one per core unless configured)
    let runner: Box<dyn audit::CommandRunner> = if skip_rotten {
        Box::new(crate::gate::ShRunner)
    } else {
        let cfg = super::load_config(&vitrify_core::config::Overrides::default())?;
        Box::new(MemoRunner::for_verified(
            inst,
            root,
            cfg.run.rows_at_once(),
        )?)
    };
    audit::run(inst, root, &opts, runner.as_ref(), UtcTime::now())
}

/// The audit's findings for one SDD, report-only: no fix is applied and
/// nothing is written. Check 4 runs unless `skip_rotten`.
pub fn findings(
    root: &Path,
    inst: &Instance,
    skip_rotten: bool,
) -> Result<Vec<Finding>, CoreError> {
    let opts = Options {
        skip_rotten,
        ..Default::default()
    };
    Ok(examine(root, inst, opts)?.0)
}

/// Audit one SDD: run the checks, apply what the fixing flags ask for,
/// append the `audit` event. Returns the findings as found, before any fix.
fn audit_one(root: &Path, sdd: &SddHandle, flags: &Flags) -> Result<Vec<Finding>, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let opts = Options {
        skip_rotten: flags.skip_rotten,
        fix_leases: flags.fix_leases,
        apply_stale: flags.apply_stale,
        ..Default::default()
    };
    let (findings, edits) = examine(root, &inst, opts)?;

    let (fixed_leases, marked_stale) = apply_edits(&inst, &edits)?;
    if fixed_leases > 0 || marked_stale > 0 {
        let fresh = Instance::load(inst.handle())?;
        ledger::regen(&fresh, &UtcTime::now().format_date())?;
    }
    if fixed_leases > 0 {
        diag(&format!(
            "note: {fixed_leases} lapsed lease(s) returned to Ready — commit this"
        ));
    }
    if marked_stale > 0 {
        diag(&format!(
            "note: {marked_stale} unit(s) marked Stale — commit this"
        ));
    }
    if flags.journal {
        append_event(
            &inst,
            "audit",
            None,
            None,
            None,
            Some(findings.len() as i64),
        )?;
    }
    Ok(findings)
}

fn report_json(findings: &[Finding]) -> Vec<(String, J)> {
    vec![
        ("clean".into(), J::Bool(findings.is_empty())),
        (
            "findings".into(),
            J::Arr(findings.iter().map(finding_json).collect()),
        ),
    ]
}

/// The report's lines: the clean line, or one per finding — each under
/// `name` when several SDDs are being reported on.
fn report_text(findings: &[Finding], name: Option<&str>) {
    let under = name.map(|n| format!("{n}: ")).unwrap_or_default();
    if findings.is_empty() {
        emit(&format!("{under}audit clean (7/7 checks)"));
    }
    for f in findings {
        emit(&format!(
            "{under}[check {}] {}: {} → {}",
            f.check, f.subject, f.cause, f.remediation
        ));
    }
}

pub fn run(flags: Flags) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, flags.sdd.as_deref(), flags.all, &super::cwd())?;
    if !flags.all {
        let findings = audit_one(&root, &sdds[0], &flags)?;
        if flags.json {
            emit(&J::Obj(report_json(&findings)).dumps_indent2());
        } else {
            report_text(&findings, None);
        }
        return Ok(if findings.is_empty() { 0 } else { 2 });
    }

    let mut reports: Vec<J> = Vec::new();
    let code = super::each(&root, &sdds, |sdd| {
        let findings = audit_one(&root, sdd, &flags)?;
        let name = super::label(sdd, &root);
        if flags.json {
            let mut pairs = vec![
                ("sdd".into(), J::s(name)),
                ("root".into(), J::s(super::shown(&root, &sdd.root))),
            ];
            pairs.extend(report_json(&findings));
            reports.push(J::Obj(pairs));
        } else {
            report_text(&findings, Some(&name));
        }
        Ok(if findings.is_empty() { 0 } else { 2 })
    });
    if flags.json {
        emit(&J::Arr(reports).dumps_indent2());
    }
    Ok(code)
}

fn finding_json(f: &Finding) -> J {
    J::Obj(vec![
        ("check".into(), J::Int(f.check as i64)),
        ("subject".into(), J::s(f.subject.clone())),
        ("cause".into(), J::s(f.cause.clone())),
        ("remediation".into(), J::s(f.remediation.clone())),
    ])
}
