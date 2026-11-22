//! `vitrify check` — every read-only consistency check vitrify has, over one
//! SDD or all of them: lint, the audit, the journal replay, the ledger check
//! and the pin states, each finding with its source and its remediation;
//! open questions as notes. Then the dirty units: Verified units a committed
//! change reached since their last recorded check, each with the changed
//! paths that made it so — listed, never re-checked; that is a run's job.
//! Writes nothing.

use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::cascade::{Anchors, Dirty, Why};
use vitrify_core::config::Overrides;
use vitrify_core::graph::{self, render_path};
use vitrify_core::journal as core_journal;
use vitrify_core::model::{Instance, SddHandle, is_empty_marker};

use crate::git::Git;
use crate::incremental::dirty::{self, Cascade, DirtySet, Mode, Snapshot};
use crate::out::{J, diag, emit};

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
    /// Its dirty units, sorted by unit id.
    dirty: Vec<Dirty>,
    /// Its recorded checks.
    anchors: Anchors,
    code: u8,
}

/// `changed`, `no-anchor`, `unresolved-anchor` or `everything`: why a unit
/// is dirty, as `--json` names it.
fn why_code(why: &Why) -> &'static str {
    match why {
        Why::Paths(_) => "changed",
        Why::NoAnchor => "no-anchor",
        Why::NoDiff(_) => "unresolved-anchor",
        Why::Everything => "everything",
    }
}

fn dirty_json(d: &Dirty, anchor: Option<String>) -> J {
    let paths = match &d.why {
        Why::Paths(reasons) => reasons
            .iter()
            .map(|r| {
                J::Obj(vec![
                    ("path".into(), J::s(r.path.clone())),
                    ("owner".into(), J::s(r.owner.describe())),
                ])
            })
            .collect(),
        _ => Vec::new(),
    };
    J::Obj(vec![
        ("unit".into(), J::s(d.node.unit.clone())),
        ("anchor".into(), J::opt_str(anchor)),
        ("why".into(), J::s(why_code(&d.why))),
        ("paths".into(), J::Arr(paths)),
    ])
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
            (
                "dirty".into(),
                J::Arr(
                    self.dirty
                        .iter()
                        .map(|d| dirty_json(d, self.anchor_of(d)))
                        .collect(),
                ),
            ),
        ])
    }

    /// The anchor a dirty unit's reason names, when it has one.
    fn anchor_of(&self, d: &Dirty) -> Option<String> {
        match &d.why {
            Why::NoDiff(anchor) => Some(anchor.clone()),
            Why::NoAnchor | Why::Everything => None,
            Why::Paths(_) => self.anchors.unit(&d.node.unit).map(str::to_string),
        }
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
        for d in &self.dirty {
            emit(&format!(
                "{}: dirty {}: {}",
                self.sdd,
                d.node.unit,
                dirty::describe(&d.why)
            ));
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
    dirty: Option<&DirtySet>,
) -> Result<Report, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let at = super::shown(root, &inst.root);
    let (events, _) = core_journal::read(&inst.handle())?;
    let slug = graph::slug_of(&inst);
    let mut report = Report {
        sdd: super::label(sdd, root),
        root: at.clone(),
        findings: Vec::new(),
        notes: Vec::new(),
        dirty: dirty
            .map(|set| set.of_sdd(slug).cloned().collect())
            .unwrap_or_default(),
        anchors: Anchors::from_events(&events),
        code: 0,
    };
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

/// The repository's dirty units at `HEAD`, from the checkout at `root`.
/// `None` outside a git repository, before its first commit, and — with a
/// note on stderr — when the SDDs cannot all be read or the unit graph has
/// a cycle: there is nothing to cascade along then.
fn dirty_units(root: &Path) -> Option<DirtySet> {
    let git = Git::new(root);
    let head = git.resolve_commit("HEAD").ok().flatten()?;
    let not_computed = |why: &str| diag(&format!("note: dirty units not computed: {why}"));
    let snapshot = match Snapshot::load(&git, root, &head) {
        Ok(s) => s,
        Err(e) => {
            let text = e.to_string();
            not_computed(text.strip_prefix("error: ").unwrap_or(&text));
            return None;
        }
    };
    match snapshot.dirty(&git, Mode::Changes) {
        Ok(Cascade::Resolved(set)) => Some(set),
        Ok(Cascade::Cycles(cycles)) => {
            let paths: Vec<String> = cycles.cycles.iter().map(|c| render_path(c)).collect();
            not_computed(&format!("the unit graph has a cycle: {}", paths.join("; ")));
            None
        }
        Err(e) => {
            let text = e.to_string();
            not_computed(text.strip_prefix("error: ").unwrap_or(&text));
            None
        }
    }
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

    let dirty = dirty_units(&root);
    let mut reports: Vec<Report> = Vec::new();
    let code = if flags.all {
        super::each(&root, &sdds, |sdd| {
            let report = check_one(&root, sdd, budget, flags.skip_rotten, dirty.as_ref())?;
            let code = report.code;
            if !flags.json {
                report.emit_text();
            }
            reports.push(report);
            Ok(code)
        })
    } else {
        let report = check_one(&root, &sdds[0], budget, flags.skip_rotten, dirty.as_ref())?;
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
    let dirty: usize = reports.iter().map(|r| r.dirty.len()).sum();
    let mut line = if findings == 0 && unchecked == 0 {
        format!("check clean: {}", plural(sdds.len(), "SDD"))
    } else {
        let mut line = format!(
            "check: {} in {unclean} of {}",
            plural(findings, "finding"),
            plural(sdds.len(), "SDD")
        );
        if unchecked > 0 {
            line.push_str(&format!("; {unchecked} not checked"));
        }
        line
    };
    if dirty > 0 {
        line.push_str(&format!("; {} dirty", plural(dirty, "unit")));
    }
    emit(&line);
    Ok(code)
}
