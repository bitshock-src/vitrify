//! `vitrify pin list|sync` — external contracts: one SDD's pins on
//! contracts another SDD owns, and its transcriptions of sources no SDD owns
//! ([`vitrify_core::contract`]).
//!
//! `list` reads: every external contract, the authority it names, and
//! whether it still matches — never calling one clean that it could not
//! check. `sync` writes, in the working tree only: a stale pin takes its
//! authority's hash, and the units whose recorded trace that moved are
//! printed as the `Staleness applied` line of the change entry the author
//! still writes.

use std::path::{Path, PathBuf};

use vitrify_core::contract::{self, External, Kind, Peers, State};
use vitrify_core::model::{self, Instance, SddHandle};
use vitrify_core::{CoreError, trace};

use crate::out::{J, diag, emit};

/// The states, in the order a summary counts them.
const STATES: [State; 5] = [
    State::Ok,
    State::Stale,
    State::MissingAuthority,
    State::Invalid,
    State::Unchecked,
];

/// One SDD's external contracts, examined.
struct Examined {
    name: String,
    inst: Instance,
    contracts: Vec<External>,
}

fn examine(root: &Path, sdd: &SddHandle, peers: &Peers) -> Result<Examined, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let contracts = contract::examine(&inst, peers)?;
    Ok(Examined {
        name: super::label(sdd, root),
        inst,
        contracts,
    })
}

/// Examine each of `sdds`; one that cannot be read is reported under its
/// name and counted in the returned exit code.
fn examine_each(root: &Path, sdds: &[SddHandle], peers: &Peers) -> (Vec<Examined>, u8) {
    let mut out = Vec::new();
    let code = super::each(root, sdds, |sdd| {
        out.push(examine(root, sdd, peers)?);
        Ok(0)
    });
    (out, code)
}

fn consumers(inst: &Instance, id: &str) -> Vec<String> {
    inst.units
        .iter()
        .filter(|u| u.meta.get_list("consumes").iter().any(|c| c == id))
        .map(|u| u.unit.clone())
        .collect()
}

/// `pin of alpha C-001`, or what it is instead.
fn names(ext: &External) -> String {
    match (&ext.authority, ext.kind) {
        (Some(a), _) => format!("pin of {} {}", a.slug, a.contract),
        (None, Kind::Pin) => "pin".to_string(),
        (None, Kind::Transcription) => "transcription".to_string(),
    }
}

fn authority_json(root: &Path, ext: &External) -> J {
    match &ext.authority {
        Some(a) => J::Obj(vec![
            ("sdd".into(), J::s(a.slug.clone())),
            ("contract".into(), J::s(a.contract.clone())),
            (
                "file".into(),
                J::opt_str(a.file.as_ref().map(|f| super::shown(root, f))),
            ),
            (
                "bytes".into(),
                a.body_bytes.map(|b| J::Int(b as i64)).unwrap_or(J::Null),
            ),
        ]),
        None => J::Null,
    }
}

fn counts(all: &[Examined]) -> Vec<(State, usize)> {
    STATES
        .iter()
        .map(|s| {
            (
                *s,
                all.iter()
                    .flat_map(|e| &e.contracts)
                    .filter(|c| c.state == *s)
                    .count(),
            )
        })
        .collect()
}

pub fn list(sdd: Option<&str>, all: bool, json: bool) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, sdd, all, &super::cwd())?;
    let peers = super::peers(&root);
    let (examined, read_code) = examine_each(&root, &sdds, &peers);
    let counts = counts(&examined);
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    let findings = examined
        .iter()
        .flat_map(|e| &e.contracts)
        .any(|c| c.state.is_finding());

    if json {
        let rows: Vec<J> = examined
            .iter()
            .flat_map(|e| e.contracts.iter().map(move |c| (e, c)))
            .map(|(e, c)| {
                J::Obj(vec![
                    ("sdd".into(), J::s(e.name.clone())),
                    ("contract".into(), J::s(c.id.clone())),
                    ("file".into(), J::s(super::shown(&root, &c.file))),
                    ("kind".into(), J::s(c.kind.as_str())),
                    ("state".into(), J::s(c.state.as_str())),
                    ("authority".into(), authority_json(&root, c)),
                    ("bytes".into(), J::Int(c.bytes as i64)),
                    (
                        "consumers".into(),
                        J::Arr(consumers(&e.inst, &c.id).into_iter().map(J::Str).collect()),
                    ),
                    (
                        "detail".into(),
                        J::opt_str((c.state != State::Ok).then(|| c.detail.clone())),
                    ),
                ])
            })
            .collect();
        let obj = J::Obj(vec![
            ("clean".into(), J::Bool(!findings)),
            ("contracts".into(), J::Arr(rows)),
            (
                "counts".into(),
                J::Obj(
                    counts
                        .iter()
                        .map(|(s, n)| (s.as_str().to_string(), J::Int(*n as i64)))
                        .collect(),
                ),
            ),
        ]);
        emit(&obj.dumps_indent2());
    } else {
        let rows: Vec<(String, &External)> = examined
            .iter()
            .flat_map(|e| {
                e.contracts
                    .iter()
                    .map(move |c| (format!("{} {}", e.name, c.id), c))
            })
            .collect();
        let width =
            |f: &dyn Fn(&(String, &External)) -> usize| rows.iter().map(f).max().unwrap_or(0);
        let (w_id, w_names) = (
            width(&|r| r.0.chars().count()),
            width(&|r| names(r.1).chars().count()),
        );
        for (id, c) in &rows {
            let mut line = format!(
                "{id:<w_id$}  {:<17}  {:<w_names$}  {}",
                c.state.as_str(),
                names(c),
                super::shown(&root, &c.file)
            );
            if c.state.is_finding() {
                line.push_str(&format!(" — {}", c.detail));
            }
            emit(&line);
        }
        if total == 0 {
            emit("no external contracts");
        } else {
            let parts: Vec<String> = counts
                .iter()
                .filter(|(_, n)| *n > 0)
                .map(|(s, n)| format!("{n} {}", s.as_str()))
                .collect();
            emit(&format!(
                "{total} external contract{}: {}",
                if total == 1 { "" } else { "s" },
                parts.join(" · ")
            ));
        }
    }
    Ok(match (read_code, findings) {
        (0, false) => 0,
        (0, true) => 2,
        (code, _) => code,
    })
}

/// One pin a sync rewrote.
struct Synced {
    sdd: String,
    id: String,
    file: String,
    authority: Option<(String, String)>,
    hash: Option<String>,
}

/// Per contract a sync rewrote, the units it staled.
type Moved = Vec<(String, Vec<String>)>;

/// The files a dry run would have written, with what it would have written.
type Overlay = Vec<(PathBuf, String)>;

/// The stamped units of `inst` whose recorded trace no longer matches one
/// of `ids` — per contract, in `ids` order. A dry run's contracts are read
/// as `overlay` has them, as the sync would have left them.
fn moved(inst: &Instance, ids: &[String], overlay: &Overlay) -> Result<Moved, CoreError> {
    let mut out: Moved = ids.iter().map(|c| (c.clone(), Vec::new())).collect();
    let mut would: Vec<(String, String)> = Vec::new();
    if !overlay.is_empty() {
        let read = |p: &Path| match overlay.iter().find(|(f, _)| f == p) {
            Some((_, text)) => Ok(text.clone()),
            None => model::read_text(p),
        };
        for cid in ids {
            if let Some(text) = model::contract_text_with(&inst.handle(), cid, &read)? {
                would.push((cid.clone(), trace::hash12(&text)));
            }
        }
    }
    for u in &inst.units {
        if !matches!(u.meta.get_str_or("status", "Draft"), "Built" | "Verified") {
            continue;
        }
        let Some(recorded) = u.meta.get_map("trace").filter(|m| !m.is_empty()) else {
            continue;
        };
        let (mut current, _) = trace::compute(inst, u)?;
        for (key, hash) in current.iter_mut() {
            if let Some((_, h)) = would.iter().find(|(c, _)| c == key) {
                *hash = h.clone();
            }
        }
        for mismatch in trace::verify(recorded, &current) {
            if let Some(slot) = out.iter_mut().find(|(c, _)| *c == mismatch.key) {
                slot.1.push(u.unit.clone());
            }
        }
    }
    out.retain(|(_, units)| !units.is_empty());
    Ok(out)
}

/// The change entry's line for what a sync staled (schema §12).
fn staleness_line(moved: &Moved) -> String {
    if moved.is_empty() {
        return "- Staleness applied: —".to_string();
    }
    let parts: Vec<String> = moved
        .iter()
        .map(|(c, units)| format!("{} → Stale (consume {c})", units.join(", ")))
        .collect();
    format!("- Staleness applied: {}", parts.join("; "))
}

pub fn sync(sdd: Option<&str>, all: bool, dry_run: bool, json: bool) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, sdd, all, &super::cwd())?;
    let peers = super::peers(&root);

    // a pin's authority is never itself a pin, so one pass puts every
    // stale pin in step
    let mut synced: Vec<Synced> = Vec::new();
    let mut overlay: Overlay = Vec::new();
    for sdd in &sdds {
        let Ok(examined) = examine(&root, sdd, &peers) else {
            continue; // reported below, when everything is examined again
        };
        let stale: Vec<&External> = examined
            .contracts
            .iter()
            .filter(|c| c.state == State::Stale && c.fix.is_some())
            .collect();
        let fixes: Vec<contract::Fix> = stale.iter().filter_map(|c| c.fix.clone()).collect();
        if fixes.is_empty() {
            continue;
        }
        if dry_run {
            overlay.extend(contract::rendered(&fixes)?);
        } else {
            contract::apply(&fixes)?;
        }
        for c in stale {
            synced.push(Synced {
                sdd: examined.name.clone(),
                id: c.id.clone(),
                file: super::shown(&root, &c.file),
                authority: c
                    .authority
                    .as_ref()
                    .map(|a| (a.slug.clone(), a.contract.clone())),
                hash: c.hash.clone(),
            });
        }
    }

    // what is left: nothing a sync can write, or it would have (a dry run
    // finds on disk the stale pins it did not write)
    let (examined, read_code) = examine_each(&root, &sdds, &peers);
    let would_sync = |e: &Examined, c: &External| {
        dry_run && synced.iter().any(|s| s.sdd == e.name && s.id == c.id)
    };
    let unresolved: Vec<(&Examined, &External)> = examined
        .iter()
        .flat_map(|e| e.contracts.iter().map(move |c| (e, c)))
        .filter(|(e, c)| c.state.is_finding() && !would_sync(e, c))
        .collect();
    // per consumer SDD: the stamped units the rewritten pins staled, and
    // the SDD's root as `--sdd` names it
    let mut staleness: Vec<(String, Moved, String)> = Vec::new();
    for e in &examined {
        let ids: Vec<String> = synced
            .iter()
            .filter(|s| s.sdd == e.name)
            .map(|s| s.id.clone())
            .collect();
        if !ids.is_empty() {
            staleness.push((
                e.name.clone(),
                moved(&e.inst, &ids, &overlay)?,
                super::shown(&root, &e.inst.root),
            ));
        }
    }

    if json {
        let obj = J::Obj(vec![
            (
                "synced".into(),
                J::Arr(
                    synced
                        .iter()
                        .map(|s| {
                            J::Obj(vec![
                                ("sdd".into(), J::s(s.sdd.clone())),
                                ("contract".into(), J::s(s.id.clone())),
                                ("file".into(), J::s(s.file.clone())),
                                (
                                    "authority".into(),
                                    match &s.authority {
                                        Some((slug, id)) => J::Obj(vec![
                                            ("sdd".into(), J::s(slug.clone())),
                                            ("contract".into(), J::s(id.clone())),
                                        ]),
                                        None => J::Null,
                                    },
                                ),
                                ("hash".into(), J::opt_str(s.hash.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "staleness".into(),
                J::Arr(
                    staleness
                        .iter()
                        .map(|(name, moved, _)| {
                            J::Obj(vec![
                                ("sdd".into(), J::s(name.clone())),
                                (
                                    "stale".into(),
                                    J::Arr(
                                        moved
                                            .iter()
                                            .map(|(c, units)| {
                                                J::Obj(vec![
                                                    ("contract".into(), J::s(c.clone())),
                                                    (
                                                        "units".into(),
                                                        J::Arr(
                                                            units
                                                                .iter()
                                                                .map(|u| J::s(u.clone()))
                                                                .collect(),
                                                        ),
                                                    ),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                                ("line".into(), J::s(staleness_line(moved))),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "unresolved".into(),
                J::Arr(
                    unresolved
                        .iter()
                        .map(|(e, c)| {
                            J::Obj(vec![
                                ("sdd".into(), J::s(e.name.clone())),
                                ("contract".into(), J::s(c.id.clone())),
                                ("state".into(), J::s(c.state.as_str())),
                                ("detail".into(), J::s(c.detail.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("dry_run".into(), J::Bool(dry_run)),
        ]);
        emit(&obj.dumps_indent2());
    } else {
        for s in &synced {
            let from = s
                .authority
                .as_ref()
                .map(|(slug, id)| format!("{slug} {id}"))
                .unwrap_or_default();
            emit(&format!("synced {} {} ← {from}  {}", s.sdd, s.id, s.file));
        }
        for (name, moved, _) in &staleness {
            emit(&format!("{name}: {}", staleness_line(moved)));
        }
        for (e, c) in &unresolved {
            diag(&format!("not synced: {} {}: {}", e.name, c.id, c.detail));
        }
        let in_step = examined
            .iter()
            .flat_map(|e| &e.contracts)
            .filter(|c| c.state == State::Ok)
            .count();
        emit(&match (synced.len(), unresolved.len()) {
            (0, 0) => format!("nothing to sync ({in_step} in step)"),
            (n, 0) => format!("{n} synced"),
            (n, m) => format!("{n} synced · {m} not synced"),
        });
    }
    if dry_run {
        diag("dry run: nothing written");
    } else {
        // the next step, once per consumer SDD whose pins moved
        for (name, _, at) in &staleness {
            diag(&format!(
                "next: write {name}'s change entry around its line, then run vitrify change apply --sdd {at}"
            ));
        }
    }
    Ok(match (read_code, unresolved.is_empty()) {
        (0, true) => 0,
        (0, false) => 2,
        (code, _) => code,
    })
}
