//! `vitrify trace record|verify` — the trace verbs, wiring
//! `vitrify-core::trace` (the only hashing/masking implementation) plus the
//! stamp edits — and the editorial re-stamp `vitrify change apply` does.
//! These edit the working tree and leave committing to the caller.

use std::path::PathBuf;
use std::process::Command;

use vitrify_core::model::{self, Instance, UtcTime};
use vitrify_core::{CoreError, trace, yamlish};

use crate::cli::StampKind;
use crate::out::{J, emit};

fn short_head() -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();
    if let Ok(o) = out
        && o.status.success()
        && let Ok(s) = String::from_utf8(o.stdout)
    {
        let t = s.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    "0000000".to_string()
}

fn load(sdd: Option<&str>, unit: &str) -> Result<(Instance, model::Unit), CoreError> {
    let root = super::repo_root();
    let inst = Instance::load(super::resolve_sdd(&root, sdd, &super::cwd())?)?;
    let u = inst
        .unit(unit)
        .cloned()
        .ok_or_else(|| CoreError::refusal(4, format!("error: unit not found: {unit}")))?;
    Ok((inst, u))
}

/// The unit's trace as its current inputs give it; a missing input (a
/// consumed contract that does not exist) refuses, exit 4.
pub fn current(inst: &Instance, u: &model::Unit) -> Result<trace::TraceMap, CoreError> {
    let (trace_map, problems) = trace::compute(inst, u)?;
    if !problems.is_empty() {
        let msg = problems
            .iter()
            .map(|p| format!("error: {}: {p}", u.unit))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(CoreError::refusal(4, msg));
    }
    Ok(trace_map)
}

/// The editorial re-stamp: rewrite the unit's recorded trace to `trace_map`
/// (its [`current`] trace), keeping its status and stamps. Returns the file
/// written.
pub fn restamp(
    inst: &Instance,
    u: &model::Unit,
    trace_map: &trace::TraceMap,
) -> Result<PathBuf, CoreError> {
    model::rewrite_unit_meta(&inst.handle(), u, |inner| {
        yamlish::set_map_in_place(inner, "trace", trace_map)
    })
}

pub fn record(
    unit: &str,
    stamp: StampKind,
    commit: Option<&str>,
    sdd: Option<&str>,
) -> Result<u8, CoreError> {
    let (inst, u) = load(sdd, unit)?;
    let trace_map = current(&inst, &u)?;

    let version = inst.meta.get_str_or("version", "0.0.0").to_string();
    let commit = commit.map(str::to_string).unwrap_or_else(short_head);
    let stamp_text = model::render_stamp(&UtcTime::now().format_date(), &version, &commit);
    let stamp_name = match stamp {
        StampKind::Built => "built",
        StampKind::Verified => "verified",
    };

    let path = model::rewrite_unit_meta(&inst.handle(), &u, |inner| {
        let block = match stamp {
            StampKind::Built => {
                let b = yamlish::set_scalar_in_place(inner, "status", "Built");
                yamlish::set_scalar_in_place(&b, "built", &stamp_text)
            }
            StampKind::Verified => {
                let b = yamlish::set_scalar_in_place(inner, "status", "Verified");
                let b = yamlish::set_scalar_in_place(&b, "verified", &stamp_text);
                let b = yamlish::set_scalar_in_place(&b, "claimed_by", "—");
                yamlish::set_scalar_in_place(&b, "claim_expires", "—")
            }
        };
        yamlish::set_map_in_place(&block, "trace", &trace_map)
    })?;

    let recorded = J::Obj(
        trace_map
            .iter()
            .map(|(k, v)| (k.clone(), J::s(v.clone())))
            .collect(),
    );
    let obj = J::Obj(vec![
        ("unit".into(), J::s(unit)),
        ("recorded".into(), recorded),
        ("stamp".into(), J::s(stamp_name)),
        ("file".into(), J::s(path.display().to_string())),
    ]);
    emit(&obj.dumps_default());
    Ok(0)
}

pub fn verify(unit: &str, sdd: Option<&str>, json: bool) -> Result<u8, CoreError> {
    let (inst, u) = load(sdd, unit)?;
    let Some(recorded) = u.meta.get_map("trace").cloned() else {
        return Err(CoreError::refusal(
            4,
            format!("error: no trace recorded for {unit}"),
        ));
    };
    if recorded.is_empty() {
        return Err(CoreError::refusal(
            4,
            format!("error: no trace recorded for {unit}"),
        ));
    }
    let (current, problems) = trace::compute(&inst, &u)?;
    for p in &problems {
        crate::out::diag(&format!("error: {p}"));
    }
    let mismatches = trace::verify(&recorded, &current);
    if json {
        let jm: Vec<J> = mismatches
            .iter()
            .map(|m| {
                J::Obj(vec![
                    ("key".into(), J::s(m.key.clone())),
                    ("recorded".into(), J::opt_str(m.recorded.clone())),
                    ("current".into(), J::opt_str(m.current.clone())),
                ])
            })
            .collect();
        let obj = J::Obj(vec![
            ("unit".into(), J::s(unit)),
            ("match".into(), J::Bool(mismatches.is_empty())),
            ("mismatches".into(), J::Arr(jm)),
        ]);
        emit(&obj.dumps_indent2());
    } else if mismatches.is_empty() {
        emit(&format!("{unit}: trace matches"));
    } else {
        let keys: Vec<&str> = mismatches.iter().map(|m| m.key.as_str()).collect();
        emit(&format!(
            "{unit}: STALE — mismatched keys: {}",
            keys.join(", ")
        ));
    }
    Ok(if mismatches.is_empty() { 0 } else { 3 })
}
