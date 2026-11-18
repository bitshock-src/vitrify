//! `vitrify change apply` — a change entry's bookkeeping in one step.
//!
//! For each Active SDD whose last change entry has no `change` event yet:
//! hold `version` and `updated` to the entry (as lint L10 does) and refuse
//! on a stale pin; take the units whose recorded trace no longer matches
//! (audit check 5); a PATCH entry re-records their traces, a MINOR or MAJOR
//! entry marks them Stale; append the events and regenerate the ledger.
//! Working tree only — the commit is the author's.

use std::path::Path;

use vitrify_core::audit::{self, Options, StateEdit};
use vitrify_core::changelog::{self, Bump};
use vitrify_core::contract::{self, State};
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{self, Instance, SddHandle, Shape, UtcTime};
use vitrify_core::{CoreError, ledger, yamlish};

use crate::out::{J, diag, emit};

pub struct ApplyFlags {
    pub sdd: Option<String>,
    /// Every SDD of the repository.
    pub all: bool,
    /// Print what would be done; write nothing.
    pub dry_run: bool,
    /// The code commit recorded on the events.
    pub commit: Option<String>,
    pub json: bool,
}

/// What became of one SDD.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Applied,
    AlreadyRecorded,
    /// Not Active: a Draft SDD (or a Deprecated one) has nothing to apply.
    NotActive,
    NoEntry,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Action::Applied => "applied",
            Action::AlreadyRecorded => "already-recorded",
            Action::NotActive => "draft",
            Action::NoEntry => "no-entry",
        }
    }
}

/// One SDD's report.
struct Report {
    label: String,
    root: String,
    /// The SDD's own status, for the line of an SDD that is not Active.
    status: String,
    action: Action,
    entry: Option<String>,
    from: Option<String>,
    to: Option<String>,
    bump: Option<Bump>,
    restamped: Vec<String>,
    stale: Vec<String>,
    /// Where the entry is written: `changelog.md`, or the single file.
    changelog: String,
}

impl Report {
    fn staleness(&self) -> String {
        if self.stale.is_empty() {
            "- Staleness applied: —".to_string()
        } else {
            format!("- Staleness applied: {} → Stale", self.stale.join(", "))
        }
    }

    fn transition(&self) -> String {
        let mut t = match (&self.from, &self.to) {
            (Some(from), Some(to)) => format!("{from} → {to}"),
            (None, Some(to)) => format!("→ {to}"),
            (Some(from), None) => format!("{from} →"),
            (None, None) => String::new(),
        };
        if let Some(b) = self.bump {
            t.push_str(&format!(" ({})", b.as_str()));
        }
        t
    }

    fn lines(&self) -> Vec<String> {
        let at = &self.label;
        let entry = self.entry.as_deref().unwrap_or("");
        match self.action {
            Action::NotActive => vec![format!("{at}: {} — nothing to apply", self.status)],
            Action::NoEntry => vec![format!("{at}: no change entry — nothing to apply")],
            Action::AlreadyRecorded => {
                vec![format!("{at}: {entry} already recorded — nothing to apply")]
            }
            Action::Applied => {
                let what = if self.bump == Some(Bump::Patch) {
                    if self.restamped.is_empty() {
                        "no trace to re-record".to_string()
                    } else {
                        format!("traces of {} re-recorded", self.restamped.join(", "))
                    }
                } else if self.stale.is_empty() {
                    "no unit marked Stale".to_string()
                } else {
                    format!("{} marked Stale", self.stale.join(", "))
                };
                let head = match self.transition() {
                    t if t.is_empty() => format!("{at}: {entry}: {what}"),
                    t => format!("{at}: {entry} {t}: {what}"),
                };
                vec![head, format!("{at}: {}", self.staleness())]
            }
        }
    }

    fn json(&self, dry_run: bool) -> J {
        let units = |us: &[String]| J::Arr(us.iter().map(|u| J::s(u.clone())).collect());
        J::Obj(vec![
            ("sdd".into(), J::s(self.label.clone())),
            ("root".into(), J::s(self.root.clone())),
            ("entry".into(), J::opt_str(self.entry.clone())),
            ("from".into(), J::opt_str(self.from.clone())),
            ("to".into(), J::opt_str(self.to.clone())),
            (
                "bump".into(),
                J::opt_str(self.bump.map(|b| b.as_str().to_string())),
            ),
            ("action".into(), J::s(self.action.as_str())),
            ("restamped".into(), units(&self.restamped)),
            ("stale".into(), units(&self.stale)),
            (
                "staleness_line".into(),
                J::opt_str((self.action == Action::Applied).then(|| self.staleness())),
            ),
            ("dry_run".into(), J::Bool(dry_run)),
        ])
    }
}

/// Why an SDD is refused: one `error: …` line each.
struct Refused(Vec<String>);

fn event(inst: &Instance, events: &[Event], kind: &str, commit: Option<&str>) -> Event {
    Event {
        seq: journal::next_seq(events),
        at: UtcTime::now().format(),
        event: kind.to_string(),
        sdd: inst.meta.get_str_or("version", "0.0.0").to_string(),
        commit: commit.map(str::to_string),
        ..Default::default()
    }
}

fn append(inst: &Instance, e: Event) -> Result<(), CoreError> {
    journal::append_line(&inst.handle(), &journal::render_line(&e))?;
    Ok(())
}

/// The units whose recorded trace no longer matches their current inputs
/// (audit check 5), each with the cause `audit --apply-stale` records, in
/// unit order. A Built or Verified unit with no recorded trace is the
/// audit's to report, not an entry's to apply.
fn moved(root: &Path, inst: &Instance) -> Result<Vec<(String, String)>, CoreError> {
    let opts = super::audit_options(
        Options {
            skip_rotten: true,
            apply_stale: true,
            ..Default::default()
        },
        root,
        inst,
    );
    let (_, edits) = audit::run(inst, root, &opts, &crate::gate::ShRunner, UtcTime::now())?;
    Ok(edits
        .into_iter()
        .filter_map(|edit| match edit {
            StateEdit::ApplyStale { unit, cause } => Some((unit, cause)),
            StateEdit::FixLease { .. } => None,
        })
        .filter(|(unit, _)| {
            inst.unit(unit)
                .and_then(|u| u.meta.get_map("trace"))
                .is_some_and(|t| !t.is_empty())
        })
        .collect())
}

fn apply_one(
    root: &Path,
    sdd: &SddHandle,
    flags: &ApplyFlags,
) -> Result<Result<Report, Refused>, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let handle = inst.handle();
    let label = super::label(sdd, root);
    let shown_root = super::shown(root, &sdd.root);
    let changelog_path = match handle.shape {
        Shape::Bundle => handle.root.join("changelog.md"),
        Shape::Single => handle.file.clone(),
    };
    let mut report = Report {
        label: label.clone(),
        root: shown_root.clone(),
        status: inst.meta.get_str_or("status", "Draft").trim().to_string(),
        action: Action::NotActive,
        entry: None,
        from: None,
        to: None,
        bump: None,
        restamped: Vec::new(),
        stale: Vec::new(),
        changelog: super::shown(root, &changelog_path),
    };
    if report.status != "Active" {
        return Ok(Ok(report));
    }
    // an entry where none is read would be passed over, or taken for no
    // entry at all
    let misplaced = changelog::misplaced(&handle)?;
    if !misplaced.is_empty() {
        return Ok(Err(Refused(
            misplaced
                .iter()
                .map(|(id, hashes)| {
                    let (cause, fix) = changelog::misplaced_finding(id, *hashes);
                    format!(
                        "error: {label}: {id}: {cause} → {fix}, then run vitrify change apply --sdd {shown_root}"
                    )
                })
                .collect(),
        )));
    }
    let entries = changelog::entries(&handle)?;
    let Some(last) = changelog::last(&entries) else {
        report.action = Action::NoEntry;
        return Ok(Ok(report));
    };
    report.entry = Some(last.id.clone());
    report.from = changelog::from_version(&entries, last);
    report.to = last.to.clone();
    report.bump = match (&report.from, &report.to) {
        (Some(from), Some(to)) => changelog::bump(from, to),
        _ => None,
    };
    let (events, _) = journal::read(&handle)?;
    if journal::records_change(&events, &last.id) {
        report.action = Action::AlreadyRecorded;
        return Ok(Ok(report));
    }

    // Refusals: the front matter not at the entry, a pin behind its authority.
    let mut refused = Vec::new();
    let version = inst.meta.get_str_or("version", "");
    let updated = inst.meta.get_str_or("updated", "");
    for (subject, cause) in changelog::findings(&entries, version, updated) {
        if subject == "version" || subject == "updated" {
            refused.push(format!(
                "error: {label}: {subject}: {cause} → set version and updated to {}'s, then run vitrify change apply --sdd {shown_root}",
                last.id
            ));
        }
    }
    for ext in contract::examine(&inst, &super::peers(root))? {
        if ext.state == State::Stale {
            refused.push(format!(
                "error: {label}: {}'s pin is stale → vitrify pin sync --sdd {shown_root} first",
                ext.id
            ));
        }
    }
    let moved = moved(root, &inst)?;
    if report.bump.is_none() && !moved.is_empty() {
        refused.push(format!(
            "error: {label}: {}'s heading names no version bump ({}) and traces moved → write its transition, <from> → <to>, in the heading",
            last.id,
            match report.transition() {
                t if t.is_empty() => "no →".to_string(),
                t => t,
            }
        ));
    }
    if !refused.is_empty() {
        return Ok(Err(Refused(refused)));
    }

    report.action = Action::Applied;
    let commit = flags.commit.as_deref();
    let mut change = event(&inst, &events, "change", commit);
    change.entry = Some(last.id.clone());
    if report.bump == Some(Bump::Patch) {
        // compute every trace before writing any, so a missing input
        // refuses with nothing written
        let mut traces = Vec::new();
        for (unit, _) in &moved {
            let Some(u) = inst.unit(unit) else { continue };
            traces.push((u.clone(), super::trace::current(&inst, u)?));
        }
        report.restamped = traces.iter().map(|(u, _)| u.unit.clone()).collect();
        if !flags.dry_run {
            for (u, t) in &traces {
                super::trace::restamp(&inst, u, t)?;
            }
            append(&inst, change)?;
        }
    } else {
        report.stale = moved.iter().map(|(u, _)| u.clone()).collect();
        if !flags.dry_run {
            if !report.stale.is_empty() {
                change.stale = Some(report.stale.clone());
            }
            append(&inst, change)?;
            for (unit, cause) in &moved {
                let Some(u) = inst.unit(unit) else { continue };
                model::rewrite_unit_meta(&handle, u, |inner| {
                    yamlish::set_scalar_in_place(inner, "status", "Stale")
                })?;
                let (events, _) = journal::read(&handle)?;
                // the mark `audit --apply-stale` makes, with the commit
                let mut stale = event(&inst, &events, "stale", commit);
                stale.unit = Some(unit.clone());
                stale.actor = Some("audit".to_string());
                stale.cause = Some(cause.clone());
                append(&inst, stale)?;
            }
        }
    }
    if !flags.dry_run {
        let fresh = Instance::load(inst.handle())?;
        ledger::regen(&fresh, &UtcTime::now().format_date())?;
    }
    Ok(Ok(report))
}

/// One SDD, reported: its lines (or JSON object) and the `next:` hint, or
/// its refusal. Returns its exit code.
fn report_one(
    root: &Path,
    sdd: &SddHandle,
    flags: &ApplyFlags,
    json_out: &mut Vec<J>,
) -> Result<u8, CoreError> {
    match apply_one(root, sdd, flags)? {
        Err(Refused(lines)) => {
            for line in lines {
                diag(&line);
            }
            Ok(2)
        }
        Ok(report) => {
            if flags.json {
                json_out.push(report.json(flags.dry_run));
            } else {
                for line in report.lines() {
                    emit(&line);
                }
            }
            if report.action == Action::Applied && !flags.dry_run {
                diag(&format!(
                    "next: copy the Staleness applied line into {} ({}), then run vitrify check",
                    report.entry.as_deref().unwrap_or(""),
                    report.changelog
                ));
            }
            Ok(0)
        }
    }
}

pub fn apply(flags: ApplyFlags) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, flags.sdd.as_deref(), flags.all, &super::cwd())?;
    let mut json_out: Vec<J> = Vec::new();
    let code = if flags.all {
        let code = super::each(&root, &sdds, |sdd| {
            report_one(&root, sdd, &flags, &mut json_out)
        });
        if flags.json {
            emit(&J::Arr(json_out).dumps_indent2());
        }
        code
    } else {
        let code = report_one(&root, &sdds[0], &flags, &mut json_out)?;
        if let Some(obj) = json_out.pop() {
            emit(&obj.dumps_indent2());
        }
        code
    };
    if flags.dry_run {
        diag("dry run: nothing written");
    }
    Ok(code)
}
