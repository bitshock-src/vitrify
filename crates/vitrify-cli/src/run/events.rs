//! What happened to a run, one event at a time: the lines the run appends
//! to `events.jsonl` as things happen, and the lines `vitrify run status
//! --follow` prints. One shape serves both, so a follower can replay what
//! the run wrote and then carry on with what it sees for itself.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use vitrify_core::model::UtcTime;

use crate::out::{J, diag};

/// One thing that happened to a run or to one of its units. (Not `Event`:
/// the scheduler has a private enum of that name for its own channel.)
#[derive(Debug, Clone, PartialEq)]
pub struct RunEvent {
    pub at: UtcTime,
    /// The kind: `dispatched`, `outcome`, `gate`, `run-ended`, …
    pub kind: String,
    pub unit: Option<String>,
    pub claim_seq: Option<i64>,
    /// Human-readable text, possibly empty.
    pub detail: String,
    /// The fields of this kind. Written in key order, whatever order they
    /// were added in.
    pub fields: Vec<(String, J)>,
}

/// The keys every event has; any other key is a field of its kind.
const COMMON: [&str; 6] = ["at", "event", "unit", "claim_seq", "detail", "replay"];

impl RunEvent {
    pub fn new(
        at: UtcTime,
        kind: &str,
        unit: Option<&str>,
        claim_seq: Option<i64>,
        detail: impl Into<String>,
    ) -> RunEvent {
        RunEvent {
            at,
            kind: kind.to_string(),
            unit: unit.map(str::to_string),
            claim_seq,
            detail: detail.into(),
            fields: Vec::new(),
        }
    }

    /// The event with one more field of its kind.
    pub fn with(mut self, key: &str, value: J) -> RunEvent {
        self.fields.push((key.to_string(), value));
        self
    }

    /// A field of this kind, if it has it.
    pub fn field(&self, key: &str) -> Option<&J> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The event as one JSON object: the common keys first, then the
    /// fields of its kind in key order. `replay` is for `run status` output
    /// only (whether the line was read back from `events.jsonl`); the run's
    /// own log is written with `None`.
    pub fn to_json(&self, replay: Option<bool>) -> J {
        let mut pairs = vec![
            ("at".to_string(), J::s(self.at.format())),
            ("event".to_string(), J::s(self.kind.clone())),
            ("unit".to_string(), J::opt_str(self.unit.clone())),
            (
                "claim_seq".to_string(),
                self.claim_seq.map(J::Int).unwrap_or(J::Null),
            ),
            ("detail".to_string(), J::s(self.detail.clone())),
        ];
        let mut fields = self.fields.clone();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        pairs.extend(fields);
        if let Some(replay) = replay {
            pairs.push(("replay".to_string(), J::Bool(replay)));
        }
        J::Obj(pairs)
    }

    /// An event read back from one line of `events.jsonl`. `None` for
    /// anything that is not an event: a line cut short by a crash, or one
    /// with no time or kind. A missing common key takes its default.
    pub fn from_json(v: &serde_json::Value) -> Option<RunEvent> {
        let obj = v.as_object()?;
        let at = UtcTime::parse(obj.get("at")?.as_str()?)?;
        let kind = obj.get("event")?.as_str()?.to_string();
        let mut fields: Vec<(String, J)> = obj
            .iter()
            .filter(|(k, _)| !COMMON.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), J::from_value(v)))
            .collect();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        Some(RunEvent {
            at,
            kind,
            unit: obj.get("unit").and_then(|u| u.as_str()).map(str::to_string),
            claim_seq: obj.get("claim_seq").and_then(|c| c.as_i64()),
            detail: obj
                .get("detail")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            fields,
        })
    }
}

/// The run's event log, `<logs>/events.jsonl`: appended to as things
/// happen, never rewritten. Shared by the scheduler and the integrator's
/// gate board; each line goes out in one write under the lock, so lines
/// from the two threads never interleave. Evidence for a reader, not state:
/// nothing reads it back to decide anything, and a log that cannot be
/// written does not stop the run.
#[derive(Clone)]
pub struct EventLog(Arc<Mutex<Option<File>>>);

impl EventLog {
    /// Create or append to `events.jsonl` in `logs_dir`. A log that cannot
    /// be opened is said once and then disabled.
    pub fn open(logs_dir: &Path) -> EventLog {
        let path = logs_dir.join("events.jsonl");
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => EventLog(Arc::new(Mutex::new(Some(file)))),
            Err(e) => {
                diag(&format!("warning: events.jsonl not written: {e}"));
                EventLog::off()
            }
        }
    }

    /// A log that writes nothing (tests, and integrators run outside `run`).
    pub fn off() -> EventLog {
        EventLog(Arc::new(Mutex::new(None)))
    }

    /// Append one event. Errors are ignored: the log is evidence, and a run
    /// does not stop for want of it.
    pub fn write(&self, e: &RunEvent) {
        let line = format!("{}\n", e.to_json(None).dumps_default());
        let mut file = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(f) = file.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RunEvent {
        RunEvent::new(
            UtcTime::parse("2026-11-16T14:08:01Z").unwrap(),
            "outcome",
            Some("U-004"),
            Some(1),
            "rejected-gate — failing gate: `pytest -q`",
        )
        .with("outcome", J::s("rejected-gate"))
        .with("duration_s", J::Int(660))
    }

    #[test]
    fn fields_follow_the_common_keys_in_key_order() {
        let line = sample().to_json(Some(false)).dumps_default();
        assert_eq!(
            line,
            r#"{"at": "2026-11-16T14:08:01Z", "event": "outcome", "unit": "U-004", "claim_seq": 1, "detail": "rejected-gate \u2014 failing gate: `pytest -q`", "duration_s": 660, "outcome": "rejected-gate", "replay": false}"#
        );
        assert!(!sample().to_json(None).dumps_default().contains("replay"));
    }

    #[test]
    fn an_event_survives_a_round_trip_field_order_included() {
        let mut e = sample();
        e.fields.sort_by(|a, b| a.0.cmp(&b.0));
        let back: serde_json::Value =
            serde_json::from_str(&e.to_json(None).dumps_default()).unwrap();
        assert_eq!(RunEvent::from_json(&back), Some(e));

        let run =
            RunEvent::new(UtcTime(0), "run-ended", None, None, "").with("state", J::s("complete"));
        let back: serde_json::Value =
            serde_json::from_str(&run.to_json(Some(true)).dumps_default()).unwrap();
        assert_eq!(RunEvent::from_json(&back), Some(run));
    }

    #[test]
    fn a_line_that_is_not_an_event_is_skipped() {
        for text in [
            r#"{"event": "gate"}"#,
            r#"{"at": "2026-11-16T14:08:01Z"}"#,
            "[]",
        ] {
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(RunEvent::from_json(&v), None, "{text}");
        }
    }

    #[test]
    fn the_log_appends_one_line_per_event() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path());
        log.write(&sample());
        log.clone().write(&sample());
        EventLog::off().write(&sample());
        let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with("}\n"));
    }
}
