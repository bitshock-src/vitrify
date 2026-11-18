//! The gate board: what the integrator is doing now, rewritten whole to
//! `<logs>/gate.json` at every change, so `vitrify run status` can show a
//! gate that is otherwise silent while it passes. For observability only:
//! nothing reads it back to decide anything.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use vitrify_core::model::UtcTime;

use crate::gate;
use crate::out::J;
use crate::run::events::{EventLog, RunEvent};

/// A handle on the board. Clones share it: the integrator's commands report
/// to it from the threads that run them.
#[derive(Clone)]
pub struct GateBoard {
    inner: Arc<Mutex<Board>>,
}

/// What the board holds. Kept under one lock with the file write, so the
/// file always shows one consistent moment.
struct Board {
    /// Where the board is written; `None` writes nothing.
    path: Option<PathBuf>,
    events: EventLog,
    step: &'static str,
    since: UtcTime,
    batch: Vec<String>,
    /// Commands in progress with when each began, oldest first.
    running: Vec<(String, String)>,
    /// While counting a pass: how many distinct commands it has, and those
    /// finished so far (a failure confirmed alone counts once).
    count: Option<(usize, HashSet<String>)>,
}

impl Board {
    /// Begin step `name`; every step but `idle` is also a `gate` event.
    fn begin(&mut self, name: &'static str) {
        self.step = name;
        self.since = UtcTime::now();
        self.running.clear();
        self.count = None;
        if name == "idle" {
            return;
        }
        let mut detail = name.to_string();
        if !self.batch.is_empty() {
            detail = format!("{detail} · {}", self.batch.join(", "));
        }
        let units = self.batch.iter().map(|u| J::s(u.clone())).collect();
        self.events.write(
            &RunEvent::new(UtcTime::now(), "gate", None, None, detail)
                .with("batch", J::Arr(units))
                .with("step", J::s(name)),
        );
    }

    /// The board as `gate.json` holds it.
    fn to_json(&self) -> J {
        let (done, total) = match &self.count {
            Some((total, done)) => (J::Int(done.len() as i64), J::Int(*total as i64)),
            None => (J::Null, J::Null),
        };
        let running = self
            .running
            .iter()
            .map(|(cmd, since)| {
                J::Obj(vec![
                    ("command".to_string(), J::s(cmd.clone())),
                    ("since".to_string(), J::s(since.clone())),
                ])
            })
            .collect();
        J::Obj(vec![
            ("step".to_string(), J::s(self.step)),
            ("since".to_string(), J::s(self.since.format())),
            (
                "batch".to_string(),
                J::Arr(self.batch.iter().map(|u| J::s(u.clone())).collect()),
            ),
            ("running".to_string(), J::Arr(running)),
            ("done".to_string(), done),
            ("total".to_string(), total),
        ])
    }

    /// Rewrite the file whole (tmp + rename), so a reader never sees half a
    /// board. Errors are ignored: the board is evidence, not state.
    fn write(&self) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_file_name("gate.json.tmp");
        if std::fs::write(&tmp, self.to_json().dumps_indent2()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

impl GateBoard {
    /// A board at rest, written to `path` if there is one.
    fn new(path: Option<PathBuf>) -> GateBoard {
        GateBoard {
            inner: Arc::new(Mutex::new(Board {
                path,
                events: EventLog::off(),
                step: "idle",
                since: UtcTime::now(),
                batch: Vec::new(),
                running: Vec::new(),
                count: None,
            })),
        }
    }

    /// A board that records nothing.
    pub fn off() -> GateBoard {
        GateBoard::new(None)
    }

    /// A board written to `path`.
    pub fn to(path: PathBuf) -> GateBoard {
        GateBoard::new(Some(path))
    }

    /// The board, also appending a `gate` event to `events` at each step.
    pub fn with_events(self, events: EventLog) -> GateBoard {
        self.lock().events = events;
        self
    }

    /// The board, even if a thread panicked holding it: what it shows is
    /// only ever evidence.
    fn lock(&self) -> MutexGuard<'_, Board> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Change the board and write it out, under the lock, so concurrent
    /// commands never leave the file behind the board.
    fn change(&self, f: impl FnOnce(&mut Board)) {
        let mut board = self.lock();
        f(&mut board);
        board.write();
    }

    /// The units being integrated now; kept across steps.
    pub fn batch(&self, units: Vec<String>) {
        self.change(|b| b.batch = units);
    }

    /// A new step begins.
    pub fn step(&self, name: &'static str) {
        self.change(|b| b.begin(name));
    }

    /// Start counting the commands of a pass of `total` distinct commands.
    pub fn rows(&self, total: usize) {
        self.change(|b| b.count = Some((total, HashSet::new())));
    }

    /// Nothing is being integrated.
    pub fn idle(&self) {
        self.change(|b| {
            b.batch.clear();
            b.begin("idle");
        });
    }
}

impl gate::Watch for GateBoard {
    fn started(&self, cmd: &str) {
        self.change(|b| b.running.push((cmd.to_string(), UtcTime::now().format())));
    }

    fn ended(&self, cmd: &str) {
        self.change(|b| {
            if let Some(at) = b.running.iter().position(|(c, _)| c == cmd) {
                b.running.remove(at);
            }
            // A pass counts its own commands only. One that runs after it in
            // the same step — the head check's Test command, after the rows —
            // is not one of them, and must not push `done` past `total`.
            if let Some((total, done)) = &mut b.count
                && done.len() < *total
            {
                done.insert(cmd.to_string());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::Watch;

    fn read(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn running(v: &serde_json::Value) -> Vec<String> {
        v["running"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["command"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn the_file_follows_each_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gate.json");
        let board = GateBoard::to(path.clone());

        board.batch(vec!["U-002".into(), "U-005".into()]);
        board.step("rows");
        let v = read(&path);
        assert_eq!(v["step"], "rows");
        assert_eq!(v["batch"], serde_json::json!(["U-002", "U-005"]));
        assert_eq!(v["running"], serde_json::json!([]));
        assert!(v["done"].is_null() && v["total"].is_null());
        assert!(UtcTime::parse(v["since"].as_str().unwrap()).is_some());

        board.rows(2);
        board.started("a");
        board.started("b");
        let v = read(&path);
        assert_eq!(
            (v["done"].as_i64(), v["total"].as_i64()),
            (Some(0), Some(2))
        );
        assert_eq!(running(&v), ["a", "b"]);
        assert!(UtcTime::parse(v["running"][0]["since"].as_str().unwrap()).is_some());

        board.ended("a");
        board.started("a"); // a failure confirmed alone counts once
        board.ended("a");
        let v = read(&path);
        assert_eq!(v["done"], 1);
        assert_eq!(running(&v), ["b"]);

        // a command after the pass, in the same step, is not one of its rows
        board.ended("b");
        board.started("make test");
        board.ended("make test");
        let v = read(&path);
        assert_eq!(
            (v["done"].as_i64(), v["total"].as_i64()),
            (Some(2), Some(2))
        );

        board.step("lint");
        let v = read(&path);
        assert_eq!(v["running"], serde_json::json!([]));
        assert!(v["done"].is_null() && v["total"].is_null());
        assert_eq!(v["batch"], serde_json::json!(["U-002", "U-005"]));

        board.idle();
        let v = read(&path);
        assert_eq!(v["step"], "idle");
        assert_eq!(v["batch"], serde_json::json!([]));

        // the key order is part of the format
        let text = std::fs::read_to_string(&path).unwrap();
        let at = |k: &str| text.find(&format!("\"{k}\"")).unwrap();
        assert!(at("step") < at("since") && at("since") < at("batch"));
        assert!(at("batch") < at("running") && at("running") < at("done"));
        assert!(at("done") < at("total"));
        assert!(!dir.path().join("gate.json.tmp").exists());
    }

    #[test]
    fn commands_started_at_once_from_many_threads_are_all_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gate.json");
        let board = GateBoard::to(path.clone());
        board.step("rows");
        std::thread::scope(|s| {
            for t in 0..8 {
                let board = board.clone();
                s.spawn(move || {
                    for k in 0..10 {
                        board.started(&format!("cmd {t}-{k}"));
                    }
                });
            }
        });
        let mut got = running(&read(&path));
        got.sort();
        let mut want: Vec<String> = (0..8)
            .flat_map(|t| (0..10).map(move |k| format!("cmd {t}-{k}")))
            .collect();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn each_step_but_idle_is_a_gate_event() {
        let dir = tempfile::tempdir().unwrap();
        let board =
            GateBoard::to(dir.path().join("gate.json")).with_events(EventLog::open(dir.path()));
        board.batch(vec!["U-002".into(), "U-005".into()]);
        board.step("rows");
        board.idle();
        board.step("head-check");
        let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        let events: Vec<RunEvent> = text
            .lines()
            .map(|l| RunEvent::from_json(&serde_json::from_str(l).unwrap()).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "gate");
        assert_eq!(events[0].detail, "rows · U-002, U-005");
        assert_eq!(events[0].field("step"), Some(&J::s("rows")));
        assert_eq!(
            events[0].field("batch"),
            Some(&J::Arr(vec![J::s("U-002"), J::s("U-005")]))
        );
        assert_eq!(events[1].detail, "head-check");
        assert_eq!(events[1].field("batch"), Some(&J::Arr(vec![])));
    }
}
