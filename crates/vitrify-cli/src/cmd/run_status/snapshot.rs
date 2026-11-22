//! One look at a run: its record, the gate board, and a probe of each
//! worker that is still running. Prints nothing; `render` shows it and
//! `follow` compares one look with the next.
//!
//! The job's own records (`run.json`, `project.json`, `events.jsonl`) are
//! in `.vitrify/logs/<job>/`; each lane's — its gate board, its workers'
//! logs — in `.vitrify/logs/<job>/<slug>/`, the lanes being listed in
//! `project.json`. Two shapes of that file are read: lanes with their SDD
//! and state alone, and the project record's, whose lanes also carry their
//! root and the plan's counts. A job recorded before lanes existed has no
//! `project.json`; its one lane is the job itself.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vitrify_core::graph::{Node, UnitGraph, slug_of};
use vitrify_core::model::{Instance, UtcTime};
use vitrify_core::{CoreError, ledger};

use super::render::{died_line, duration, hhmmss, kill_line};
use crate::cmd;
use crate::git::Git;
use crate::signals;

/// Whether the run is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// Its process exists and its job lock file is there.
    Alive,
    /// Gone without recording an end (killed outright, or a crash).
    Died,
    /// It recorded how it ended (`terminated` in `run.json`).
    Ended,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Alive => "alive",
            RunState::Died => "died",
            RunState::Ended => "ended",
        }
    }
}

/// How a run ended, as it recorded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Terminated {
    /// `complete`, `blocked`, `interrupted` or `error`.
    pub state: String,
    pub at: String,
    pub detail: String,
}

/// The run's parameters, as recorded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    pub cap: i64,
    pub gate_jobs: i64,
    pub lease_minutes: i64,
    pub worker_timeout_minutes: i64,
}

/// What the integrator is doing now (`gate.json`).
#[derive(Debug, Clone, PartialEq)]
pub struct GateView {
    pub step: String,
    pub since: Option<UtcTime>,
    pub batch: Vec<String>,
    /// The commands in progress and when each started, oldest first.
    pub running: Vec<(String, Option<UtcTime>)>,
    pub done: Option<i64>,
    pub total: Option<i64>,
}

/// One attempt at a unit: its record, and — while it runs — what a probe
/// found.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerView {
    pub unit: String,
    pub claim_seq: i64,
    pub holder: String,
    /// Also its process-group id.
    pub pid: u32,
    pub retry: bool,
    pub worktree: String,
    /// The slug of the lane the attempt belongs to; empty in a job
    /// recorded before lanes existed.
    pub lane: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub outcome: String,
    /// `reading`, `editing`, `stamped`, `exited`, `unknown`, `queued`,
    /// `gate`, `integrating`, or the final outcome.
    pub stage: String,
    // probes: `None` when the worker was not probed (it is not running) or
    // the probe could not tell
    pub alive: Option<bool>,
    pub log_bytes: Option<u64>,
    pub log_quiet_s: Option<i64>,
    pub files_changed: Option<u64>,
    pub insertions: Option<u64>,
    pub deletions: Option<u64>,
    pub carried_over: Option<u64>,
    pub stamped: Option<bool>,
    /// Why the attempt ended, as the run recorded it.
    pub detail: Option<String>,
    /// Why the worktree probe failed.
    pub error: Option<String>,
}

impl WorkerView {
    pub fn started(&self) -> Option<UtcTime> {
        UtcTime::parse(&self.started_at)
    }

    pub fn ended(&self) -> Option<UtcTime> {
        self.ended_at.as_deref().and_then(UtcTime::parse)
    }
}

/// One lane as the look found it: its recorded state, how far it has got,
/// and, when it is idle or blocked, the units elsewhere it waits on.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneView {
    /// Names the lane's records directory.
    pub slug: String,
    /// The SDD's path from the repository root.
    pub root: String,
    /// As the run recorded it: `idle`, `build`, `recheck`, `up to date`,
    /// `complete` or `blocked`.
    pub state: String,
    /// `None` when neither the SDD nor the record gave the counts.
    pub verified: Option<u64>,
    /// Non-Retired units.
    pub total: Option<u64>,
    /// Attempts running or waiting for the gate.
    pub in_flight: u64,
    /// Units neither Verified nor in flight.
    pub waiting: Option<u64>,
    /// `<slug> <unit>` of the other lanes' units that hold this lane's
    /// units back, sorted. Only an idle or blocked lane has any.
    pub waits_on: Vec<String>,
}

/// One look at a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub job_id: String,
    pub pid: u32,
    pub sdd: String,
    pub harness: String,
    pub started_at: String,
    /// When the look was taken.
    pub at: UtcTime,
    pub state: RunState,
    pub terminated: Option<Terminated>,
    /// The final report's lines, once the run has printed it.
    pub report: Option<Vec<String>>,
    /// The SDD's progress line in the primary checkout, if it could be read.
    pub progress: Option<String>,
    pub params: Params,
    /// In `run.json` order.
    pub workers: Vec<WorkerView>,
    /// Every lane, in `project.json` order; one for a job recorded before
    /// lanes existed.
    pub lanes: Vec<LaneView>,
    /// Only while the run is alive.
    pub gate: Option<GateView>,
    pub warnings: Vec<String>,
    /// `.vitrify/logs/<job>/`
    pub logs: String,
}

/// Look at the run `job` of the repository whose primary checkout is
/// `root`, as of `now`.
pub fn take(root: &Path, job: &str, now: UtcTime, quiet_after: u64) -> Result<Snapshot, CoreError> {
    let scratch = root.join(".vitrify");
    let logs_dir = scratch.join("logs").join(job);
    // the job's own copy: two runs at once take turns writing the top-level one
    let path = logs_dir.join("run.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CoreError::refusal(
                4,
                format!("error: no run recorded as {job}"),
            ));
        }
        Err(e) => return Err(super::cannot_read(&path, e)),
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| super::cannot_read(&path, e))?;

    let pid = as_u32(&v["pid"]);
    let sdd = str_of(&v["sdd"]);
    let terminated = v["terminated"].as_object().map(|t| Terminated {
        state: t.get("state").map(str_of).unwrap_or_default(),
        at: t.get("at").map(str_of).unwrap_or_default(),
        detail: t.get("detail").map(str_of).unwrap_or_default(),
    });
    let report = v["report"].as_array().map(|lines| {
        lines
            .iter()
            .map(|l| l.as_str().unwrap_or_default().to_string())
            .collect()
    });
    // only test that the job lock exists: holding it, even for a moment,
    // would make a recovering run take a dead run for a live one
    let state = if terminated.is_some() {
        RunState::Ended
    } else if signals::alive(pid, false)
        && scratch.join("jobs").join(format!("{job}.lock")).exists()
    {
        RunState::Alive
    } else {
        RunState::Died
    };
    let progress = cmd::resolve_sdd_in(root, Path::new(&sdd))
        .and_then(Instance::load)
        .ok()
        .map(|inst| ledger::progress_line(&inst.units));
    let p = &v["params"];
    let params = Params {
        cap: p["cap"].as_i64().unwrap_or(0),
        gate_jobs: p["gate_jobs"].as_i64().unwrap_or(0),
        lease_minutes: p["lease_minutes"].as_i64().unwrap_or(0),
        worker_timeout_minutes: p["worker_timeout_minutes"].as_i64().unwrap_or(0),
    };
    let lanes = lane_dirs(&logs_dir);
    let gate = if state == RunState::Alive {
        // the board of a lane that is gating, else the first lane's
        let boards: Vec<GateView> = lanes
            .iter()
            .filter_map(|(_, dir)| std::fs::read_to_string(dir.join("gate.json")).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .filter_map(|g| parse_gate(&g))
            .collect();
        let busy = boards.iter().position(|g| g.step != "idle").unwrap_or(0);
        boards.into_iter().nth(busy)
    } else {
        None
    };

    let mut workers = Vec::new();
    for w in v["workers"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let mut view = WorkerView {
            unit: str_of(&w["unit"]),
            claim_seq: w["claim_seq"].as_i64().unwrap_or(0),
            holder: str_of(&w["holder"]),
            pid: as_u32(&w["pid"]),
            retry: w["retry"].as_bool().unwrap_or(false),
            worktree: str_of(&w["worktree"]),
            lane: str_of(&w["lane"]),
            started_at: str_of(&w["started_at"]),
            ended_at: w["ended_at"].as_str().map(str::to_string),
            outcome: str_of(&w["outcome"]),
            stage: String::new(),
            alive: None,
            log_bytes: None,
            log_quiet_s: None,
            files_changed: None,
            insertions: None,
            deletions: None,
            carried_over: None,
            stamped: None,
            detail: w["detail"].as_str().map(str::to_string),
            error: None,
        };
        if view.outcome == "running" {
            // its lane's records; a worker that names no lane is the
            // first lane's
            let lane = w["lane"].as_str();
            let dir = lanes
                .iter()
                .find(|(slug, _)| Some(slug.as_str()) == lane)
                .or(lanes.first())
                .map_or(logs_dir.as_path(), |(_, dir)| dir.as_path());
            probe(&mut view, dir, &sdd, now);
        }
        view.stage = stage(&view, state, gate.as_ref());
        workers.push(view);
    }

    let lane_views = lane_views(root, &logs_dir, &sdd, terminated.as_ref(), &workers);
    let mut snap = Snapshot {
        job_id: v["job_id"].as_str().unwrap_or(job).to_string(),
        pid,
        sdd,
        harness: str_of(&v["harness"]),
        started_at: str_of(&v["started_at"]),
        at: now,
        state,
        terminated,
        report,
        progress,
        params,
        workers,
        lanes: lane_views,
        gate,
        warnings: Vec::new(),
        logs: format!(".vitrify/logs/{job}/"),
    };
    snap.warnings = warnings(&snap, quiet_after);
    Ok(snap)
}

/// One lane as `project.json` lists it.
struct LaneRecord {
    slug: String,
    /// The `root` of the project record's lanes, else the `sdd` of the
    /// minimal shape.
    root: String,
    state: String,
    /// The plan's counts: only the project record's lanes carry them.
    units: Option<u64>,
    verified: Option<u64>,
}

/// The lanes of `project.json`; `None` when the job has no such file (it
/// was recorded before lanes existed) or it cannot be read.
fn project_lanes(logs_dir: &Path) -> Option<Vec<LaneRecord>> {
    let project = fs_err::read_to_string(logs_dir.join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())?;
    let lanes = project["lanes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|l| {
            let slug = l["slug"].as_str()?;
            // a name, never a path out of the job's directory
            if slug.is_empty() || slug.contains('/') || slug == ".." {
                return None;
            }
            let root = l["root"].as_str().or(l["sdd"].as_str());
            Some(LaneRecord {
                slug: slug.to_string(),
                root: root.unwrap_or_default().to_string(),
                state: str_of(&l["state"]),
                units: l["units"].as_u64(),
                verified: l["verified"].as_u64(),
            })
        })
        .collect();
    Some(lanes)
}

/// Each lane of the job whose records are in `logs_dir`, with the
/// directory of its records, in `project.json` order. A job recorded
/// before runs had lanes keeps them in its own directory.
pub fn lane_dirs(logs_dir: &Path) -> Vec<(String, PathBuf)> {
    match project_lanes(logs_dir) {
        Some(lanes) => lanes
            .into_iter()
            .map(|l| {
                let dir = logs_dir.join(&l.slug);
                (l.slug, dir)
            })
            .collect(),
        None => vec![(String::new(), logs_dir.to_path_buf())],
    }
}

/// An SDD of the primary checkout, if it can be read.
fn load_sdd(root: &Path, rel: &str) -> Option<Instance> {
    if rel.is_empty() {
        return None;
    }
    cmd::resolve_sdd_in(root, Path::new(rel))
        .and_then(Instance::load)
        .ok()
}

/// Every lane of the job: what `project.json` records of it, the progress
/// of its SDD in the primary checkout, the attempts it has in flight, and,
/// for an idle or blocked lane, what it waits on. A job recorded before
/// lanes existed has the one lane its `run.json` names, in the state its
/// end gives it.
fn lane_views(
    root: &Path,
    logs_dir: &Path,
    sdd: &str,
    terminated: Option<&Terminated>,
    workers: &[WorkerView],
) -> Vec<LaneView> {
    let records = project_lanes(logs_dir).unwrap_or_else(|| {
        let state = match terminated.map(|t| t.state.as_str()) {
            None => "build",
            Some("complete") => "complete",
            Some(_) => "blocked",
        };
        vec![LaneRecord {
            slug: String::new(),
            root: sdd.to_string(),
            state: state.to_string(),
            units: None,
            verified: None,
        }]
    });
    let insts: Vec<Option<Instance>> = records.iter().map(|r| load_sdd(root, &r.root)).collect();
    let graph = if records
        .iter()
        .any(|r| r.state == "idle" || r.state == "blocked")
    {
        let loaded: Vec<Instance> = insts.iter().flatten().cloned().collect();
        UnitGraph::build(&loaded).ok()
    } else {
        None
    };

    records
        .iter()
        .zip(&insts)
        .enumerate()
        .map(|(i, (rec, inst))| {
            // an attempt that names no lane is the first lane's
            let in_flight = workers
                .iter()
                .filter(|w| w.outcome == "running" || w.outcome == "integrating")
                .filter(|w| {
                    w.lane == rec.slug || (i == 0 && !records.iter().any(|r| r.slug == w.lane))
                })
                .count() as u64;
            let counts = match inst {
                Some(inst) => {
                    let live: Vec<&str> = inst
                        .units
                        .iter()
                        .map(|u| u.meta.get_str_or("status", "Draft"))
                        .filter(|s| *s != "Retired")
                        .collect();
                    let verified = live.iter().filter(|s| **s == "Verified").count();
                    Some((verified as u64, live.len() as u64))
                }
                None => rec.verified.zip(rec.units),
            };
            // a job recorded before lanes existed names no slug: its SDD's
            let slug = match inst {
                Some(inst) if rec.slug.is_empty() => slug_of(inst).to_string(),
                _ => rec.slug.clone(),
            };
            let waits_on = match (&graph, rec.state.as_str()) {
                (Some(g), "idle" | "blocked") => waits_on_of(g, &slug),
                _ => Vec::new(),
            };
            LaneView {
                slug,
                root: rec.root.clone(),
                state: rec.state.clone(),
                verified: counts.map(|c| c.0),
                total: counts.map(|c| c.1),
                in_flight,
                waiting: counts.map(|(v, t)| t.saturating_sub(v).saturating_sub(in_flight)),
                waits_on,
            }
        })
        .collect()
}

/// The units of other lanes that hold back the unfinished units of lane
/// `slug`: those not yet Built, since a Built unit holds nothing back.
/// Sorted.
fn waits_on_of(graph: &UnitGraph, slug: &str) -> Vec<String> {
    let done = |n: &Node| matches!(graph.status(n), Some("Verified" | "Retired"));
    let released = |n: &Node| done(n) || graph.status(n) == Some("Built");
    let mut out: BTreeSet<&Node> = BTreeSet::new();
    for n in graph.nodes().iter().filter(|n| n.slug == slug && !done(n)) {
        out.extend(
            graph
                .dependencies(n)
                .into_iter()
                .filter(|d| d.slug != slug && !released(d)),
        );
    }
    out.into_iter().map(Node::to_string).collect()
}

/// A string field, `""` when absent or not a string.
fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// A pid field, 0 (never alive) when absent or out of range.
fn as_u32(v: &Value) -> u32 {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0)
}

/// `gate.json` as the integrator wrote it; `None` when it is not an object.
pub fn parse_gate(g: &Value) -> Option<GateView> {
    let g = g.as_object()?;
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let running = g
        .get("running")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|r| {
                    (
                        str_of(&r["command"]),
                        r["since"].as_str().and_then(UtcTime::parse),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Some(GateView {
        step: g.get("step").map(str_of).unwrap_or_default(),
        since: g
            .get("since")
            .and_then(Value::as_str)
            .and_then(UtcTime::parse),
        batch: strings(g.get("batch")),
        running,
        done: g.get("done").and_then(Value::as_i64),
        total: g.get("total").and_then(Value::as_i64),
    })
}

/// Fill in what can be seen of a running worker. Each probe that fails
/// leaves its fields empty: one worker never fails the whole look.
fn probe(w: &mut WorkerView, logs_dir: &Path, sdd: &str, now: UtcTime) {
    w.alive = Some(signals::alive(w.pid, true));

    let mut bytes = None;
    let mut latest: Option<i64> = None;
    for ext in ["out", "err"] {
        if let Ok(meta) = std::fs::metadata(logs_dir.join(format!("worker-{}.{ext}", w.holder))) {
            bytes = Some(bytes.unwrap_or(0) + meta.len());
            if let Some(t) = meta.modified().ok().and_then(epoch) {
                latest = Some(latest.map_or(t, |l: i64| l.max(t)));
            }
        }
    }
    w.log_bytes = bytes;
    w.log_quiet_s = latest.map(|t| (now.0 - t).max(0));

    let wt = Path::new(&w.worktree);
    if !wt.is_dir() {
        w.error = Some("worktree missing".to_string());
    } else {
        let prompt = std::fs::metadata(logs_dir.join(format!("worker-{}.prompt.md", w.holder)))
            .and_then(|m| m.modified())
            .ok();
        match probe_work(wt, prompt) {
            Ok(work) => {
                w.files_changed = Some(work.files);
                w.insertions = Some(work.insertions);
                w.deletions = Some(work.deletions);
                w.carried_over = Some(work.carried_over);
            }
            Err(e) => w.error = Some(e),
        }
    }

    w.stamped = cmd::resolve_sdd_in(wt, Path::new(sdd))
        .and_then(Instance::load)
        .ok()
        .and_then(|inst| {
            inst.unit(&w.unit)
                .map(|u| u.meta.get_str_or("status", "") == "Built")
        });
}

/// Epoch seconds of a file time.
fn epoch(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// A worktree's uncommitted work.
struct Work {
    files: u64,
    insertions: u64,
    deletions: u64,
    carried_over: u64,
}

/// At most this many untracked files have their lines counted.
const MAX_COUNTED: usize = 500;

/// What a worker has changed in its worktree, read with git's optional
/// locks off. An entry not touched since `prompt` (the moment the worker
/// was handed its brief) was carried over from an earlier attempt.
fn probe_work(wt: &Path, prompt: Option<SystemTime>) -> Result<Work, String> {
    let first_line =
        |e: crate::git::GitError| e.to_string().lines().next().unwrap_or_default().to_string();
    let git = Git::new(wt);
    let changes = git.changes_readonly().map_err(first_line)?;
    let (mut insertions, deletions) = git.numstat_readonly("HEAD").map_err(first_line)?;
    let mut counted = 0;
    let mut carried_over = 0;
    for (xy, path) in &changes {
        let full = wt.join(path);
        if xy == "??" && counted < MAX_COUNTED {
            counted += 1;
            insertions += count_lines(&full);
        }
        let edited = match (prompt, std::fs::metadata(&full).and_then(|m| m.modified())) {
            (None, _) | (_, Err(_)) => true,
            (Some(p), Ok(m)) => m > p,
        };
        if !edited {
            carried_over += 1;
        }
    }
    Ok(Work {
        files: changes.len() as u64,
        insertions,
        deletions,
        carried_over,
    })
}

/// The lines of an untracked file, as a diff would count them: at most
/// 1 MiB is read, and a NUL in the first 8 KiB means binary (0 lines).
pub fn count_lines(path: &Path) -> u64 {
    let mut buf = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(1024 * 1024).read_to_end(&mut buf));
    if read.is_err() {
        return 0;
    }
    if buf[..buf.len().min(8192)].contains(&0) {
        return 0;
    }
    let newlines = buf.iter().filter(|&&b| b == b'\n').count() as u64;
    let unterminated = !buf.is_empty() && buf.last() != Some(&b'\n');
    newlines + u64::from(unterminated)
}

/// The stage of one attempt: what its record, the probe and the gate board
/// say together. The first rule that matches wins.
pub fn stage(w: &WorkerView, state: RunState, gate: Option<&GateView>) -> String {
    match w.outcome.as_str() {
        "running" => {
            if w.alive == Some(false) {
                "exited"
            } else if w.error.is_some() {
                "unknown"
            } else if w.stamped == Some(true) {
                "stamped"
            } else if w.files_changed.unwrap_or(0) > w.carried_over.unwrap_or(0) {
                "editing"
            } else {
                "reading"
            }
        }
        "integrating" => {
            if state != RunState::Alive {
                "integrating"
            } else if gate.is_some_and(|g| g.batch.contains(&w.unit)) {
                "gate"
            } else {
                "queued"
            }
        }
        other => other,
    }
    .to_string()
}

/// The warnings a look raises: silent and quiet workers, workers near
/// their time budget, then a run that died and the workers it left behind.
pub fn warnings(s: &Snapshot, quiet_after: u64) -> Vec<String> {
    let quiet_after = quiet_after as i64;
    let running = || s.workers.iter().filter(|w| w.outcome == "running");
    let elapsed = |w: &WorkerView| w.started().map(|t| (s.at.0 - t.0).max(0));
    let mut out = Vec::new();
    for w in running().filter(|w| w.alive == Some(true)) {
        let label = format!("{} c{}", w.unit, w.claim_seq);
        let ran = elapsed(w).unwrap_or(0);
        if w.log_bytes == Some(0) && ran >= quiet_after {
            out.push(format!("{label}: no output at all after {}", duration(ran)));
        } else if let Some(q) = w.log_quiet_s.filter(|&q| q >= quiet_after) {
            out.push(format!(
                "{label}: no output for {} (since {})",
                duration(q),
                hhmmss(UtcTime(s.at.0 - q))
            ));
        }
    }
    let budget = s.params.worker_timeout_minutes * 60;
    if budget > 0 {
        for w in running() {
            if let Some(ran) = elapsed(w).filter(|&r| r * 10 >= budget * 9) {
                out.push(format!(
                    "{} c{}: {} of its {} budget used",
                    w.unit,
                    w.claim_seq,
                    duration(ran),
                    duration(budget)
                ));
            }
        }
    }
    if s.state == RunState::Died {
        out.push(died_line(s.pid));
        for w in running().filter(|w| w.alive == Some(true)) {
            out.push(kill_line(&s.job_id, w.pid, &w.unit));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A worker record as `take` builds it, before any probe.
    pub(crate) fn worker(unit: &str, outcome: &str) -> WorkerView {
        WorkerView {
            unit: unit.to_string(),
            claim_seq: 1,
            holder: "run-x-w1".to_string(),
            pid: 4242,
            retry: false,
            worktree: "/tmp/wt".to_string(),
            lane: String::new(),
            started_at: "2026-11-16T14:00:00Z".to_string(),
            ended_at: None,
            outcome: outcome.to_string(),
            stage: String::new(),
            alive: None,
            log_bytes: None,
            log_quiet_s: None,
            files_changed: None,
            insertions: None,
            deletions: None,
            carried_over: None,
            stamped: None,
            detail: None,
            error: None,
        }
    }

    fn gate(batch: &[&str]) -> GateView {
        GateView {
            step: "rows".to_string(),
            since: None,
            batch: batch.iter().map(|u| u.to_string()).collect(),
            running: Vec::new(),
            done: None,
            total: None,
        }
    }

    #[test]
    fn running_stages() {
        let alive = RunState::Alive;
        let mut w = worker("U-001", "running");
        w.alive = Some(true);
        w.files_changed = Some(0);
        w.carried_over = Some(0);
        assert_eq!(stage(&w, alive, None), "reading");
        // only carried-over files: still reading
        w.files_changed = Some(2);
        w.carried_over = Some(2);
        assert_eq!(stage(&w, alive, None), "reading");
        w.carried_over = Some(1);
        assert_eq!(stage(&w, alive, None), "editing");
        w.stamped = Some(true);
        assert_eq!(stage(&w, alive, None), "stamped");
        w.error = Some("worktree missing".to_string());
        assert_eq!(stage(&w, alive, None), "unknown");
        w.alive = Some(false);
        assert_eq!(stage(&w, alive, None), "exited");
    }

    #[test]
    fn integrating_and_final_stages() {
        let w = worker("U-002", "integrating");
        let g = gate(&["U-002"]);
        assert_eq!(stage(&w, RunState::Alive, Some(&g)), "gate");
        assert_eq!(
            stage(&w, RunState::Alive, Some(&gate(&["U-009"]))),
            "queued"
        );
        assert_eq!(stage(&w, RunState::Alive, None), "queued");
        assert_eq!(stage(&w, RunState::Died, Some(&g)), "integrating");
        assert_eq!(stage(&w, RunState::Ended, None), "integrating");
        let done = worker("U-003", "rejected-gate");
        assert_eq!(stage(&done, RunState::Alive, None), "rejected-gate");
    }

    #[test]
    fn untracked_lines() {
        let dir = std::env::temp_dir().join(format!("vitrify-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, body: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        assert_eq!(count_lines(&file("a.txt", b"one\ntwo\n")), 2);
        assert_eq!(count_lines(&file("b.txt", b"one\ntwo")), 2);
        assert_eq!(count_lines(&file("c.txt", b"")), 0);
        assert_eq!(count_lines(&file("d.bin", b"PK\x03\x04\0\0\nmore\n")), 0);
        assert_eq!(count_lines(&dir.join("missing")), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lane_records_are_found_through_project_json() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path();
        // a job from before lanes: its records are its own directory
        assert_eq!(lane_dirs(logs), vec![(String::new(), logs.to_path_buf())]);
        std::fs::write(
            logs.join("project.json"),
            r#"{"job_id": "run-x", "lanes": [
                {"sdd": "specs/a", "slug": "a", "state": "build"},
                {"sdd": "specs/b", "slug": "../b", "state": "build"},
                {"sdd": "specs/c", "slug": "c", "state": "complete"}]}"#,
        )
        .unwrap();
        assert_eq!(
            lane_dirs(logs),
            vec![
                ("a".to_string(), logs.join("a")),
                ("c".to_string(), logs.join("c"))
            ]
        );
    }

    #[test]
    fn both_shapes_of_project_json_name_a_root_and_a_state() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path();
        assert!(project_lanes(logs).is_none());
        // the minimal shape: the SDD's path is the root, there are no counts
        std::fs::write(
            logs.join("project.json"),
            r#"{"job_id": "run-x", "lanes": [{"sdd": "specs/a", "slug": "a", "state": "idle"}]}"#,
        )
        .unwrap();
        let lanes = project_lanes(logs).unwrap();
        assert_eq!(
            (lanes[0].root.as_str(), lanes[0].state.as_str()),
            ("specs/a", "idle")
        );
        assert_eq!((lanes[0].units, lanes[0].verified), (None, None));
        // the project record's: a root, and the plan's counts
        std::fs::write(
            logs.join("project.json"),
            r#"{"job_id": "run-x", "lanes": [{"slug": "a", "root": "specs/a",
                "state": "up to date", "units": 4, "verified": 4, "regenerate": 0,
                "adopt": 0, "recheck": 0}], "critical_path": []}"#,
        )
        .unwrap();
        let lanes = project_lanes(logs).unwrap();
        assert_eq!(
            (lanes[0].root.as_str(), lanes[0].state.as_str()),
            ("specs/a", "up to date")
        );
        assert_eq!((lanes[0].units, lanes[0].verified), (Some(4), Some(4)));
    }

    #[test]
    fn gate_json_is_read_with_defaults() {
        let v: Value = serde_json::from_str(
            r#"{"step": "rows", "since": "2026-11-16T14:08:01Z", "batch": ["U-002"],
                "running": [{"command": "pytest -q", "since": "2026-11-16T14:08:03Z"}],
                "done": 3, "total": 7}"#,
        )
        .unwrap();
        let g = parse_gate(&v).unwrap();
        assert_eq!(g.step, "rows");
        assert_eq!(g.batch, vec!["U-002"]);
        assert_eq!(g.running[0].0, "pytest -q");
        assert_eq!((g.done, g.total), (Some(3), Some(7)));
        let g = parse_gate(&serde_json::json!({"step": "idle"})).unwrap();
        assert!(g.batch.is_empty() && g.done.is_none());
        assert!(parse_gate(&serde_json::json!([])).is_none());
    }
}
