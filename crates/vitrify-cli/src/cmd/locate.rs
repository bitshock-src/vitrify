//! `vitrify locate` — SDD discovery; `--units` adds the per-unit view with
//! the computed claimable flag. Output held by the regression goldens
//! (`fixtures/golden/calc/locate.json`).

use vitrify_core::CoreError;
use vitrify_core::model::{self, UtcTime};

use crate::out::{J, emit};

pub fn run(units: bool, sdd: Option<&str>, json: bool) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let cwd = super::cwd();

    if units {
        let handle = super::resolve_sdd(&root, sdd, &cwd)?;
        let inst = model::Instance::load(handle)?;
        let now = UtcTime::now();
        let views: Vec<model::UnitView> = inst
            .units
            .iter()
            .map(|u| model::unit_view(u, &inst.units, now))
            .collect();
        if json {
            let junits: Vec<J> = views.iter().map(view_to_json).collect();
            let obj = J::Obj(vec![
                ("sdd".into(), J::s(inst.root.display().to_string())),
                ("shape".into(), J::s(inst.shape.as_str())),
                (
                    "version".into(),
                    J::opt_str(inst.meta.get_str("version").map(str::to_string)),
                ),
                (
                    "status".into(),
                    J::opt_str(inst.meta.get_str("status").map(str::to_string)),
                ),
                ("units".into(), J::Arr(junits)),
            ]);
            emit(&obj.dumps_indent2());
        } else {
            for v in &views {
                let flag = if v.claimable { "claimable" } else { "" };
                let group = v.parallel_group.as_deref().unwrap_or("-");
                emit(&format!(
                    "{:<8} {:<9} group={:<3} seq={} {}",
                    v.unit, v.status, group, v.claim_seq, flag
                ));
            }
        }
        return Ok(0);
    }

    let found = super::discover(&root);
    if found.is_empty() {
        return Err(CoreError::refusal(4, model::NO_SDD));
    }
    let mut recs: Vec<(J, String)> = Vec::new();
    for s in &found {
        let n_units = model::load_units(s)?.len();
        let scope: Vec<J> = s
            .meta
            .get_list("scope_roots")
            .into_iter()
            .map(J::Str)
            .collect();
        let opt = |k: &str| J::opt_str(s.meta.get_str(k).map(str::to_string));
        let obj = J::Obj(vec![
            ("root".into(), J::s(s.root.display().to_string())),
            ("shape".into(), J::s(s.shape.as_str())),
            ("sdd".into(), opt("sdd")),
            ("title".into(), opt("title")),
            ("version".into(), opt("version")),
            ("status".into(), opt("status")),
            ("scope_roots".into(), J::Arr(scope)),
            ("units".into(), J::Int(n_units as i64)),
        ]);
        let py_none = |k: &str| {
            s.meta
                .get_str(k)
                .map(str::to_string)
                .unwrap_or_else(|| "None".to_string())
        };
        let line = format!(
            "{}  [{}]  {} v{} {}  units={}",
            s.root.display(),
            s.shape.as_str(),
            py_none("sdd"),
            py_none("version"),
            py_none("status"),
            n_units
        );
        recs.push((obj, line));
    }
    if json {
        emit(&J::Arr(recs.iter().map(|(o, _)| o.clone()).collect()).dumps_indent2());
    } else {
        for (_, line) in &recs {
            emit(line);
        }
    }
    Ok(0)
}

fn view_to_json(v: &model::UnitView) -> J {
    let deps = J::Obj(
        v.depends_on
            .iter()
            .map(|(d, s)| (d.clone(), J::s(s.clone())))
            .collect(),
    );
    J::Obj(vec![
        ("unit".into(), J::s(v.unit.clone())),
        ("status".into(), J::s(v.status.clone())),
        (
            "parallel_group".into(),
            J::opt_str(v.parallel_group.clone()),
        ),
        ("depends_on".into(), deps),
        ("claimed_by".into(), J::opt_str(v.claimed_by.clone())),
        ("claim_expires".into(), J::opt_str(v.claim_expires.clone())),
        ("claim_seq".into(), J::Int(v.claim_seq)),
        ("lease_live".into(), J::Bool(v.lease_live)),
        ("claimable".into(), J::Bool(v.claimable)),
        ("file".into(), J::s(v.file.display().to_string())),
    ])
}
