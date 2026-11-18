//! `vitrify run status --follow`: look again every interval and print one
//! line per change, until the run ends.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use vitrify_core::CoreError;
use vitrify_core::model::UtcTime;

use super::render;
use super::snapshot::{RunState, Snapshot, WorkerView};
use crate::cli::StatusArgs;
use crate::out::{J, emit_checked};
use crate::run::events::RunEvent;

/// The stages a running worker moves through on its own. `unknown` is not
/// one of them: a probe that could not tell is not a change.
const RUNNING_STAGES: [&str; 4] = ["reading", "editing", "stamped", "exited"];

/// How long work counts must stand before a new report: a worker that
/// writes steadily would otherwise print a line every tick.
const WORK_EVERY_S: i64 = 60;

/// How long snapshots may fail in a row before the follower gives up: a
/// run rewrites its record at every change, so one bad read is no reason to
/// stop.
const GIVE_UP_AFTER_S: i64 = 60;

/// What the follower remembers between looks beyond the last snapshot.
pub struct Tracker {
    /// Per holder: the work counts last reported, and when.
    reported: HashMap<String, ((u64, u64, u64), UtcTime)>,
    /// The holders flagged quiet now, so each quiet spell is said once.
    quiet: HashSet<String>,
    /// Per holder: the last running stage a probe could tell, so a stage
    /// seen again after an `unknown` is compared with what came before.
    stage: HashMap<String, String>,
    /// Whether `quiet` has been filled from the first snapshot. That needs
    /// the quiet threshold, which only `diff` is given.
    primed: bool,
}

impl Tracker {
    /// Start from the first snapshot: what it shows was already printed, so
    /// it counts as reported.
    pub fn new(first: &Snapshot) -> Tracker {
        let mut t = Tracker {
            reported: HashMap::new(),
            quiet: HashSet::new(),
            stage: HashMap::new(),
            primed: false,
        };
        for w in first.workers.iter().filter(|w| w.outcome == "running") {
            if let Some(counts) = counts(w) {
                t.reported.insert(w.holder.clone(), (counts, first.at));
            }
            if RUNNING_STAGES.contains(&w.stage.as_str()) {
                t.stage.insert(w.holder.clone(), w.stage.clone());
            }
        }
        t
    }

    /// Mark the workers of the first snapshot that were already quiet, so
    /// the follower does not repeat what the one-shot view just warned of.
    fn prime(&mut self, first: &Snapshot, quiet_after: u64) {
        if self.primed {
            return;
        }
        self.primed = true;
        for w in &first.workers {
            if is_quiet(w, quiet_after) {
                self.quiet.insert(w.holder.clone());
            }
        }
    }
}

/// A worker's work counts, when the probe found all three.
fn counts(w: &WorkerView) -> Option<(u64, u64, u64)> {
    Some((w.files_changed?, w.insertions?, w.deletions?))
}

/// Whether a worker is running, alive, and silent for at least
/// `quiet_after` seconds.
fn is_quiet(w: &WorkerView, quiet_after: u64) -> bool {
    w.outcome == "running"
        && w.alive == Some(true)
        && w.log_quiet_s.is_some_and(|q| q >= quiet_after as i64)
}

/// Whether an outcome is final: the attempt is over.
fn is_final(outcome: &str) -> bool {
    outcome != "running" && outcome != "integrating"
}

/// The events between two looks at a run.
pub fn diff(prev: &Snapshot, next: &Snapshot, t: &mut Tracker, quiet_after: u64) -> Vec<RunEvent> {
    t.prime(prev, quiet_after);
    let mut events = Vec::new();
    for w in &next.workers {
        worker_events(prev, next, w, t, quiet_after, &mut events);
    }

    if next.state == RunState::Alive
        && let Some(gate) = &next.gate
    {
        let before = prev.gate.as_ref().map(|g| (&g.step, &g.batch));
        if gate.step != "idle" && before != Some((&gate.step, &gate.batch)) {
            events.push(gate_event(next.at, &gate.step, &gate.batch));
        }
    }
    if prev.state == RunState::Alive && next.state == RunState::Died {
        events.push(died_event(next));
    }
    if next.state == RunState::Ended
        && prev.state != RunState::Ended
        && let Some(e) = ended_event(next)
    {
        events.push(e);
    }
    events
}

/// The events of one worker between two looks.
fn worker_events(
    prev: &Snapshot,
    next: &Snapshot,
    w: &WorkerView,
    t: &mut Tracker,
    quiet_after: u64,
    events: &mut Vec<RunEvent>,
) {
    let unit = Some(w.unit.as_str());
    let seq = Some(w.claim_seq);
    let before = prev.workers.iter().find(|p| p.holder == w.holder);
    let prev_outcome = match before {
        Some(p) => p.outcome.as_str(),
        None => {
            // a new holder starts as if it had been seen reading, with
            // nothing done yet
            events.push(dispatched_event(w));
            t.stage
                .entry(w.holder.clone())
                .or_insert_with(|| "reading".to_string());
            let at = w.started().unwrap_or(next.at);
            t.reported
                .entry(w.holder.clone())
                .or_insert(((0, 0, 0), at));
            "running"
        }
    };

    if w.outcome == "running" {
        let stage = w.stage.as_str();
        if RUNNING_STAGES.contains(&stage)
            && let Some(last) = t.stage.insert(w.holder.clone(), stage.to_string())
            && last != stage
        {
            let mut detail = format!("{last} → {stage}");
            if stage == "editing"
                && let Some(c) = counts(w)
            {
                detail.push_str(&format!("  {}", render::work(c.0, c.1, c.2)));
                t.reported.insert(w.holder.clone(), (c, next.at));
            }
            events.push(
                RunEvent::new(next.at, "stage", unit, seq, detail)
                    .with("from", J::s(last))
                    .with("to", J::s(stage)),
            );
        }

        if let Some(c) = counts(w) {
            match t.reported.get(&w.holder) {
                Some((last, at)) if *last != c && next.at.0 - at.0 >= WORK_EVERY_S => {
                    events.push(work_event(next.at, w, c));
                    t.reported.insert(w.holder.clone(), (c, next.at));
                }
                Some(_) => {}
                None => {
                    t.reported.insert(w.holder.clone(), (c, next.at));
                }
            }
        }

        if is_quiet(w, quiet_after) {
            if t.quiet.insert(w.holder.clone()) {
                let q = w.log_quiet_s.unwrap_or(0);
                events.push(
                    RunEvent::new(
                        next.at,
                        "quiet",
                        unit,
                        seq,
                        format!("no output for {}", render::duration(q)),
                    )
                    .with("quiet_s", J::Int(q)),
                );
            }
        } else if w.log_quiet_s.is_some_and(|q| q < quiet_after as i64) && t.quiet.remove(&w.holder)
        {
            events.push(RunEvent::new(next.at, "active", unit, seq, "output again"));
        }
    } else {
        t.quiet.remove(&w.holder);
    }

    if prev_outcome == "running" && w.outcome == "integrating" {
        events.push(RunEvent::new(
            next.at,
            "integrating",
            unit,
            seq,
            "reported built; waiting for the gate",
        ));
    }
    if !is_final(prev_outcome) && is_final(&w.outcome) {
        events.push(outcome_event(w, next.at));
    }
}

/// `dispatched`, timed when the worker started; the same detail the run
/// writes, so a replayed line and a seen one look alike.
fn dispatched_event(w: &WorkerView) -> RunEvent {
    let n = w.holder.rsplit_once("-w").map(|(_, n)| n).unwrap_or("?");
    let detail = format!("→ w{n}{}", if w.retry { " (retry)" } else { "" });
    RunEvent::new(
        w.started().unwrap_or(UtcTime(0)),
        "dispatched",
        Some(&w.unit),
        Some(w.claim_seq),
        detail,
    )
    .with("holder", J::s(w.holder.clone()))
    .with("retry", J::Bool(w.retry))
}

/// `outcome`, timed when the attempt ended (`fallback` when it recorded
/// no end).
fn outcome_event(w: &WorkerView, fallback: UtcTime) -> RunEvent {
    let ended = w.ended().unwrap_or(fallback);
    let duration = w.started().map(|s| ended.0 - s.0).unwrap_or(0).max(0);
    let mut detail = w.outcome.clone();
    if let Some(d) = w.detail.as_deref().filter(|d| !d.is_empty()) {
        detail.push_str(&format!(" — {d}"));
    }
    RunEvent::new(ended, "outcome", Some(&w.unit), Some(w.claim_seq), detail)
        .with("duration_s", J::Int(duration))
        .with("outcome", J::s(w.outcome.clone()))
}

/// `work`: the counts now, whatever stage the worker is in.
fn work_event(at: UtcTime, w: &WorkerView, (files, ins, del): (u64, u64, u64)) -> RunEvent {
    RunEvent::new(
        at,
        "work",
        Some(&w.unit),
        Some(w.claim_seq),
        render::work(files, ins, del),
    )
    .with("deletions", J::Int(del as i64))
    .with("files", J::Int(files as i64))
    .with("insertions", J::Int(ins as i64))
}

/// `gate`: what the integrator took up, and for which units.
fn gate_event(at: UtcTime, step: &str, batch: &[String]) -> RunEvent {
    let mut detail = step.to_string();
    if !batch.is_empty() {
        detail.push_str(&format!(" · {}", batch.join(", ")));
    }
    RunEvent::new(at, "gate", None, None, detail)
        .with(
            "batch",
            J::Arr(batch.iter().map(|b| J::s(b.clone())).collect()),
        )
        .with("step", J::s(step))
}

/// `run-died`, with the process groups still running: those are what the
/// reader has to stop by hand.
fn died_event(s: &Snapshot) -> RunEvent {
    let survivors = s
        .workers
        .iter()
        .filter(|w| w.outcome == "running" && w.alive == Some(true))
        .map(|w| J::Int(w.pid as i64))
        .collect();
    RunEvent::new(
        s.at,
        "run-died",
        None,
        None,
        "the run is gone and recorded no end",
    )
    .with("pid", J::Int(s.pid as i64))
    .with("survivors", J::Arr(survivors))
}

/// `run-ended`, from what the run recorded; `None` when it recorded no end.
fn ended_event(s: &Snapshot) -> Option<RunEvent> {
    let term = s.terminated.as_ref()?;
    let mut detail = format!("{}: {}", term.state, term.detail);
    if let Some(p) = &s.progress {
        detail.push_str(&format!(" · {p}"));
    }
    Some(
        RunEvent::new(s.at, "run-ended", None, None, detail)
            .with("state", J::s(term.state.clone())),
    )
}

/// The follower's exit code for a run that ended in `state`.
fn exit_for(state: &str) -> u8 {
    match state {
        "complete" => 0,
        "blocked" => 2,
        _ => 1,
    }
}

/// The event's text in a human line. Kinds the follower does not make
/// itself (they come from a replay) show as `<kind>: <detail>`.
fn human_text(e: &RunEvent, stage_of: Option<&str>) -> String {
    let str_field = |k: &str| match e.field(k) {
        Some(J::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let int_field = |k: &str| match e.field(k) {
        Some(J::Int(i)) => *i,
        _ => 0,
    };
    match e.kind.as_str() {
        "dispatched" => format!("dispatched {}", e.detail),
        "stage" | "gate" | "waiting" => e.detail.clone(),
        "work" => match stage_of {
            Some(stage) => format!("{stage}  {}", e.detail),
            None => e.detail.clone(),
        },
        "quiet" => {
            let q = int_field("quiet_s");
            format!(
                "! quiet {} (no output since {})",
                render::duration(q),
                render::hhmmss(UtcTime(e.at.0 - q))
            )
        }
        "active" => "active again".to_string(),
        "integrating" => "reported built → waiting for the gate".to_string(),
        "outcome" => {
            let mut text = format!(
                "{}  {}",
                str_field("outcome"),
                render::duration(int_field("duration_s"))
            );
            if let Some((_, why)) = e.detail.split_once(" — ") {
                text.push_str(&format!("  {why}"));
            }
            text
        }
        "run-started" => format!("started · {}", e.detail),
        "run-died" => format!(
            "! died (pid {}) — the next `vitrify run` recovers its claims",
            int_field("pid")
        ),
        "run-ended" => {
            let state = str_field("state");
            let mut text = format!("ended: {state}");
            if let Some(i) = e.detail.find(" · Progress:") {
                text.push_str(&e.detail[i..]);
            }
            text.push_str(&format!(" · exit {}", exit_for(&state)));
            text
        }
        kind => format!("{kind}: {}", e.detail),
    }
}

/// One human line: `HH:MM:SS  <label>  <text>`, the label and the two
/// spaces after it taking 10 columns, so the texts line up. A longer label
/// still keeps its two spaces.
fn human_line(e: &RunEvent, stage_of: Option<&str>) -> String {
    let label = match (&e.unit, e.claim_seq) {
        (Some(u), Some(seq)) => format!("{u} c{seq}"),
        (Some(u), None) => u.clone(),
        _ if e.kind == "gate" => "gate".to_string(),
        _ => "run".to_string(),
    };
    format!(
        "{}  {label:<8}  {}",
        render::hhmmss(e.at),
        human_text(e, stage_of)
    )
}

/// Where the follower's lines go: human lines or NDJSON. Each print says
/// whether stdout is still there, so the follower can stop when it is not.
struct Printer<'a> {
    job: &'a str,
    json: bool,
}

impl Printer<'_> {
    /// Print one event; `false` when stdout has gone away. `snap` names the
    /// stage for a `work` line and the unit of each `run-died` survivor.
    fn event(&self, e: &RunEvent, replay: bool, snap: Option<&Snapshot>) -> bool {
        if self.json {
            return emit_checked(&e.to_json(Some(replay)).dumps_default());
        }
        let find = |pred: &dyn Fn(&WorkerView) -> bool| {
            snap.and_then(|s| s.workers.iter().find(|w| pred(w)))
        };
        let stage_of =
            find(&|w| Some(&w.unit) == e.unit.as_ref() && Some(w.claim_seq) == e.claim_seq)
                .map(|w| w.stage.as_str())
                .filter(|s| RUNNING_STAGES.contains(s));
        if !emit_checked(&human_line(e, stage_of)) {
            return false;
        }
        if e.kind == "run-died"
            && let Some(J::Arr(pids)) = e.field("survivors")
        {
            for pid in pids {
                let J::Int(pid) = pid else { continue };
                let pgid = *pid as u32;
                let unit = find(&|w| w.pid == pgid)
                    .map(|w| w.unit.clone())
                    .unwrap_or_else(|| "?".to_string());
                if !emit_checked(&render::kill_line(self.job, pgid, &unit)) {
                    return false;
                }
            }
        }
        true
    }

    /// Print the view of a whole snapshot; `false` when stdout has gone
    /// away.
    fn snapshot(&self, s: &Snapshot) -> bool {
        if self.json {
            let e =
                RunEvent::new(s.at, "snapshot", None, None, "").with("snapshot", render::json(s));
            return emit_checked(&e.to_json(Some(false)).dumps_default());
        }
        render::human(s).iter().all(|line| emit_checked(line))
            && emit_checked(&format!("--- following {} (Ctrl-C to stop) ---", self.job))
    }
}

/// Follow the run `job` until it ends; returns the exit code.
pub fn run(root: &Path, job: &str, args: &StatusArgs) -> Result<u8, CoreError> {
    let out = Printer {
        job,
        json: args.json,
    };
    let mut replayed_end = false;
    if args.since_start {
        let events = replay_events(root, job)?;
        if !out.json && !events.is_empty() && !emit_checked("(replayed from events.jsonl)") {
            return Ok(0);
        }
        for e in &events {
            replayed_end |= e.kind == "run-ended";
            if !out.event(e, true, None) {
                return Ok(0);
            }
        }
    }

    let mut prev = super::snapshot::take(root, job, UtcTime::now(), args.quiet_after)?;
    if !out.snapshot(&prev) {
        return Ok(0);
    }
    match prev.state {
        RunState::Alive => {}
        RunState::Died => {
            out.event(&died_event(&prev), false, Some(&prev));
            return Ok(1);
        }
        RunState::Ended => {
            if !replayed_end && let Some(e) = ended_event(&prev) {
                out.event(&e, false, Some(&prev));
            }
            let state = prev.terminated.as_ref().map(|t| t.state.as_str());
            return Ok(exit_for(state.unwrap_or("error")));
        }
    }

    let mut t = Tracker::new(&prev);
    let mut failing_since: Option<UtcTime> = None;
    loop {
        std::thread::sleep(Duration::from_secs(args.interval));
        let now = UtcTime::now();
        let next = match super::snapshot::take(root, job, now, args.quiet_after) {
            Ok(next) => next,
            Err(e) => {
                let since = *failing_since.get_or_insert(now);
                if now.0 - since.0 >= GIVE_UP_AFTER_S {
                    return Err(CoreError::refusal(1, e.to_string()));
                }
                continue;
            }
        };
        failing_since = None;
        // one look can find several changes: say them in the order they
        // happened (an attempt that ended before another's stage changed
        // comes first), but the run's own end always last, since nothing
        // is printed after it
        let mut events = diff(&prev, &next, &mut t, args.quiet_after);
        events.sort_by_key(|e| (matches!(e.kind.as_str(), "run-ended" | "run-died"), e.at));
        for e in events {
            if !out.event(&e, false, Some(&next)) {
                return Ok(0);
            }
            match e.kind.as_str() {
                "run-died" => return Ok(1),
                "run-ended" => {
                    let state = match e.field("state") {
                        Some(J::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    return Ok(exit_for(&state));
                }
                _ => {}
            }
        }
        prev = next;
    }
}

/// What the run recorded since it started: its event log. A run with no
/// event log replays nothing.
fn replay_events(root: &Path, job: &str) -> Result<Vec<RunEvent>, CoreError> {
    let path = root
        .join(".vitrify")
        .join("logs")
        .join(job)
        .join("events.jsonl");
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter_map(|v| RunEvent::from_json(&v))
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(super::cannot_read(&path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::run_status::snapshot::{GateView, Params, Terminated};

    /// 2027-01-15T08:00:00Z
    const T0: i64 = 1_800_000_000;

    fn at(secs: i64) -> UtcTime {
        UtcTime(T0 + secs)
    }

    /// The one place a worker fixture is built: a running worker of U-001
    /// that started at `T0`, probed and with nothing done yet.
    fn worker(holder: &str, stage: &str) -> WorkerView {
        WorkerView {
            unit: "U-001".to_string(),
            claim_seq: 1,
            holder: holder.to_string(),
            pid: 4242,
            retry: false,
            worktree: format!(".vitrify/worktrees/{holder}"),
            started_at: at(0).format(),
            ended_at: None,
            outcome: "running".to_string(),
            stage: stage.to_string(),
            alive: Some(true),
            log_bytes: Some(10),
            log_quiet_s: Some(0),
            files_changed: Some(0),
            insertions: Some(0),
            deletions: Some(0),
            carried_over: Some(0),
            stamped: Some(false),
            detail: None,
            error: None,
        }
    }

    /// The one place a snapshot fixture is built: a live run looked at
    /// `secs` after `T0`.
    fn snap(secs: i64, workers: Vec<WorkerView>) -> Snapshot {
        Snapshot {
            job_id: "run-x".to_string(),
            pid: 100,
            sdd: "specs/calc".to_string(),
            harness: "claude".to_string(),
            started_at: at(0).format(),
            at: at(secs),
            state: RunState::Alive,
            terminated: None,
            report: None,
            progress: Some("Progress: 1/2 Verified · 1 Claimed".to_string()),
            params: Params::default(),
            workers,
            gate: None,
            warnings: Vec::new(),
            logs: ".vitrify/logs/run-x/".to_string(),
        }
    }

    fn finished(mut w: WorkerView, outcome: &str, ended: i64, detail: Option<&str>) -> WorkerView {
        w.outcome = outcome.to_string();
        w.stage = outcome.to_string();
        w.ended_at = Some(at(ended).format());
        w.detail = detail.map(str::to_string);
        w.alive = None;
        w
    }

    fn kinds(events: &[RunEvent]) -> Vec<&str> {
        events.iter().map(|e| e.kind.as_str()).collect()
    }

    fn json(e: &RunEvent) -> String {
        e.to_json(None).dumps_default()
    }

    #[test]
    fn a_new_worker_is_dispatched_at_its_start() {
        let first = snap(0, vec![]);
        let mut t = Tracker::new(&first);
        let mut w = worker("run-x-w3", "reading");
        w.retry = true;
        let ev = diff(&first, &snap(5, vec![w]), &mut t, 300);
        assert_eq!(kinds(&ev), ["dispatched"]);
        assert_eq!(
            json(&ev[0]),
            r#"{"at": "2027-01-15T08:00:00Z", "event": "dispatched", "unit": "U-001", "claim_seq": 1, "detail": "\u2192 w3 (retry)", "holder": "run-x-w3", "retry": true}"#
        );
    }

    #[test]
    fn a_worker_that_finished_before_its_first_look_is_dispatched_then_ends() {
        let first = snap(0, vec![]);
        let mut t = Tracker::new(&first);
        let w = finished(
            worker("run-x-w1", "reading"),
            "rejected-gate",
            660,
            Some("failing gate: `pytest -q`"),
        );
        let ev = diff(&first, &snap(700, vec![w]), &mut t, 300);
        assert_eq!(kinds(&ev), ["dispatched", "outcome"]);
        assert_eq!(ev[1].at, at(660));
        assert_eq!(ev[1].detail, "rejected-gate — failing gate: `pytest -q`");
        assert_eq!(ev[1].field("duration_s"), Some(&J::Int(660)));
        assert_eq!(ev[1].field("outcome"), Some(&J::s("rejected-gate")));
    }

    #[test]
    fn a_new_worker_already_editing_changes_stage_from_reading() {
        let first = snap(0, vec![]);
        let mut t = Tracker::new(&first);
        let mut w = worker("run-x-w1", "editing");
        w.files_changed = Some(1);
        w.insertions = Some(3);
        let ev = diff(&first, &snap(5, vec![w]), &mut t, 300);
        assert_eq!(kinds(&ev), ["dispatched", "stage"]);
        assert_eq!(ev[1].detail, "reading → editing  1 file +3 -0");
    }

    #[test]
    fn each_stage_change_is_said_and_unknown_is_skipped() {
        let w = |stage: &str| worker("run-x-w1", stage);
        let first = snap(0, vec![w("reading")]);
        let mut t = Tracker::new(&first);
        let mut editing = w("editing");
        editing.files_changed = Some(2);
        editing.insertions = Some(40);
        let s1 = snap(2, vec![editing.clone()]);
        let ev = diff(&first, &s1, &mut t, 300);
        assert_eq!(kinds(&ev), ["stage"]);
        assert_eq!(ev[0].detail, "reading → editing  2 files +40 -0");
        assert_eq!(ev[0].field("from"), Some(&J::s("reading")));
        assert_eq!(ev[0].field("to"), Some(&J::s("editing")));

        // into unknown and out to a new stage: compared with editing
        let mut unknown = w("unknown");
        unknown.files_changed = None;
        let s2 = snap(4, vec![unknown.clone()]);
        assert!(diff(&s1, &s2, &mut t, 300).is_empty());
        let mut stamped = editing.clone();
        stamped.stage = "stamped".to_string();
        let s3 = snap(6, vec![stamped.clone()]);
        let ev = diff(&s2, &s3, &mut t, 300);
        assert_eq!(kinds(&ev), ["stage"]);
        assert_eq!(ev[0].detail, "editing → stamped");

        // back through unknown to the same stage: nothing
        let s4 = snap(8, vec![unknown]);
        assert!(diff(&s3, &s4, &mut t, 300).is_empty());
        let s5 = snap(10, vec![stamped.clone()]);
        assert!(diff(&s4, &s5, &mut t, 300).is_empty());

        let mut exited = stamped;
        exited.stage = "exited".to_string();
        let s6 = snap(12, vec![exited.clone()]);
        let ev = diff(&s5, &s6, &mut t, 300);
        assert_eq!(kinds(&ev), ["stage"]);
        assert_eq!(ev[0].detail, "stamped → exited");

        let mut integrating = exited;
        integrating.outcome = "integrating".to_string();
        integrating.stage = "integrating".to_string();
        let ev = diff(&s6, &snap(14, vec![integrating]), &mut t, 300);
        assert_eq!(kinds(&ev), ["integrating"]);
        assert_eq!(ev[0].detail, "reported built; waiting for the gate");
        assert!(ev[0].fields.is_empty());
    }

    #[test]
    fn work_is_said_at_most_once_a_minute_and_a_held_change_comes_later() {
        let mut w = worker("run-x-w1", "editing");
        w.files_changed = Some(1);
        w.insertions = Some(10);
        let first = snap(0, vec![w.clone()]);
        let mut t = Tracker::new(&first);
        let with = |secs: i64, files: u64, ins: u64| {
            let mut w = w.clone();
            w.files_changed = Some(files);
            w.insertions = Some(ins);
            snap(secs, vec![w])
        };
        // a change 30 s after the last report is held back
        let s1 = with(30, 2, 20);
        assert!(diff(&first, &s1, &mut t, 300).is_empty());
        // once the minute is up it is said, even with nothing new since
        let s2 = with(60, 2, 20);
        let ev = diff(&s1, &s2, &mut t, 300);
        assert_eq!(kinds(&ev), ["work"]);
        assert_eq!(
            json(&ev[0]),
            r#"{"at": "2027-01-15T08:01:00Z", "event": "work", "unit": "U-001", "claim_seq": 1, "detail": "2 files +20 -0", "deletions": 0, "files": 2, "insertions": 20}"#
        );
        // unchanged counts are not repeated
        let s3 = with(200, 2, 20);
        assert!(diff(&s2, &s3, &mut t, 300).is_empty());
        // a minute has passed since the last report: said at once
        let s4 = with(201, 5, 130);
        assert_eq!(kinds(&diff(&s3, &s4, &mut t, 300)), ["work"]);
        // and the next change waits for the minute again
        let s5 = with(220, 6, 140);
        assert!(diff(&s4, &s5, &mut t, 300).is_empty());
        let s6 = with(261, 6, 140);
        assert_eq!(kinds(&diff(&s5, &s6, &mut t, 300)), ["work"]);
    }

    #[test]
    fn a_quiet_worker_is_flagged_once_then_active_again() {
        let quiet = |secs: i64, q: i64| {
            let mut w = worker("run-x-w1", "reading");
            w.log_quiet_s = Some(q);
            snap(secs, vec![w])
        };
        let first = quiet(0, 10);
        let mut t = Tracker::new(&first);
        let s1 = quiet(300, 310);
        let ev = diff(&first, &s1, &mut t, 300);
        assert_eq!(kinds(&ev), ["quiet"]);
        assert_eq!(ev[0].detail, "no output for 5m");
        assert_eq!(ev[0].field("quiet_s"), Some(&J::Int(310)));
        let s2 = quiet(302, 312);
        assert!(diff(&s1, &s2, &mut t, 300).is_empty());
        let s3 = quiet(304, 1);
        let ev = diff(&s2, &s3, &mut t, 300);
        assert_eq!(kinds(&ev), ["active"]);
        assert_eq!(ev[0].detail, "output again");
        assert!(ev[0].fields.is_empty());

        // already quiet at the first look: the one-shot view said so
        let first = quiet(0, 400);
        let mut t = Tracker::new(&first);
        assert!(diff(&first, &quiet(2, 402), &mut t, 300).is_empty());
    }

    #[test]
    fn a_gate_step_change_is_said_but_idle_is_not() {
        let gate = |step: &str, batch: &[&str]| GateView {
            step: step.to_string(),
            since: None,
            batch: batch.iter().map(|b| b.to_string()).collect(),
            running: Vec::new(),
            done: None,
            total: None,
        };
        let with = |secs: i64, g: GateView| {
            let mut s = snap(secs, vec![]);
            s.gate = Some(g);
            s
        };
        let s0 = with(0, gate("idle", &[]));
        let mut t = Tracker::new(&s0);
        let s1 = with(2, gate("rows", &["U-002", "U-005"]));
        let ev = diff(&s0, &s1, &mut t, 300);
        assert_eq!(kinds(&ev), ["gate"]);
        assert_eq!(
            json(&ev[0]),
            r#"{"at": "2027-01-15T08:00:02Z", "event": "gate", "unit": null, "claim_seq": null, "detail": "rows \u00b7 U-002, U-005", "batch": ["U-002", "U-005"], "step": "rows"}"#
        );
        let same = with(4, gate("rows", &["U-002", "U-005"]));
        assert!(diff(&s1, &same, &mut t, 300).is_empty());
        let s2 = with(6, gate("head-check", &[]));
        assert_eq!(diff(&same, &s2, &mut t, 300)[0].detail, "head-check");
        assert!(diff(&s2, &with(8, gate("idle", &[])), &mut t, 300).is_empty());
    }

    #[test]
    fn a_run_that_dies_names_its_surviving_workers() {
        let mut gone = worker("run-x-w2", "editing");
        gone.pid = 7;
        gone.alive = Some(false);
        let live = worker("run-x-w1", "editing");
        let first = snap(0, vec![live.clone(), gone.clone()]);
        let mut t = Tracker::new(&first);
        let mut died = snap(2, vec![live, gone]);
        died.state = RunState::Died;
        let ev = diff(&first, &died, &mut t, 300);
        assert_eq!(kinds(&ev), ["run-died"]);
        assert_eq!(
            json(&ev[0]),
            r#"{"at": "2027-01-15T08:00:02Z", "event": "run-died", "unit": null, "claim_seq": null, "detail": "the run is gone and recorded no end", "pid": 100, "survivors": [4242]}"#
        );
    }

    #[test]
    fn a_run_that_ends_says_how() {
        let first = snap(0, vec![worker("run-x-w1", "editing")]);
        let mut t = Tracker::new(&first);
        let mut ended = snap(
            30,
            vec![finished(
                worker("run-x-w1", "editing"),
                "verified",
                20,
                None,
            )],
        );
        ended.state = RunState::Ended;
        ended.terminated = Some(Terminated {
            state: "complete".to_string(),
            at: at(25).format(),
            detail: "done-condition met".to_string(),
        });
        let ev = diff(&first, &ended, &mut t, 300);
        assert_eq!(kinds(&ev), ["outcome", "run-ended"]);
        assert_eq!(ev[0].detail, "verified");
        assert_eq!(
            ev[1].detail,
            "complete: done-condition met · Progress: 1/2 Verified · 1 Claimed"
        );
        assert_eq!(ev[1].field("state"), Some(&J::s("complete")));
        assert!(diff(&ended, &ended, &mut t, 300).is_empty());
    }

    #[test]
    fn human_lines_line_up_and_say_each_kind_plainly() {
        let line = |e: RunEvent| human_line(&e, Some("editing"));
        let u = Some("U-003");
        let cases = [
            (
                RunEvent::new(at(0), "dispatched", u, Some(1), "→ w3 (retry)"),
                "08:00:00  U-003 c1  dispatched → w3 (retry)",
            ),
            (
                RunEvent::new(at(0), "work", u, Some(1), "4 files +120 -3"),
                "08:00:00  U-003 c1  editing  4 files +120 -3",
            ),
            (
                RunEvent::new(at(600), "quiet", u, Some(1), "").with("quiet_s", J::Int(300)),
                "08:10:00  U-003 c1  ! quiet 5m (no output since 08:05:00)",
            ),
            (
                RunEvent::new(at(0), "active", u, Some(1), "output again"),
                "08:00:00  U-003 c1  active again",
            ),
            (
                RunEvent::new(at(0), "integrating", u, Some(1), ""),
                "08:00:00  U-003 c1  reported built → waiting for the gate",
            ),
            (
                RunEvent::new(
                    at(0),
                    "outcome",
                    u,
                    Some(1),
                    "rejected-gate — failing gate: x",
                )
                .with("duration_s", J::Int(660))
                .with("outcome", J::s("rejected-gate")),
                "08:00:00  U-003 c1  rejected-gate  11m  failing gate: x",
            ),
            (
                RunEvent::new(at(0), "outcome", u, Some(1), "verified")
                    .with("duration_s", J::Int(420))
                    .with("outcome", J::s("verified")),
                "08:00:00  U-003 c1  verified  7m",
            ),
            (
                RunEvent::new(at(0), "gate", None, None, "rows · U-002"),
                "08:00:00  gate      rows · U-002",
            ),
            (
                RunEvent::new(at(0), "run-started", None, None, "specs/calc · cap 3"),
                "08:00:00  run       started · specs/calc · cap 3",
            ),
            (
                RunEvent::new(
                    at(0),
                    "run-ended",
                    None,
                    None,
                    "complete: done · Progress: 5/5 Verified",
                )
                .with("state", J::s("complete")),
                "08:00:00  run       ended: complete · Progress: 5/5 Verified · exit 0",
            ),
            (
                RunEvent::new(at(0), "run-ended", None, None, "blocked: stuck")
                    .with("state", J::s("blocked")),
                "08:00:00  run       ended: blocked · exit 2",
            ),
            (
                RunEvent::new(at(0), "run-died", None, None, "").with("pid", J::Int(9)),
                "08:00:00  run       ! died (pid 9) — the next `vitrify run` recovers its claims",
            ),
            (
                RunEvent::new(at(0), "stale", u, Some(1), "its input changed"),
                "08:00:00  U-003 c1  stale: its input changed",
            ),
            (
                RunEvent::new(at(0), "waiting", None, None, "waiting: the run resumes"),
                "08:00:00  run       waiting: the run resumes",
            ),
        ];
        for (e, want) in cases {
            assert_eq!(line(e), want);
        }
    }
}
