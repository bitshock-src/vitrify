//! `vitrify locate` — SDD discovery; `--units` adds the per-unit view with
//! the computed claimable flag; `--graph` the SDD graph the units' edges
//! imply. Output held by the regression goldens
//! (`fixtures/golden/calc/locate.json`).

use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::graph::{self, EdgeView, GraphView, Node, UnitRef};
use vitrify_core::model::{self, UtcTime};

use crate::out::{J, diag, emit};

pub fn run(units: bool, graph: bool, sdd: Option<&str>, json: bool) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let cwd = super::cwd();

    if graph {
        return show_graph(&root, json);
    }

    if units {
        let handle = super::resolve_sdd(&root, sdd, &cwd)?;
        let inst = model::Instance::load(handle)?;
        let now = UtcTime::now();
        // a pin in depends_on is met by the units of other SDDs it names
        let unit_graph = if super::reaches_out(&inst) {
            Some(super::repo_graph(&root, Some(&inst))?)
        } else {
            None
        };
        let slug = graph::slug_of(&inst);
        let views: Vec<model::UnitView> = inst
            .units
            .iter()
            .map(|u| match &unit_graph {
                Some(g) => {
                    let node = Node::new(slug, u.unit.as_str());
                    model::unit_view_with(u, &inst.units, now, &g.resolver(&node))
                }
                None => model::unit_view(u, &inst.units, now),
            })
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

/// `--graph`: every SDD and the SDDs its units depend on, each SDD edge
/// with the pins and unit edges behind it; with a cycle, each cycle and the
/// units waiting behind one. `--json` adds the unit graph and its order.
fn show_graph(root: &Path, json: bool) -> Result<u8, CoreError> {
    let (insts, refused) = super::loaded(root, None);
    if insts.is_empty() && refused.is_empty() {
        return Err(CoreError::refusal(4, model::NO_SDD));
    }
    for (h, e) in &refused {
        let text = e.to_string();
        diag(&format!(
            "{}: left out of the graph: {}",
            super::label(h, root),
            text.strip_prefix("error: ").unwrap_or(&text)
        ));
    }
    let view = graph::UnitGraph::build(&insts)?.view();
    if json {
        emit(&graph_json(&view).dumps_indent2());
        return Ok(0);
    }
    for slug in &view.sdds {
        let out: Vec<_> = view.sdd_edges.iter().filter(|e| &e.from == slug).collect();
        if out.is_empty() {
            emit(&format!("{slug}  (depends on no other SDD)"));
        }
        for e in out {
            emit(&format!("{} → {}  via {}", e.from, e.to, e.pins.join(", ")));
            for u in &e.units {
                let via = u.via.as_deref().map(|v| format!("  via {v}"));
                emit(&format!(
                    "  {} → {}{}",
                    unit_name(&u.from),
                    unit_name(&u.to),
                    via.unwrap_or_default()
                ));
            }
        }
    }
    for c in &view.cycles {
        let path: Vec<String> = c.iter().map(unit_name).collect();
        emit(&format!("cycle: {}", path.join(" → ")));
    }
    for w in &view.waiting {
        emit(&format!("waiting behind a cycle: {}", unit_name(w)));
    }
    Ok(0)
}

fn unit_name(u: &UnitRef) -> String {
    format!("{} {}", u.sdd, u.unit)
}

fn unit_json(u: &UnitRef) -> J {
    J::Obj(vec![
        ("sdd".into(), J::s(u.sdd.clone())),
        ("unit".into(), J::s(u.unit.clone())),
    ])
}

fn units_json(us: &[UnitRef]) -> J {
    J::Arr(us.iter().map(unit_json).collect())
}

fn edge_json(e: &EdgeView) -> J {
    J::Obj(vec![
        ("from".into(), unit_json(&e.from)),
        ("to".into(), unit_json(&e.to)),
        ("via".into(), J::opt_str(e.via.clone())),
    ])
}

fn strings_json(ss: &[String]) -> J {
    J::Arr(ss.iter().map(|s| J::s(s.clone())).collect())
}

/// The graph view as ordered pairs, in [`GraphView`]'s field order.
fn graph_json(v: &GraphView) -> J {
    let sdd_edges = v
        .sdd_edges
        .iter()
        .map(|e| {
            J::Obj(vec![
                ("from".into(), J::s(e.from.clone())),
                ("to".into(), J::s(e.to.clone())),
                ("pins".into(), strings_json(&e.pins)),
                (
                    "units".into(),
                    J::Arr(e.units.iter().map(edge_json).collect()),
                ),
            ])
        })
        .collect();
    J::Obj(vec![
        ("sdds".into(), strings_json(&v.sdds)),
        ("sdd_edges".into(), J::Arr(sdd_edges)),
        (
            "unit_edges".into(),
            J::Arr(v.unit_edges.iter().map(edge_json).collect()),
        ),
        ("order".into(), units_json(&v.order)),
        (
            "cycles".into(),
            J::Arr(v.cycles.iter().map(|c| units_json(c)).collect()),
        ),
        ("waiting".into(), units_json(&v.waiting)),
    ])
}
