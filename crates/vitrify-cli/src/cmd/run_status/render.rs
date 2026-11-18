//! Showing a snapshot: the human view, the JSON object, and the formatting
//! the follower's event lines share with them.

use vitrify_core::model::UtcTime;

use super::snapshot::{GateView, RunState, Snapshot, WorkerView};
use crate::out::J;

/// The human view, one line per element.
pub fn human(s: &Snapshot) -> Vec<String> {
    let since = |t: &str| UtcTime::parse(t).map(|t| s.at.0 - t.0).unwrap_or(0);
    let mut out = Vec::new();

    let state = match (&s.state, &s.terminated) {
        (RunState::Ended, Some(t)) => {
            let ran = UtcTime::parse(&t.at)
                .zip(UtcTime::parse(&s.started_at))
                .map(|(end, start)| end.0 - start.0)
                .unwrap_or(0);
            let detail = if t.detail.is_empty() {
                String::new()
            } else {
                format!(" ({})", t.detail)
            };
            format!("ended: {}{detail} · ran {}", t.state, duration(ran))
        }
        (RunState::Died, _) => format!("died · started {} ago", duration(since(&s.started_at))),
        _ => format!("alive · {} elapsed", duration(since(&s.started_at))),
    };
    let mut header = format!("{} · {state} · {}", s.job_id, s.sdd);
    if !s.harness.is_empty() {
        header.push_str(&format!(" · {}", s.harness));
    }
    out.push(header);
    out.push(match &s.progress {
        Some(p) => p.clone(),
        None => format!(
            "Progress: ? ({} could not be read in the primary checkout)",
            s.sdd
        ),
    });

    out.push(String::new());
    let active: Vec<(usize, &WorkerView)> = s
        .workers
        .iter()
        .enumerate()
        .filter(|(_, w)| w.outcome == "running" || w.outcome == "integrating")
        .collect();
    if active.is_empty() {
        out.push("No unit in progress.".to_string());
    } else {
        let mut rows = vec![
            ["UNIT", "TRY", "STAGE", "TIME", "OUTPUT", "WORK"]
                .map(str::to_string)
                .to_vec(),
        ];
        let budget = duration(s.params.worker_timeout_minutes * 60);
        for (i, w) in active {
            let running = w.outcome == "running";
            let mut time = duration(since(&w.started_at));
            if running {
                time.push('/');
                time.push_str(&budget);
            }
            let output = match (running, w.log_quiet_s, w.log_bytes) {
                (true, Some(q), Some(b)) => format!("{} ago  {}", duration(q), size(b)),
                _ => String::new(),
            };
            let mut work_text = match w.stage.as_str() {
                "reading" | "editing" | "stamped" => {
                    let mut t = work(
                        w.files_changed.unwrap_or(0),
                        w.insertions.unwrap_or(0),
                        w.deletions.unwrap_or(0),
                    );
                    let carried = w.carried_over.unwrap_or(0);
                    if carried > 0 {
                        t.push_str(&format!(" ({carried} carried over)"));
                    }
                    t
                }
                "unknown" => format!("? {}", w.error.as_deref().unwrap_or_default()),
                "exited" => "process ended".to_string(),
                "queued" => "waiting for the gate".to_string(),
                "gate" => s
                    .gate
                    .as_ref()
                    .map(|g| gate_text(g, s.at))
                    .unwrap_or_default(),
                "integrating" => "reported built".to_string(),
                _ => String::new(),
            };
            if w.retry {
                match s.workers[..i].iter().rev().find(|p| p.unit == w.unit) {
                    Some(p) => {
                        work_text.push_str(&format!(" (retry: c{} {})", p.claim_seq, p.outcome))
                    }
                    None => work_text.push_str(" (retry)"),
                }
            }
            let stage = if w.stage == "unknown" { "?" } else { &w.stage };
            rows.push(vec![
                w.unit.clone(),
                format!("c{}", w.claim_seq),
                stage.to_string(),
                time,
                output,
                work_text,
            ]);
        }
        out.extend(table(&rows));
    }

    if let Some(g) = &s.gate
        && s.state == RunState::Alive
        && g.step != "idle"
        && !s.workers.iter().any(|w| w.stage == "gate")
    {
        out.push(format!("Gate: {}", gate_text(g, s.at)));
    }

    if !s.warnings.is_empty() {
        out.push(String::new());
        out.extend(s.warnings.iter().map(|w| format!("! {w}")));
    }

    let finished: Vec<&WorkerView> = s
        .workers
        .iter()
        .filter(|w| w.outcome != "running" && w.outcome != "integrating")
        .collect();
    if !finished.is_empty() {
        out.push(String::new());
        out.push("Finished".to_string());
        let skipped = finished.len().saturating_sub(MAX_FINISHED);
        if skipped > 0 {
            out.push(format!(
                "… {skipped} earlier attempts — --json lists them all"
            ));
        }
        let rows: Vec<Vec<String>> = finished[skipped..]
            .iter()
            .map(|w| {
                let took = w
                    .ended()
                    .zip(w.started())
                    .map(|(end, start)| duration(end.0 - start.0))
                    .unwrap_or_default();
                vec![
                    w.unit.clone(),
                    format!("c{}", w.claim_seq),
                    w.outcome.clone(),
                    took,
                    w.detail.clone().unwrap_or_default(),
                ]
            })
            .collect();
        out.extend(table(&rows));
    }

    if let Some(report) = &s.report {
        out.push(String::new());
        out.push("Report:".to_string());
        out.extend(report.iter().cloned());
    }

    out.push(String::new());
    out.push(format!("Logs: {}", s.logs));
    out
}

/// The Finished section shows only the latest attempts; `--json` has them
/// all.
const MAX_FINISHED: usize = 20;

/// Rows as aligned columns: each padded to its widest cell (in chars, so
/// `…` and `·` count once), two spaces apart, no trailing blanks.
fn table(rows: &[Vec<String>]) -> Vec<String> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|r| {
            let mut line = String::new();
            for (c, cell) in r.iter().enumerate() {
                if c > 0 {
                    line.push_str("  ");
                }
                line.push_str(cell);
                let pad = widths[c] - cell.chars().count();
                line.extend(std::iter::repeat_n(' ', pad));
            }
            line.trim_end().to_string()
        })
        .collect()
}

/// What the integrator is doing, in words: the step, and for a step that
/// runs commands, the oldest one running and for how long.
pub fn gate_text(g: &GateView, now: UtcTime) -> String {
    let lead = match g.step.as_str() {
        "lock" => return "waiting for the integration checkout".to_string(),
        "sync" => return "syncing with mainline".to_string(),
        "merge" => return "merging".to_string(),
        "publish" => return "stamping and publishing".to_string(),
        "test-base" => "measuring the Test command before the merge".to_string(),
        "rows" => match (g.done, g.total) {
            (Some(done), Some(total)) => format!("rows {done}/{total}"),
            _ => "rows".to_string(),
        },
        "head-check" => "checking the mainline head".to_string(),
        other => other.to_string(),
    };
    let Some((cmd, since)) = g.running.first() else {
        return lead;
    };
    let mut text = format!("{lead} · {}", clip(cmd, 60));
    if let Some(since) = since {
        text.push_str(&format!(" ({})", duration(now.0 - since.0)));
    }
    if g.running.len() > 1 {
        text.push_str(&format!(" +{} more", g.running.len() - 1));
    }
    text
}

/// `s` cut to `max` chars, the last one `…` when anything was cut: gate
/// commands can be whole scripts.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The JSON object (`--json`).
pub fn json(s: &Snapshot) -> J {
    let opt_int = |v: Option<i64>| v.map_or(J::Null, J::Int);
    let opt_u64 = |v: Option<u64>| v.map_or(J::Null, |n| J::Int(n as i64));
    let opt_bool = |v: Option<bool>| v.map_or(J::Null, J::Bool);
    let workers = s
        .workers
        .iter()
        .map(|w| {
            J::Obj(vec![
                ("unit".into(), J::s(w.unit.clone())),
                ("claim_seq".into(), J::Int(w.claim_seq)),
                ("holder".into(), J::s(w.holder.clone())),
                ("pid".into(), J::Int(w.pid as i64)),
                ("retry".into(), J::Bool(w.retry)),
                ("worktree".into(), J::s(w.worktree.clone())),
                ("started_at".into(), J::s(w.started_at.clone())),
                ("ended_at".into(), J::opt_str(w.ended_at.clone())),
                ("outcome".into(), J::s(w.outcome.clone())),
                ("stage".into(), J::s(w.stage.clone())),
                ("alive".into(), opt_bool(w.alive)),
                ("log_bytes".into(), opt_u64(w.log_bytes)),
                ("log_quiet_s".into(), opt_int(w.log_quiet_s)),
                ("files_changed".into(), opt_u64(w.files_changed)),
                ("insertions".into(), opt_u64(w.insertions)),
                ("deletions".into(), opt_u64(w.deletions)),
                ("carried_over".into(), opt_u64(w.carried_over)),
                ("stamped".into(), opt_bool(w.stamped)),
                ("detail".into(), J::opt_str(w.detail.clone())),
                ("error".into(), J::opt_str(w.error.clone())),
            ])
        })
        .collect();
    let terminated = s.terminated.as_ref().map_or(J::Null, |t| {
        J::Obj(vec![
            ("state".into(), J::s(t.state.clone())),
            ("at".into(), J::s(t.at.clone())),
            ("detail".into(), J::s(t.detail.clone())),
        ])
    });
    J::Obj(vec![
        ("job_id".into(), J::s(s.job_id.clone())),
        ("pid".into(), J::Int(s.pid as i64)),
        ("sdd".into(), J::s(s.sdd.clone())),
        ("harness".into(), J::s(s.harness.clone())),
        ("started_at".into(), J::s(s.started_at.clone())),
        ("at".into(), J::s(s.at.format())),
        ("state".into(), J::s(s.state.as_str())),
        ("terminated".into(), terminated),
        (
            "report".into(),
            s.report.as_ref().map_or(J::Null, |lines| {
                J::Arr(lines.iter().map(|l| J::s(l.clone())).collect())
            }),
        ),
        ("progress".into(), J::opt_str(s.progress.clone())),
        (
            "params".into(),
            J::Obj(vec![
                ("cap".into(), J::Int(s.params.cap)),
                ("gate_jobs".into(), J::Int(s.params.gate_jobs)),
                ("lease_minutes".into(), J::Int(s.params.lease_minutes)),
                (
                    "worker_timeout_minutes".into(),
                    J::Int(s.params.worker_timeout_minutes),
                ),
            ]),
        ),
        ("workers".into(), J::Arr(workers)),
        ("gate".into(), s.gate.as_ref().map_or(J::Null, gate_json)),
        (
            "warnings".into(),
            J::Arr(s.warnings.iter().map(|w| J::s(w.clone())).collect()),
        ),
        ("logs".into(), J::s(s.logs.clone())),
    ])
}

/// The gate board in `gate.json`'s own key order, not the sorted order a
/// `serde_json` read would give.
pub fn gate_json(g: &GateView) -> J {
    let time = |t: &Option<UtcTime>| t.map_or(J::Null, |t| J::s(t.format()));
    J::Obj(vec![
        ("step".into(), J::s(g.step.clone())),
        ("since".into(), time(&g.since)),
        (
            "batch".into(),
            J::Arr(g.batch.iter().map(|u| J::s(u.clone())).collect()),
        ),
        (
            "running".into(),
            J::Arr(
                g.running
                    .iter()
                    .map(|(cmd, since)| {
                        J::Obj(vec![
                            ("command".into(), J::s(cmd.clone())),
                            ("since".into(), time(since)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("done".into(), g.done.map_or(J::Null, J::Int)),
        ("total".into(), g.total.map_or(J::Null, J::Int)),
    ])
}

/// `42s` under a minute, `12m` under an hour, `2h05m` beyond. Negative
/// spans (clock skew) show as zero.
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// `0 B`, `512 B`, `41 KB` (under 1 MiB: KiB, rounded), `3.2 MB`.
pub fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes + 512) / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// `HH:MM:SS` of a UTC time.
pub fn hhmmss(t: UtcTime) -> String {
    t.format()[11..19].to_string()
}

/// A worktree's changes: `no changes`, `1 file +40 -0`, `7 files +210 -14`.
pub fn work(files: u64, insertions: u64, deletions: u64) -> String {
    match files {
        0 => "no changes".to_string(),
        1 => format!("1 file +{insertions} -{deletions}"),
        n => format!("{n} files +{insertions} -{deletions}"),
    }
}

/// The warning for a run that is gone without recording an end.
pub fn died_line(pid: u32) -> String {
    format!(
        "the run (pid {pid}) is gone and recorded no end — the next `vitrify run` releases its claims and keeps its workers' files"
    )
}

/// The warning for a worker that outlived its run: the same wording a later
/// run uses when it finds one.
pub fn kill_line(job: &str, pgid: u32, unit: &str) -> String {
    format!(
        "a worker of {job} (process group {pgid}, unit {unit}) is still running — stop it with: kill -9 -- -{pgid}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(60), "1m");
        assert_eq!(duration(12 * 60 + 59), "12m");
        assert_eq!(duration(3600 + 5 * 60), "1h05m");
        assert_eq!(duration(2 * 3600 + 5 * 60 + 30), "2h05m");
        assert_eq!(duration(-5), "0s");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(41 * 1024 + 100), "41 KB");
        assert_eq!(size(1536), "2 KB");
        assert_eq!(size(3 * 1024 * 1024 + 210 * 1024), "3.2 MB");
    }

    #[test]
    fn clock_and_work() {
        assert_eq!(
            hhmmss(UtcTime::parse("2026-11-16T14:08:01Z").unwrap()),
            "14:08:01"
        );
        assert_eq!(work(0, 0, 0), "no changes");
        assert_eq!(work(1, 40, 0), "1 file +40 -0");
        assert_eq!(work(7, 210, 14), "7 files +210 -14");
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::cmd::run_status::snapshot::tests::worker;
    use crate::cmd::run_status::snapshot::{Params, RunState, Terminated};

    fn snap() -> Snapshot {
        let mut w = worker("U-001", "running");
        w.stage = "editing".to_string();
        w.alive = Some(true);
        w.log_bytes = Some(2048);
        w.log_quiet_s = Some(4);
        w.files_changed = Some(2);
        w.insertions = Some(40);
        w.deletions = Some(1);
        w.carried_over = Some(0);
        w.stamped = Some(false);
        let mut done = worker("U-002", "verified");
        done.stage = "verified".to_string();
        done.ended_at = Some("2026-11-16T14:08:00Z".to_string());
        Snapshot {
            job_id: "run-x".to_string(),
            pid: 77,
            sdd: "specs/calc".to_string(),
            harness: "claude".to_string(),
            started_at: "2026-11-16T14:00:00Z".to_string(),
            at: UtcTime::parse("2026-11-16T14:10:00Z").unwrap(),
            state: RunState::Alive,
            terminated: None,
            report: None,
            progress: Some("Progress: 1/2 Verified · 1 Claimed".to_string()),
            params: Params {
                cap: 2,
                gate_jobs: 4,
                lease_minutes: 60,
                worker_timeout_minutes: 45,
            },
            workers: vec![w, done],
            gate: Some(GateView {
                step: "rows".to_string(),
                since: UtcTime::parse("2026-11-16T14:09:00Z"),
                batch: vec!["U-003".to_string()],
                running: vec![(
                    "pytest tests/test_parse.py".to_string(),
                    UtcTime::parse("2026-11-16T14:09:18Z"),
                )],
                done: Some(3),
                total: Some(7),
            }),
            warnings: vec!["U-001 c1: no output for 6m (since 14:04:00)".to_string()],
            logs: ".vitrify/logs/run-x/".to_string(),
        }
    }

    fn keys(j: &J) -> Vec<&str> {
        match j {
            J::Obj(pairs) => pairs.iter().map(|(k, _)| k.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    fn field<'a>(j: &'a J, key: &str) -> &'a J {
        match j {
            J::Obj(pairs) => &pairs.iter().find(|(k, _)| k == key).unwrap().1,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn json_field_order() {
        let j = json(&snap());
        assert_eq!(
            keys(&j),
            [
                "job_id",
                "pid",
                "sdd",
                "harness",
                "started_at",
                "at",
                "state",
                "terminated",
                "report",
                "progress",
                "params",
                "workers",
                "gate",
                "warnings",
                "logs"
            ]
        );
        assert_eq!(
            keys(field(&j, "params")),
            [
                "cap",
                "gate_jobs",
                "lease_minutes",
                "worker_timeout_minutes"
            ]
        );
        let J::Arr(workers) = field(&j, "workers") else {
            panic!("workers is not an array")
        };
        assert_eq!(
            keys(&workers[0]),
            [
                "unit",
                "claim_seq",
                "holder",
                "pid",
                "retry",
                "worktree",
                "started_at",
                "ended_at",
                "outcome",
                "stage",
                "alive",
                "log_bytes",
                "log_quiet_s",
                "files_changed",
                "insertions",
                "deletions",
                "carried_over",
                "stamped",
                "detail",
                "error"
            ]
        );
        // a finished worker was not probed
        assert_eq!(field(&workers[1], "alive"), &J::Null);
        assert_eq!(field(&workers[1], "files_changed"), &J::Null);
        let gate = field(&j, "gate");
        assert_eq!(
            keys(gate),
            ["step", "since", "batch", "running", "done", "total"]
        );
        let J::Arr(running) = field(gate, "running") else {
            panic!("running is not an array")
        };
        assert_eq!(keys(&running[0]), ["command", "since"]);
        assert_eq!(field(&j, "state"), &J::s("alive"));
        assert_eq!(field(&j, "at"), &J::s("2026-11-16T14:10:00Z"));
        assert_eq!(field(&j, "report"), &J::Null);
    }

    #[test]
    fn an_ended_run_shows_its_report() {
        let mut s = snap();
        s.state = RunState::Ended;
        s.gate = None;
        s.terminated = Some(Terminated {
            state: "blocked".to_string(),
            at: "2026-11-16T14:10:00Z".to_string(),
            detail: "Progress: 1/2 Verified · 1 Ready".to_string(),
        });
        let report = vec![
            "Blocked.".to_string(),
            "  Progress: 1/2 Verified · 1 Ready".to_string(),
            "  U-001: open Q-001: a spec gap".to_string(),
            "  journal tail:".to_string(),
        ];
        s.report = Some(report.clone());
        let lines = human(&s);
        let at = lines.iter().position(|l| l == "Report:").unwrap();
        assert_eq!(lines[at + 1..at + 1 + report.len()], report[..]);
        assert_eq!(lines.last().unwrap(), "Logs: .vitrify/logs/run-x/");
        assert_eq!(
            field(&json(&s), "report"),
            &J::Arr(report.into_iter().map(J::s).collect())
        );
    }

    #[test]
    fn human_view() {
        let lines = human(&snap());
        assert_eq!(
            lines[0],
            "run-x · alive · 10m elapsed · specs/calc · claude"
        );
        assert_eq!(lines[1], "Progress: 1/2 Verified · 1 Claimed");
        assert_eq!(lines[3], "UNIT   TRY  STAGE    TIME     OUTPUT        WORK");
        assert_eq!(
            lines[4],
            "U-001  c1   editing  10m/45m  4s ago  2 KB  2 files +40 -1"
        );
        // U-003 is in the gate, but has no row: the gate gets its own line
        assert_eq!(
            lines[5],
            "Gate: rows 3/7 · pytest tests/test_parse.py (42s)"
        );
        assert!(lines.contains(&"! U-001 c1: no output for 6m (since 14:04:00)".to_string()));
        let fin = lines.iter().position(|l| l == "Finished").unwrap();
        assert_eq!(lines[fin + 1], "U-002  c1  verified  8m");
        assert_eq!(lines.last().unwrap(), "Logs: .vitrify/logs/run-x/");
    }

    #[test]
    fn gate_texts() {
        let mut g = snap().gate.unwrap();
        let now = UtcTime::parse("2026-11-16T14:10:00Z").unwrap();
        g.running.push(("ruff check .".to_string(), None));
        assert_eq!(
            gate_text(&g, now),
            "rows 3/7 · pytest tests/test_parse.py (42s) +1 more"
        );
        g.step = "lock".to_string();
        assert_eq!(gate_text(&g, now), "waiting for the integration checkout");
        g.step = "lint".to_string();
        g.running = vec![("x".repeat(70), None)];
        let text = gate_text(&g, now);
        assert_eq!(text, format!("lint · {}…", "x".repeat(59)));
        g.running.clear();
        g.step = "head-check".to_string();
        assert_eq!(gate_text(&g, now), "checking the mainline head");
    }
}
