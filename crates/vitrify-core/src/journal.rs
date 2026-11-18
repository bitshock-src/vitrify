//! Journal: append-only NDJSON event log, held to the golden corpus
//! (`fixtures/golden/`). One compact JSON object per line, separators
//! `(",", ":")`, non-ASCII unescaped, insertion key order `seq, at, event,
//! [unit], [actor], [claim_seq], [entry], [stale], [cause], [findings], sdd,
//! [commit]`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::model::{self, SddHandle, Shape, Unit};

/// The closed event vocabulary.
pub const EVENTS: [&str; 9] = [
    "claim", "renew", "release", "built", "verified", "stale", "retired", "change", "audit",
];

/// One journal event. Field declaration order IS the key order —
/// serde emits struct fields in declaration order (Conventions: normative
/// JSON is never serialized from maps).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Event {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<i64>,
    #[serde(default)]
    pub sdd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// Render one NDJSON line (no trailing newline). Compact separators
/// `(",", ":")` and unescaped non-ASCII — the journal line format.
pub fn render_line(event: &Event) -> String {
    // invariant: Event serialization cannot fail (no non-string keys, no NaN)
    serde_json::to_string(event).expect("event serializes")
}

pub fn parse_line(line: &str) -> Result<Event, CoreError> {
    serde_json::from_str(line)
        .map_err(|e| CoreError::refusal(1, format!("error: invalid journal line: {e}")))
}

fn find_journal_fence(text: &str) -> Option<(usize, usize)> {
    // The journal fence: `## Journal`, then the first code fence's inner span —
    // Returns (content_start, content_end) = the group(2) span.
    let (sec_start, _) = heading_line(text, "Journal")?;
    let after_heading = heading_line_end(text, sec_start);
    let open_rel = text[after_heading..].find("```")?;
    let open = after_heading + open_rel;
    // skip the rest of the fence-open line
    let nl = text[open..].find('\n')?;
    let content_start = open + nl + 1;
    // first subsequent line that is ``` plus only whitespace
    let mut pos = content_start;
    loop {
        let line_end = text[pos..].find('\n').map(|i| pos + i);
        let content = match line_end {
            Some(e) => &text[pos..e],
            None => &text[pos..],
        };
        let trimmed = content.trim_end_matches(['\r']);
        if let Some(rest) = trimmed.strip_prefix("```")
            && rest.trim().is_empty()
        {
            return Some((content_start, pos));
        }
        pos = line_end? + 1;
    }
}

fn heading_line(text: &str, heading: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.strip_prefix("##").is_some_and(|rest| {
            let t = rest.trim_start();
            t.len() < rest.len() && t.strip_prefix(heading).is_some_and(|x| x.trim().is_empty())
        }) {
            return Some((pos, pos + line.len()));
        }
        pos += line.len();
    }
    None
}

fn heading_line_end(text: &str, line_start: usize) -> usize {
    text[line_start..]
        .find('\n')
        .map(|i| line_start + i + 1)
        .unwrap_or(text.len())
}

/// Read every journal event: `(events, exists)`. Bundle — `journal.ndjson`;
/// single — the fenced block under `## Journal`.
pub fn read(sdd: &SddHandle) -> Result<(Vec<Event>, bool), CoreError> {
    let raw = match sdd.shape {
        Shape::Bundle => {
            let p = sdd.root.join("journal.ndjson");
            if !p.exists() {
                return Ok((Vec::new(), false));
            }
            model::read_text(&p)?
        }
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            match find_journal_fence(&text) {
                Some((s, e)) => text[s..e].to_string(),
                None => return Ok((Vec::new(), false)),
            }
        }
    };
    let mut events = Vec::new();
    for ln in crate::newline::split_universal(&raw) {
        let ln = ln.trim();
        if !ln.is_empty() {
            events.push(parse_line(ln)?);
        }
    }
    Ok((events, true))
}

/// The `seq` the next event appended after `events` carries.
pub fn next_seq(events: &[Event]) -> i64 {
    events.last().map(|e| e.seq).unwrap_or(0) + 1
}

/// Whether `events` hold the `change` event of the change entry `entry`
/// (`CH-###`).
pub fn records_change(events: &[Event], entry: &str) -> bool {
    events
        .iter()
        .any(|e| e.event == "change" && e.entry.as_deref() == Some(entry))
}

/// Append one rendered line. Bundle — append to `journal.ndjson`; single —
/// insert before the closing fence, creating the `## Journal` section with
/// its fence when absent.
pub fn append_line(sdd: &SddHandle, line: &str) -> Result<PathBuf, CoreError> {
    match sdd.shape {
        Shape::Bundle => {
            let p = sdd.root.join("journal.ndjson");
            let mut text = if p.exists() {
                model::read_text(&p)?
            } else {
                String::new()
            };
            text.push_str(line);
            text.push('\n');
            model::write_text(&p, &text)?;
            Ok(p)
        }
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            let new = match find_journal_fence(&text) {
                Some((_, content_end)) => {
                    format!("{}{}\n{}", &text[..content_end], line, &text[content_end..])
                }
                None => format!(
                    "{}\n\n## Journal\n\n```\n{}\n```\n",
                    text.trim_end_matches('\n'),
                    line
                ),
            };
            model::write_text(&sdd.file, &new)?;
            Ok(sdd.file.clone())
        }
    }
}

/// Replay projection: per-unit status and claim_seq in first-touch order,
/// plus order findings (non-increasing seq, unknown events).
#[derive(Debug, Clone, Default)]
pub struct Projection {
    pub status: Vec<(String, String)>,
    pub seq: Vec<(String, i64)>,
    pub order_findings: Vec<String>,
}

impl Projection {
    pub fn status_of(&self, unit: &str) -> Option<&str> {
        self.status
            .iter()
            .find(|(u, _)| u == unit)
            .map(|(_, s)| s.as_str())
    }

    pub fn seq_of(&self, unit: &str) -> Option<i64> {
        self.seq.iter().find(|(u, _)| u == unit).map(|(_, s)| *s)
    }

    fn set_status(&mut self, unit: &str, status: &str) {
        match self.status.iter_mut().find(|(u, _)| u == unit) {
            Some(slot) => slot.1 = status.to_string(),
            None => self.status.push((unit.to_string(), status.to_string())),
        }
    }

    fn set_seq(&mut self, unit: &str, seq: i64) {
        match self.seq.iter_mut().find(|(u, _)| u == unit) {
            Some(slot) => slot.1 = seq,
            None => self.seq.push((unit.to_string(), seq)),
        }
    }
}

/// The projection rules.
pub fn replay(events: &[Event]) -> Projection {
    let mut proj = Projection::default();
    let mut last = 0i64;
    for e in events {
        if e.seq <= last {
            proj.order_findings
                .push(format!("seq not strictly increasing at seq={}", e.seq));
        }
        last = e.seq;
        let unit = e.unit.as_deref();
        match (e.event.as_str(), unit) {
            ("claim", Some(u)) => {
                proj.set_status(u, "Claimed");
                let prev = proj.seq_of(u).unwrap_or(0);
                proj.set_seq(u, e.claim_seq.unwrap_or(prev));
            }
            ("renew", _) => {}
            ("release", Some(u)) => proj.set_status(u, "Ready"),
            ("built", Some(u)) => proj.set_status(u, "Built"),
            ("verified", Some(u)) => proj.set_status(u, "Verified"),
            ("stale", Some(u)) => proj.set_status(u, "Stale"),
            ("retired", Some(u)) => proj.set_status(u, "Retired"),
            ("change", _) => {
                for su in e.stale.iter().flatten() {
                    proj.set_status(su, "Stale");
                }
            }
            ("audit", _) => {}
            ("claim" | "release" | "built" | "verified" | "stale" | "retired", None) => {}
            (ev, _) => proj
                .order_findings
                .push(format!("unknown event '{ev}' at seq={}", e.seq)),
        }
    }
    proj
}

/// One projection-vs-front-matter mismatch.
#[derive(Debug, Clone)]
pub struct VerifyMismatch {
    pub unit: String,
    pub field: &'static str,
    pub projected: FieldVal,
    pub current: FieldVal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldVal {
    Str(String),
    Int(i64),
}

impl FieldVal {
    /// Single-quoted rendering for mismatch diagnostics (`'Built'`).
    pub fn repr(&self) -> String {
        match self {
            FieldVal::Str(s) => format!("'{s}'"),
            FieldVal::Int(i) => i.to_string(),
        }
    }
}

/// Compare projected status and claim_seq of every unit against current
/// front matter; units with no events must be Draft or Ready.
pub fn verify(proj: &Projection, units: &[Unit]) -> Vec<VerifyMismatch> {
    let mut mismatches = Vec::new();
    for u in units {
        let cur_status = u.meta.get_str_or("status", "Draft").to_string();
        let cur_seq = u.meta.get_int("claim_seq");
        if let Some(projected) = proj.status_of(&u.unit) {
            if cur_status != projected {
                mismatches.push(VerifyMismatch {
                    unit: u.unit.clone(),
                    field: "status",
                    projected: FieldVal::Str(projected.to_string()),
                    current: FieldVal::Str(cur_status.clone()),
                });
            }
            if let Some(pseq) = proj.seq_of(&u.unit)
                && cur_seq != pseq
            {
                mismatches.push(VerifyMismatch {
                    unit: u.unit.clone(),
                    field: "claim_seq",
                    projected: FieldVal::Int(pseq),
                    current: FieldVal::Int(cur_seq),
                });
            }
        } else if cur_status != "Draft" && cur_status != "Ready" {
            mismatches.push(VerifyMismatch {
                unit: u.unit.clone(),
                field: "status",
                projected: FieldVal::Str("Draft|Ready (no events)".to_string()),
                current: FieldVal::Str(cur_status),
            });
        }
    }
    mismatches
}
