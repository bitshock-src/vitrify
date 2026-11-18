//! `vitrify readset` — the ordered minimal reading set for one unit, with
//! the byte-budget check (byte-denominated by design). JSON output held by
//! the regression goldens (`fixtures/golden/calc/readset-*.json`).

use vitrify_core::CoreError;
use vitrify_core::config::Overrides;
use vitrify_core::model::Instance;
use vitrify_core::readset;

use crate::out::{J, emit};

pub fn run(
    unit: &str,
    sdd: Option<&str>,
    json: bool,
    budget: Option<Option<i64>>,
) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let cwd = super::cwd();
    let inst = Instance::load(super::resolve_sdd(&root, sdd, &cwd)?)?;
    let items = readset::resolve(&inst, unit, &super::peers(&root))?;

    if let Some(flag) = budget {
        let limit = match flag {
            Some(bytes) => bytes,
            None => {
                super::load_config(&Overrides::default())?
                    .lint
                    .readset_budget_bytes
            }
        };
        let total: i64 = items.iter().map(|i| i.bytes as i64).sum();
        if total > limit {
            return Err(CoreError::refusal(
                2,
                format!("error: reading set for {unit} totals {total} bytes, over budget {limit}"),
            ));
        }
    }

    let consumes = inst
        .unit(unit)
        .map(|u| u.meta.get_list("consumes"))
        .unwrap_or_default();

    if json {
        let readings: Vec<J> = items
            .iter()
            .map(|item| {
                J::Obj(vec![
                    ("path".into(), J::s(item.path.display().to_string())),
                    ("region".into(), J::opt_str(item.region.clone())),
                ])
            })
            .collect();
        let obj = J::Obj(vec![
            ("unit".into(), J::s(unit)),
            ("sdd".into(), J::s(inst.root.display().to_string())),
            ("shape".into(), J::s(inst.shape.as_str())),
            (
                "consumes".into(),
                J::Arr(consumes.into_iter().map(J::Str).collect()),
            ),
            ("readings".into(), J::Arr(readings)),
        ]);
        emit(&obj.dumps_indent2());
    } else {
        for item in &items {
            match &item.region {
                Some(r) => emit(&format!("{}  ::  {}", item.path.display(), r)),
                None => emit(&item.path.display().to_string()),
            }
        }
    }
    Ok(0)
}
