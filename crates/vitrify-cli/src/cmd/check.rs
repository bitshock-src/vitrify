//! `vitrify check` — every read-only consistency check vitrify has, over one
//! SDD or all of them: lint, the audit, the journal replay, the ledger check
//! and the pin states, each finding with its source and its remediation;
//! open questions as notes. Writes nothing.

use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::config::Overrides;
use vitrify_core::journal as core_journal;
use vitrify_core::model::{Instance, SddHandle, is_empty_marker};

use crate::out::{J, emit};

pub struct Flags {
    pub sdd: Option<String>,
    /// Every SDD of the repository.
    pub all: bool,
    /// Skip audit check 4 (re-running Verified units' Verification tables).
    pub skip_rotten: bool,
    pub json: bool,
}

/// One finding, as `check` reports it.
struct Item {
    /// `lint`, `audit`, `journal`, `ledger` or `pin`.
    source: &'static str,
    /// The check's own name within its source (`L4`, `5`), where it has one.
    check: Option<String>,
    subject: String,
    cause: String,
    remediation: String,
}

/// What `check` found in one SDD.
struct Report {
    sdd: String,
    root: String,
    findings: Vec<Item>,
    notes: Vec<String>,
    code: u8,
}

impl Report {
    fn add(
        &mut self,
        source: &'static str,
        check: Option<String>,
        (subject, cause, remediation): (String, String, String),
    ) {
        self.code = self.code.max(match source {
            "journal" | "ledger" => 3,
            _ => 2,
        });
        self.findings.push(Item {
            source,
            check,
            subject,
            cause,
            remediation,
        });
    }

    fn json(&self) -> J {
        J::Obj(vec![
            ("sdd".into(), J::s(self.sdd.clone())),
            ("root".into(), J::s(self.root.clone())),
            ("clean".into(), J::Bool(self.findings.is_empty())),
            (
                "findings".into(),
                J::Arr(
                    self.findings
                        .iter()
                        .map(|x| {
                            J::Obj(vec![
                                ("source".into(), J::s(x.source)),
                                ("check".into(), x.check.clone().map_or(J::Null, J::s)),
                                ("subject".into(), J::s(x.subject.clone())),
                                ("cause".into(), J::s(x.cause.clone())),
                                ("remediation".into(), J::s(x.remediation.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "notes".into(),
                J::Arr(self.notes.iter().map(|n| J::s(n.clone())).collect()),
            ),
        ])
    }

    fn emit_text(&self) {
        for x in &self.findings {
            let source = match &x.check {
                Some(check) => format!("{} {check}", x.source),
                None => x.source.to_string(),
            };
            emit(&format!(
                "{}: [{source}] {}: {} → {}",
                self.sdd, x.subject, x.cause, x.remediation
            ));
        }
        for n in &self.notes {
            emit(&format!("{}: note: {n}", self.sdd));
        }
    }
}

/// A unit has build state: a status beyond Draft or Ready, a claim, or a
/// recorded trace.
fn has_build_state(inst: &Instance) -> bool {
    inst.units.iter().any(|u| {
        !matches!(u.meta.get_str_or("status", "Draft"), "Draft" | "Ready")
            || !is_empty_marker(u.meta.get_str_or("claimed_by", "").trim())
            || !is_empty_marker(u.meta.get_str_or("claim_expires", "").trim())
            || u.meta.get_map("trace").is_some_and(|m| !m.is_empty())
    })
}

fn check_one(
    root: &Path,
    sdd: &SddHandle,
    budget: i64,
    skip_rotten: bool,
) -> Result<Report, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let at = super::shown(root, &inst.root);
    let mut report = Report {
        sdd: super::label(sdd, root),
        root: at.clone(),
        findings: Vec::new(),
        notes: Vec::new(),
        code: 0,
    };
    let (events, _) = core_journal::read(&inst.handle())?;
    let has_journal = !events.is_empty();
    let replayed = super::journal::replayed(&inst)?;

    // lint: --built once the SDD has build state. L9 is the pin source's.
    let lint = if has_journal || has_build_state(&inst) {
        super::lint::pre_run_findings(&inst, root, budget)?
    } else {
        super::lint::check_all(&inst, root, budget)?
    };
    for x in lint.into_iter().filter(|x| x.check != "L9") {
        report.add(
            "lint",
            Some(x.check.to_string()),
            (x.subject, x.cause, x.remediation),
        );
    }

    // audit: check 6's replay findings are the journal source's
    for x in super::audit::findings(root, &inst, skip_rotten)? {
        let replay_finding =
            x.check == 6 && (x.subject == "journal" || x.cause.starts_with("journal projects "));
        if replayed.is_some() && replay_finding {
            continue;
        }
        report.add(
            "audit",
            Some(x.check.to_string()),
            (x.subject, x.cause, x.remediation),
        );
    }

    if let Some(r) = &replayed {
        for x in r.findings() {
            report.add("journal", None, x);
        }
    }

    if let Some((subject, cause)) = super::ledger::mismatch(&inst, has_journal)? {
        report.add(
            "ledger",
            None,
            (subject, cause, format!("vitrify ledger regen --sdd {at}")),
        );
    }

    for x in super::lint::pin_findings(&inst, &super::peers(root), &at)? {
        report.add("pin", None, x);
    }

    for (q, blocks) in super::questions::open(sdd)? {
        report.notes.push(if blocks.is_empty() {
            format!("{q} is open")
        } else {
            format!("{q} is open (blocks {})", blocks.join(", "))
        });
    }
    Ok(report)
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

pub fn run(flags: Flags) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, flags.sdd.as_deref(), flags.all, &super::cwd())?;
    let budget = super::load_config(&Overrides::default())?
        .lint
        .readset_budget_bytes;

    let mut reports: Vec<Report> = Vec::new();
    let code = if flags.all {
        super::each(&root, &sdds, |sdd| {
            let report = check_one(&root, sdd, budget, flags.skip_rotten)?;
            let code = report.code;
            if !flags.json {
                report.emit_text();
            }
            reports.push(report);
            Ok(code)
        })
    } else {
        let report = check_one(&root, &sdds[0], budget, flags.skip_rotten)?;
        if !flags.json {
            report.emit_text();
        }
        let code = report.code;
        reports.push(report);
        code
    };

    if flags.json {
        let out = if flags.all {
            J::Arr(reports.iter().map(Report::json).collect())
        } else {
            reports[0].json()
        };
        emit(&out.dumps_indent2());
        return Ok(code);
    }

    let findings: usize = reports.iter().map(|r| r.findings.len()).sum();
    let unclean = reports.iter().filter(|r| !r.findings.is_empty()).count();
    let unchecked = sdds.len() - reports.len();
    if findings == 0 && unchecked == 0 {
        emit(&format!("check clean: {}", plural(sdds.len(), "SDD")));
    } else {
        let mut line = format!(
            "check: {} in {unclean} of {}",
            plural(findings, "finding"),
            plural(sdds.len(), "SDD")
        );
        if unchecked > 0 {
            line.push_str(&format!("; {unchecked} not checked"));
        }
        emit(&line);
    }
    Ok(code)
}
