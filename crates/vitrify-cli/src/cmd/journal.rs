//! `vitrify journal append|replay` — the journal verbs (NDJSON events).

use vitrify_core::CoreError;
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{Instance, UtcTime};

use crate::cli::EventKind;
use crate::out::{J, emit};

fn load(sdd: Option<&str>) -> Result<Instance, CoreError> {
    let root = super::repo_root();
    Instance::load(super::resolve_sdd(&root, sdd, &super::cwd())?)
}

#[derive(Default)]
pub struct AppendArgs {
    pub unit: Option<String>,
    pub actor: Option<String>,
    pub claim_seq: Option<i64>,
    pub entry: Option<String>,
    pub stale: Option<String>,
    pub cause: Option<String>,
    pub findings: Option<i64>,
    pub commit: Option<String>,
    pub sdd: Option<String>,
}

pub fn append(event: EventKind, args: AppendArgs) -> Result<u8, CoreError> {
    let unit_required = matches!(
        event,
        EventKind::Claim
            | EventKind::Renew
            | EventKind::Release
            | EventKind::Built
            | EventKind::Verified
            | EventKind::Stale
            | EventKind::Retired
    );
    if unit_required && args.unit.is_none() {
        return Err(CoreError::refusal(
            2,
            format!("error: --event {} requires --unit", event.as_str()),
        ));
    }
    let inst = load(args.sdd.as_deref())?;
    let handle = inst.handle();
    let (events, _) = journal::read(&handle)?;
    let stale = args.stale.map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let e = Event {
        seq: events.last().map(|e| e.seq).unwrap_or(0) + 1,
        at: UtcTime::now().format(),
        event: event.as_str().to_string(),
        unit: args.unit,
        actor: args.actor,
        claim_seq: args.claim_seq,
        entry: args.entry,
        stale,
        cause: args.cause,
        findings: args.findings,
        suite: None,
        sdd: inst.meta.get_str_or("version", "0.0.0").to_string(),
        commit: args.commit,
    };
    let line = journal::render_line(&e);
    journal::append_line(&handle, &line)?;
    emit(&line);
    Ok(0)
}

/// A journal replayed against the units' front matter.
pub struct Replayed {
    pub events: usize,
    pub projection: journal::Projection,
    pub mismatches: Vec<journal::VerifyMismatch>,
    pub consistent: bool,
}

/// Replay `inst`'s journal and compare it with front matter; `None` when
/// the SDD has no journal, or an empty one.
pub fn replayed(inst: &Instance) -> Result<Option<Replayed>, CoreError> {
    let (events, exists) = journal::read(&inst.handle())?;
    if !exists || events.is_empty() {
        return Ok(None);
    }
    let projection = journal::replay(&events);
    let mismatches = journal::verify(&projection, &inst.units);
    let consistent = mismatches.is_empty() && projection.order_findings.is_empty();
    Ok(Some(Replayed {
        events: events.len(),
        projection,
        mismatches,
        consistent,
    }))
}

impl Replayed {
    /// The exit code `journal replay` gives it.
    pub fn code(&self) -> u8 {
        if self.consistent { 0 } else { 3 }
    }

    /// Every problem as `(subject, cause, remediation)`.
    pub fn findings(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<(String, String, String)> = self
            .projection
            .order_findings
            .iter()
            .map(|f| {
                (
                    "journal".to_string(),
                    f.clone(),
                    "restore the journal from version control: it is append-only, and only vitrify's verbs append to it".to_string(),
                )
            })
            .collect();
        for m in &self.mismatches {
            out.push((
                m.unit.clone(),
                format!(
                    "{}: journal projects {}, front matter says {}",
                    m.field,
                    m.projected.repr(),
                    m.current.repr()
                ),
                format!(
                    "set {}'s {} back to {}, as the journal projects; state changes go through vitrify's verbs, never by hand",
                    m.unit,
                    m.field,
                    m.projected.repr()
                ),
            ));
        }
        out
    }

    /// `{events, consistent, order_findings, mismatches, projection}`.
    fn json(&self) -> Vec<(String, J)> {
        let jm: Vec<J> = self
            .mismatches
            .iter()
            .map(|m| {
                let val = |v: &journal::FieldVal| match v {
                    journal::FieldVal::Str(s) => J::s(s.clone()),
                    journal::FieldVal::Int(i) => J::Int(*i),
                };
                J::Obj(vec![
                    ("unit".into(), J::s(m.unit.clone())),
                    ("field".into(), J::s(m.field)),
                    ("projected".into(), val(&m.projected)),
                    ("current".into(), val(&m.current)),
                ])
            })
            .collect();
        vec![
            ("events".into(), J::Int(self.events as i64)),
            ("consistent".into(), J::Bool(self.consistent)),
            (
                "order_findings".into(),
                J::Arr(
                    self.projection
                        .order_findings
                        .iter()
                        .map(|f| J::s(f.clone()))
                        .collect(),
                ),
            ),
            ("mismatches".into(), J::Arr(jm)),
            (
                "projection".into(),
                J::Obj(
                    self.projection
                        .status
                        .iter()
                        .map(|(u, s)| (u.clone(), J::s(s.clone())))
                        .collect(),
                ),
            ),
        ]
    }

    /// The text report, each line under `under`.
    fn emit_text(&self, under: &str) {
        if self.consistent {
            emit(&format!(
                "{under}journal consistent ({} events)",
                self.events
            ));
            return;
        }
        for f in &self.projection.order_findings {
            emit(&format!("{under}order: {f}"));
        }
        for m in &self.mismatches {
            emit(&format!(
                "{under}mismatch {}.{}: journal projects {}, front matter says {}",
                m.unit,
                m.field,
                m.projected.repr(),
                m.current.repr()
            ));
        }
    }
}

pub fn replay(json: bool, sdd: Option<&str>, all: bool) -> Result<u8, CoreError> {
    if !all {
        let inst = load(sdd)?;
        let Some(r) = replayed(&inst)? else {
            return Err(CoreError::refusal(4, "error: no journal found (or empty)"));
        };
        if json {
            emit(&J::Obj(r.json()).dumps_indent2());
        } else {
            r.emit_text("");
        }
        return Ok(r.code());
    }

    // every SDD: each line under its SDD's name; one with no journal yet is
    // reported and fails nothing
    let root = super::repo_root();
    let sdds = super::targets(&root, sdd, true, &super::cwd())?;
    let mut reports: Vec<J> = Vec::new();
    let code = super::each(&root, &sdds, |sdd| {
        let inst = Instance::load(sdd.clone())?;
        let name = super::label(sdd, &root);
        let r = replayed(&inst)?;
        if json {
            let mut pairs = vec![
                ("sdd".into(), J::s(name)),
                ("root".into(), J::s(super::shown(&root, &sdd.root))),
            ];
            match &r {
                Some(r) => pairs.extend(r.json()),
                None => pairs.extend([
                    ("events".into(), J::Int(0)),
                    ("consistent".into(), J::Null),
                    ("order_findings".into(), J::Arr(Vec::new())),
                    ("mismatches".into(), J::Arr(Vec::new())),
                    ("projection".into(), J::Obj(Vec::new())),
                ]),
            }
            reports.push(J::Obj(pairs));
        } else {
            match &r {
                Some(r) => r.emit_text(&format!("{name}: ")),
                None => emit(&format!("{name}: no journal yet")),
            }
        }
        Ok(r.as_ref().map_or(0, Replayed::code))
    });
    if json {
        emit(&J::Arr(reports).dumps_indent2());
    }
    Ok(code)
}
