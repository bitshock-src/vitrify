//! What past worker sessions took, as weights for the priority order.
//!
//! With `run.priority = "history"` a unit weighs the seconds its worker
//! sessions have taken, so the critical path is counted by estimated
//! duration instead of by units — the way Ninja orders jobs by their
//! runtime history. Everything here reads the records earlier runs left
//! under `.vitrify/logs/<job>/`: each job's `run.json` lists its workers
//! (`unit`, `started_at`, `ended_at`, `outcome`, and `lane`, the slug of
//! the SDD the unit belongs to). A record from before lanes has no `lane`;
//! its unit is credited to the slug `project.json` gives the run's SDD, or,
//! without one, to the name of the `sdd` path the run recorded (the file
//! stem of a single file, the directory of a bundle).
//!
//! The estimate for a unit is the median of the durations of its last
//! [`RECENT`] sessions that verified — a median because one hung or lucky
//! session should not move it — in job order (job ids carry their start
//! time), then in the order a run recorded them. A unit none of whose
//! sessions verified is estimated from its last [`RECENT`] finished
//! sessions of any outcome: a failed attempt still measures the size of the
//! unit. A unit with no finished session has no entry; [`weights_for`] gives
//! it the median of the known weights, or 1 when nothing is known.
//! Unreadable or partial records are skipped, never fatal.

use std::collections::BTreeMap;
use std::path::Path;

use vitrify_core::graph::Node;
use vitrify_core::model::UtcTime;

/// How many of a unit's latest sessions the estimate looks at.
pub const RECENT: usize = 5;

/// A finished session: how long it ran, in seconds, and whether it verified.
struct Session {
    node: Node,
    seconds: u64,
    verified: bool,
}

/// Each unit's estimated weight in seconds, from the records under
/// `<scratch>/logs/` (`scratch` is the repository's `.vitrify`). Units with
/// no finished session are absent.
pub fn weights(scratch: &Path) -> BTreeMap<Node, u64> {
    let logs = scratch.join("logs");
    let mut jobs: Vec<String> = match std::fs::read_dir(&logs) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => return BTreeMap::new(),
    };
    jobs.sort();
    let mut sessions: Vec<Session> = Vec::new();
    for job in &jobs {
        sessions.extend(job_sessions(&logs.join(job)));
    }
    estimate(&sessions)
}

/// `known` completed for `units`: a unit without an entry weighs the median
/// of the known weights, 1 when there are none.
pub fn weights_for<'a>(
    known: &BTreeMap<Node, u64>,
    units: impl IntoIterator<Item = &'a Node>,
) -> BTreeMap<Node, u64> {
    let fallback = median(known.values().copied().collect()).unwrap_or(1);
    units
        .into_iter()
        .map(|n| (n.clone(), known.get(n).copied().unwrap_or(fallback).max(1)))
        .collect()
}

/// The estimate over `sessions`, which are in chronological order.
fn estimate(sessions: &[Session]) -> BTreeMap<Node, u64> {
    let mut by_unit: BTreeMap<&Node, Vec<&Session>> = BTreeMap::new();
    for s in sessions {
        by_unit.entry(&s.node).or_default().push(s);
    }
    let mut out = BTreeMap::new();
    for (node, all) in by_unit {
        let won: Vec<&Session> = all.iter().copied().filter(|s| s.verified).collect();
        let pool = if won.is_empty() { all } else { won };
        let recent = &pool[pool.len().saturating_sub(RECENT)..];
        if let Some(m) = median(recent.iter().map(|s| s.seconds).collect()) {
            out.insert(node.clone(), m.max(1));
        }
    }
    out
}

/// The median; of an even count, the mean of the middle two rounded up.
fn median(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        let (a, b) = (v[n / 2 - 1], v[n / 2]);
        a / 2 + b / 2 + (a % 2 + b % 2).div_ceil(2)
    })
}

/// The finished sessions in one job's `run.json`; nothing when it cannot
/// be read.
fn job_sessions(dir: &Path) -> Vec<Session> {
    let Some(run) = read_json(&dir.join("run.json")) else {
        return Vec::new();
    };
    let Some(workers) = run.get("workers").and_then(|w| w.as_array()) else {
        return Vec::new();
    };
    let lanes = project_lanes(dir);
    let fallback = run
        .get("sdd")
        .and_then(|s| s.as_str())
        .map(|sdd| {
            lanes
                .iter()
                .find(|(path, _)| path == sdd)
                .map(|(_, slug)| slug.clone())
                .or_else(|| (lanes.len() == 1).then(|| lanes[0].1.clone()))
                .unwrap_or_else(|| name_of(sdd))
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for w in workers {
        let text = |k: &str| w.get(k).and_then(|v| v.as_str());
        let (Some(unit), Some(start), Some(end)) =
            (text("unit"), text("started_at"), text("ended_at"))
        else {
            continue;
        };
        let (Some(start), Some(end)) = (UtcTime::parse(start), UtcTime::parse(end)) else {
            continue;
        };
        if end.0 < start.0 || unit.is_empty() {
            continue;
        }
        let slug = text("lane").filter(|s| !s.is_empty()).unwrap_or(&fallback);
        if slug.is_empty() {
            continue;
        }
        out.push(Session {
            node: Node::new(slug, unit),
            seconds: (end.0 - start.0) as u64,
            verified: text("outcome") == Some("verified"),
        });
    }
    out
}

/// `(sdd path, slug)` for each lane `project.json` lists; empty when it is
/// absent or unreadable.
fn project_lanes(dir: &Path) -> Vec<(String, String)> {
    let Some(project) = read_json(&dir.join("project.json")) else {
        return Vec::new();
    };
    let Some(lanes) = project.get("lanes").and_then(|l| l.as_array()) else {
        return Vec::new();
    };
    lanes
        .iter()
        .filter_map(|l| {
            let sdd = l.get("sdd")?.as_str()?;
            let slug = l.get("slug")?.as_str()?;
            Some((sdd.to_string(), slug.to_string()))
        })
        .collect()
}

/// The name an SDD path stands for: a single file's stem, a bundle's
/// directory.
fn name_of(sdd: &str) -> String {
    let p = Path::new(sdd.trim_end_matches('/'));
    if p.extension().is_some() {
        p.file_stem()
    } else {
        p.file_name()
    }
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_string()
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(slug: &str, unit: &str) -> Node {
        Node::new(slug, unit)
    }

    /// A worker record that ran `secs` seconds.
    fn worker(lane: Option<&str>, unit: &str, secs: i64, outcome: &str) -> serde_json::Value {
        let ts = |t: i64| UtcTime(1_700_000_000 + t).format();
        let mut w = json!({
            "unit": unit, "started_at": ts(0), "ended_at": ts(secs),
            "outcome": outcome,
        });
        if let Some(l) = lane {
            w["lane"] = json!(l);
        }
        w
    }

    fn put(scratch: &Path, job: &str, run: serde_json::Value, project: Option<serde_json::Value>) {
        let dir = scratch.join("logs").join(job);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.json"), run.to_string()).unwrap();
        if let Some(p) = project {
            std::fs::write(dir.join("project.json"), p.to_string()).unwrap();
        }
    }

    #[test]
    fn the_median_of_the_last_verified_sessions() {
        let d = tempfile::tempdir().unwrap();
        // seven verified sessions over seven jobs: only the last five count
        for (i, secs) in [900, 800, 10, 30, 20, 50, 40].into_iter().enumerate() {
            let run = json!({ "workers": [worker(Some("core"), "U-001", secs, "verified")] });
            put(
                d.path(),
                &format!("run-2026010{}T000000Z-p1", i + 1),
                run,
                None,
            );
        }
        // a failed attempt after them does not count while some verified
        let run = json!({ "workers": [worker(Some("core"), "U-001", 5000, "rejected-gate")] });
        put(d.path(), "run-20260201T000000Z-p1", run, None);
        let w = weights(d.path());
        assert_eq!(w.get(&n("core", "U-001")), Some(&30));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn a_unit_that_never_verified_is_estimated_from_all_its_sessions() {
        let d = tempfile::tempdir().unwrap();
        let mut running = worker(Some("a"), "U-002", 60, "running");
        running["ended_at"] = json!(null);
        let run = json!({ "workers": [
            worker(Some("a"), "U-001", 100, "rejected-gate"),
            worker(Some("a"), "U-001", 300, "timeout"),
            running,
        ]});
        put(d.path(), "run-20260101T000000Z-p1", run, None);
        let w = weights(d.path());
        // mean of 100 and 300; the unfinished session is not measured
        assert_eq!(w.get(&n("a", "U-001")), Some(&200));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn a_record_from_before_lanes_reads_through_project_json_or_the_sdd_path() {
        let d = tempfile::tempdir().unwrap();
        let old = |sdd: &str, secs| json!({ "sdd": sdd, "workers": [worker(None, "U-001", secs, "verified")] });
        let project =
            json!({ "lanes": [{ "sdd": "specs/calc/sdd.md", "slug": "calc", "state": "build" }] });
        put(
            d.path(),
            "run-20260101T000000Z-p1",
            old("specs/calc/sdd.md", 11),
            Some(project),
        );
        put(
            d.path(),
            "run-20260102T000000Z-p1",
            old("specs/solo.md", 22),
            None,
        );
        put(
            d.path(),
            "run-20260103T000000Z-p1",
            old("specs/bundle/", 33),
            None,
        );
        let w = weights(d.path());
        assert_eq!(w.get(&n("calc", "U-001")), Some(&11));
        assert_eq!(w.get(&n("solo", "U-001")), Some(&22));
        assert_eq!(w.get(&n("bundle", "U-001")), Some(&33));
    }

    #[test]
    fn unreadable_and_partial_records_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        assert!(weights(&d.path().join("nothing")).is_empty());
        let logs = d.path().join("logs");
        std::fs::create_dir_all(logs.join("run-1")).unwrap();
        std::fs::write(logs.join("run-1/run.json"), "{ not json").unwrap();
        std::fs::create_dir_all(logs.join("run-2")).unwrap(); // no run.json
        std::fs::write(logs.join("stray"), "x").unwrap(); // not a job
        let mut bad = worker(Some("a"), "U-001", 10, "verified");
        bad["started_at"] = json!("yesterday");
        let backwards = worker(Some("a"), "U-001", -5, "verified");
        let run = json!({ "workers": [bad, backwards, 7, {"unit": "U-009"},
            worker(Some("a"), "U-002", 42, "verified")] });
        put(d.path(), "run-3", run, None);
        put(d.path(), "run-4", json!({ "workers": "many" }), None);
        let w = weights(d.path());
        assert_eq!(w, BTreeMap::from([(n("a", "U-002"), 42)]));
    }

    #[test]
    fn a_unit_without_history_takes_the_median_of_the_known_weights() {
        let known = BTreeMap::from([
            (n("a", "U-001"), 10),
            (n("a", "U-002"), 50),
            (n("b", "U-001"), 20),
        ]);
        let units = [n("a", "U-001"), n("a", "U-003")];
        let w = weights_for(&known, &units);
        assert_eq!(
            w,
            BTreeMap::from([(n("a", "U-001"), 10), (n("a", "U-003"), 20)])
        );
        // nothing known: every unit weighs 1
        let w = weights_for(&BTreeMap::new(), &units);
        assert!(w.values().all(|&x| x == 1));
        assert_eq!(median(vec![1, 2, 3, 4]), Some(3));
        assert_eq!(median(vec![u64::MAX, u64::MAX]), Some(u64::MAX));
    }

    #[test]
    fn a_zero_second_session_weighs_one() {
        let d = tempfile::tempdir().unwrap();
        let run = json!({ "workers": [worker(Some("a"), "U-001", 0, "verified")] });
        put(d.path(), "run-1", run, None);
        assert_eq!(weights(d.path()).get(&n("a", "U-001")), Some(&1));
    }
}
