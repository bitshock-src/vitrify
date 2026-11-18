//! Ordered minimal reading set for one work unit (`readset`), held by
//! the regression goldens (`fixtures/golden/`): Ground Rules,
//! Technology Decisions, the unit's own entry, each consumed contract —
//! nothing else. Byte counts per item support the `--budget` check.
//!
//! The unit's own entry is measured the way the trace hashes it: with the
//! fields vitrify itself writes (`status`, the lease fields, the stamps and
//! the `trace` map) masked out. Claiming, stamping and verifying a unit
//! therefore never change the number an author sized it against.

//!
//! One reading can lie outside the SDD: a consumed contract may *pin*
//! another SDD's contract ([`crate::contract`]). Its own entry — header and pin — is read as any
//! contract is, and the body it stands for follows as a reading of its own,
//! handed out only while the pin's hash still matches it.

use std::path::{Path, PathBuf};

use crate::CoreError;
use crate::contract::{self, Peers};
use crate::model::{self, Instance, Shape};
use crate::trace;

/// One reading: a whole file (`region: None`) or a named region of a shared
/// file. `bytes` is the size of the text the item denotes — for the unit's
/// own entry, without the vitrify-managed front-matter fields
/// ([`trace::VOLATILE`]).
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub path: PathBuf,
    pub region: Option<String>,
    pub bytes: u64,
    /// The reading's text, when it is not simply what `path` and `region`
    /// name: the body a pinned contract stands for, checked against its
    /// pin.
    pub text: Option<String>,
}

/// The message prefix of a pin that does not resolve — lint tells such a
/// reading-set failure (its own check's finding) from any other by it.
pub const PIN_PROBLEM: &str = "pinned contract ";

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Resolve the unit's reading set in its fixed order. Refusal (exit 4)
/// carries one `error: ...` line per problem. `peers`
/// is where a pinned contract's authority is looked up; an SDD without pins
/// never asks for it.
pub fn resolve(inst: &Instance, uid: &str, peers: &Peers) -> Result<Vec<Item>, CoreError> {
    let core_text = model::read_text(&inst.file)?;
    let mut problems: Vec<String> = Vec::new();

    for heading in ["Ground Rules", "Technology Decisions"] {
        if model::section_span(&core_text, heading).is_none() {
            problems.push(format!(
                "required region missing: ## {} in {}",
                heading,
                inst.file.display()
            ));
        }
    }

    let Some(unit) = inst.unit(uid) else {
        problems.push(format!("unit not found: {uid}"));
        let msg = problems
            .iter()
            .map(|p| format!("error: {p}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(CoreError::refusal(4, msg));
    };

    let consumes = unit.meta.get_list("consumes");
    let mut readings: Vec<Item> = vec![
        Item {
            label: "Ground Rules".into(),
            path: inst.file.clone(),
            region: Some("Ground Rules".into()),
            bytes: model::section(&core_text, "Ground Rules").len() as u64,
            text: None,
        },
        Item {
            label: "Technology Decisions".into(),
            path: inst.file.clone(),
            region: Some("Technology Decisions".into()),
            bytes: model::section(&core_text, "Technology Decisions").len() as u64,
            text: None,
        },
    ];

    // the unit's own entry, sized as authored: the fields vitrify writes
    // (claim, stamps, trace) are masked exactly as the trace hash masks them
    match inst.shape {
        Shape::Bundle => readings.push(Item {
            label: uid.to_string(),
            path: unit.file.clone(),
            region: None,
            bytes: trace::mask_unit_text(inst, unit)?.len() as u64,
            text: None,
        }),
        Shape::Single => {
            let bytes = trace::mask_unit_text(inst, unit)
                .map(|t| t.len() as u64)
                .unwrap_or(0);
            readings.push(Item {
                label: uid.to_string(),
                path: inst.file.clone(),
                region: Some(format!("Work Units / {uid}")),
                bytes,
                text: None,
            });
        }
    }

    let handle = inst.handle();
    for cid in &consumes {
        match inst.shape {
            Shape::Bundle => {
                let cdir = inst.root.join("contracts");
                match model::contract_file(&inst.root, cid) {
                    Some(p) => {
                        let bytes = file_size(&p);
                        readings.push(Item {
                            label: cid.clone(),
                            path: p,
                            region: None,
                            bytes,
                            text: None,
                        });
                    }
                    None => {
                        problems.push(format!(
                            "consumed contract missing: {cid} (no {}/{cid}-*.md)",
                            cdir.display()
                        ));
                        continue;
                    }
                }
            }
            Shape::Single => match model::contract_text(&handle, cid)? {
                Some(text) => readings.push(Item {
                    label: cid.clone(),
                    path: inst.file.clone(),
                    region: Some(format!("Interface Contracts / {cid}")),
                    bytes: text.len() as u64,
                    text: None,
                }),
                None => {
                    problems.push(format!(
                        "consumed contract missing: {cid} (no ### {cid} region)"
                    ));
                    continue;
                }
            },
        }
        // a pinned contract's body is its authority's, read where it lives
        // and only while the pin still matches it
        if let Some(entry) = contract::find(&handle, cid)? {
            match contract::pinned_body(inst, &entry, peers) {
                Ok(None) => {}
                Ok(Some(body)) => readings.push(Item {
                    label: format!(
                        "{cid} → {} {}",
                        body.authority.slug, body.authority.contract
                    ),
                    path: body.file,
                    region: Some(body.region),
                    bytes: body.text.len() as u64,
                    text: Some(body.text),
                }),
                Err((state, why)) => problems.push(format!(
                    "{PIN_PROBLEM}{cid} does not resolve: {why}{}",
                    if state == contract::State::Stale {
                        " → vitrify pin sync --all"
                    } else {
                        ""
                    }
                )),
            }
        }
    }

    if !problems.is_empty() {
        let msg = problems
            .iter()
            .map(|p| format!("error: {p}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(CoreError::refusal(4, msg));
    }
    Ok(readings)
}

/// The text a reading denotes: what a reader of `item` is given. For a
/// pinned contract's body that is the text checked when the set was
/// resolved; for everything else, the file or region as it is on disk.
pub fn text_of(inst: &Instance, item: &Item) -> Result<String, CoreError> {
    if let Some(text) = &item.text {
        return Ok(text.clone());
    }
    let Some(region) = &item.region else {
        return model::read_text(&item.path);
    };
    let file_text = model::read_text(&item.path)?;
    Ok(if let Some(uid) = region.strip_prefix("Work Units / ") {
        model::find_unit_span(&file_text, uid)
            .map(|(s, e)| file_text[s..e].to_string())
            .unwrap_or_default()
    } else if let Some(cid) = region.strip_prefix("Interface Contracts / ") {
        model::contract_text(&inst.handle(), cid)?.unwrap_or_default()
    } else {
        model::section(&file_text, region).to_string()
    })
}
