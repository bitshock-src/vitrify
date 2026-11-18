//! `vitrify questions` — every `Q-###` of an SDD's Decision Log, whether it
//! is open, and the units it blocks. A read verb.
//!
//! A question is open unless its entry marks it resolved
//! ([`vitrify_core::questions`] — the rule lives there and nowhere else).
//! The units it blocks are those whose entry carries its `OPEN(Q-###)`
//! marker, and — for an open question a run landed — the unit it was
//! reported on: that unit's worker stopped at it, and no marker says so,
//! since a worker may not edit spec text.

use vitrify_core::CoreError;
use vitrify_core::model::{self, Instance, SddHandle, Shape};
use vitrify_core::questions::{self, Question};

use crate::out::{J, emit};

/// One question with what it blocks.
struct Row {
    sdd: String,
    q: Question,
    blocks: Vec<String>,
}

fn unit_text(inst: &Instance, u: &model::Unit) -> Result<String, CoreError> {
    match inst.shape {
        Shape::Bundle => model::read_text(&u.file),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            Ok(model::find_unit_span(&text, &u.unit)
                .map(|(s, e)| text[s..e].to_string())
                .unwrap_or_default())
        }
    }
}

fn rows_of(name: &str, sdd: &SddHandle) -> Result<Vec<Row>, CoreError> {
    let inst = Instance::load(sdd.clone())?;
    let mut marked: Vec<(String, Vec<String>)> = Vec::new();
    for u in &inst.units {
        marked.push((u.unit.clone(), questions::markers(&unit_text(&inst, u)?)));
    }
    Ok(questions::of(sdd)?
        .into_iter()
        .map(|q| {
            let mut blocks: Vec<String> = marked
                .iter()
                .filter(|(_, qs)| qs.contains(&q.id))
                .map(|(u, _)| u.clone())
                .collect();
            if q.open
                && let Some(unit) = questions::reported_on(&q.text)
                && !blocks.contains(&unit)
            {
                blocks.push(unit);
                blocks.sort();
            }
            Row {
                sdd: name.to_string(),
                q,
                blocks,
            }
        })
        .collect())
}

/// The open questions of `sdd`, each with the units it blocks.
pub fn open(sdd: &SddHandle) -> Result<Vec<(String, Vec<String>)>, CoreError> {
    Ok(rows_of("", sdd)?
        .into_iter()
        .filter(|r| r.q.open)
        .map(|r| (r.q.id, r.blocks))
        .collect())
}

/// `text` cut to `max` characters, on one line.
fn brief(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

pub fn run(sdd: Option<&str>, all: bool, open_only: bool, json: bool) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, sdd, all, &super::cwd())?;
    let mut rows: Vec<Row> = Vec::new();
    let code = super::each(&root, &sdds, |sdd| {
        rows.extend(rows_of(&super::label(sdd, &root), sdd)?);
        Ok(0)
    });
    if !all && code != 0 {
        return Ok(code); // the one SDD could not be read: already said
    }
    let n_open = rows.iter().filter(|r| r.q.open).count();
    let n_resolved = rows.len() - n_open;
    if open_only {
        rows.retain(|r| r.q.open);
    }

    let status = |q: &Question| if q.open { "open" } else { "resolved" };
    if json {
        let obj = J::Obj(vec![
            ("open".into(), J::Int(n_open as i64)),
            ("resolved".into(), J::Int(n_resolved as i64)),
            (
                "questions".into(),
                J::Arr(
                    rows.iter()
                        .map(|r| {
                            J::Obj(vec![
                                ("sdd".into(), J::s(r.sdd.clone())),
                                ("q".into(), J::s(r.q.id.clone())),
                                ("status".into(), J::s(status(&r.q))),
                                (
                                    "blocks".into(),
                                    J::Arr(r.blocks.iter().map(|u| J::s(u.clone())).collect()),
                                ),
                                ("noted".into(), J::s(r.q.meta.clone())),
                                ("text".into(), J::s(r.q.text.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        emit(&obj.dumps_indent2());
    } else {
        let w_id = rows
            .iter()
            .map(|r| r.q.id.len() + if all { r.sdd.chars().count() + 1 } else { 0 })
            .max()
            .unwrap_or(0);
        let w_blocks = rows
            .iter()
            .map(|r| r.blocks.join(", ").len().max(1))
            .max()
            .unwrap_or(1);
        for r in &rows {
            let id = if all {
                format!("{} {}", r.sdd, r.q.id)
            } else {
                r.q.id.clone()
            };
            let blocks = if r.blocks.is_empty() {
                "—".to_string()
            } else {
                r.blocks.join(", ")
            };
            // `—` is one column wide and three bytes long: pad by characters
            let pad = " ".repeat(w_blocks.saturating_sub(blocks.chars().count()));
            emit(&format!(
                "{id:<w_id$}  {:<8}  blocks {blocks}{pad}  {}",
                status(&r.q),
                brief(&r.q.text, 100)
            ));
        }
        emit(&format!("{n_open} open, {n_resolved} resolved"));
    }
    Ok(code)
}
