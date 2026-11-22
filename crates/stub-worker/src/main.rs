//! stub-worker — deterministic stand-in for an inference worker: builds
//! units from the example's canned reference implementation. Parses the
//! `vitrify-job` block from its prompt (the final argv argument) and behaves
//! per `STUB_MODE`:
//!
//! * `build` — copy the unit's artifacts from the solution tree named by
//!   `STUB_REF`, mapped by the TOML file named by `STUB_MAP`, into the
//!   worktree's owned paths; run
//!   `vitrify trace record --unit U --stamp built --commit <sha>`; emit the
//!   built report line. The worktree is left dirty — workers never
//!   run `git commit`; the integrator commits the handoff.
//! * `block` — emit a `blocked` report with a `q` (`STUB_Q`, default Q-999).
//! * `garbage` — emit non-JSON.
//! * `hang` — sleep indefinitely (killed by timeout and interrupt tests).
//!
//! Per-unit switches, for rehearsals:
//!
//! * `STUB_BLOCK_UNIT` — that unit blocks while others build (planted Q).
//! * `STUB_HANG_UNIT` — that unit hangs while others build.
//! * `STUB_BLOCK_UNLESS=<unit>:<text>` — that unit blocks unless its prompt
//!   (which inlines its reading set) contains the text: blocked until the
//!   spec is fixed.
//! * `STUB_HANG_UNLESS=<unit>:<text>` — likewise, but it hangs: a worker
//!   still building when the spec changes under it.
//! * `STUB_DRAFT_UNIT` — that unit's first attempt writes a wrong draft of
//!   its artifacts (the gate rejects it); an attempt that finds the draft
//!   already in its worktree — carried over from the last one — builds
//!   properly.
//! * `STUB_RENDEZVOUS=<dir>:<n>` — before building, leave a marker in
//!   `<dir>` and wait until `<n>` workers have: proof they ran together.
//! * `STUB_STREAM` — wrap the output in a harness event stream (one JSON
//!   event per line, the report in the final `result` event).
//! * `STUB_VERDICT=<verdict>` — a prompt with no job block is an escalation:
//!   answer it as the judge, with that verdict.
//! * `STUB_STEPS=<dir>` — in `build` mode, pause at `start` (before any
//!   artifact), `stamp` (after the artifacts, before `trace record`) and
//!   `report` (after stamping, before the report line). At each point, say
//!   `stub: at <point>` on stderr, create `<dir>/<unit>-c<seq>.<point>` and
//!   wait until `<dir>/<unit>-c<seq>.<point>.go` exists: a test steps a worker
//!   through its stages. After 120 s it reports `released`. The copied
//!   artifacts get a fresh modification time, as files an agent wrote would.
//!
//! * `STUB_MAP` may contain `{sdd}`, replaced by the job's SDD slug (the
//!   `sdd/<slug>/<unit>/c<seq>` branch's slug; the SDD path's last
//!   component when the branch has none), so one setting
//!   (`STUB_MAP=…/stub-map-{sdd}.toml`) serves every lane of a project run.
//!   A plain path is used as it is.
//! * `STUB_SLEEP_MS=<ms>` — in `build` mode, sleep that long after starting,
//!   before any artifact: every unit takes a fixed time (makespan
//!   measurements).
//! * `STUB_SLEEP_MAP=<file>` — per-unit durations, one per line:
//!   `<sdd> <unit> <ms>` or `<unit> <ms>` (blank lines and `#` comments are
//!   ignored). The `<sdd> <unit>` line wins over the `<unit>` line, which
//!   wins over `STUB_SLEEP_MS`.
//! * `STUB_LOG=<file>` — append one line when a worker starts and one when
//!   it ends (`hang` never ends): `<event> <sdd> <unit> <seq> <ms>`, with
//!   `<event>` `start` or `end`, the SDD slug, the unit, the claim seq and
//!   the milliseconds since the Unix epoch (the same clock in every worker
//!   process, so tests compare lines across them). Each line is one
//!   `O_APPEND` write, so concurrent workers do not interleave.
//!
//! Every run notes its `TMPDIR` and whether its prompt carried a session
//! note on stderr.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prompt = args.last().cloned().unwrap_or_default();
    let job = parse_job(&prompt);
    let unit = job.get("unit").cloned().unwrap_or_default();
    let branch = job.get("branch").cloned().unwrap_or_default();
    let sdd = job.get("sdd").cloned().unwrap_or_default();
    // no job block: the escalation prompt — answer as the judge
    if unit.is_empty()
        && let Ok(verdict) = std::env::var("STUB_VERDICT")
    {
        println!(
            "{{\"verdict\":{},\"unit\":null,\"q\":null,\"reason\":\"stub judge\"}}",
            serde_json::to_string(&verdict).unwrap_or_default()
        );
        return;
    }
    eprintln!(
        "stub: TMPDIR={}",
        std::env::var("TMPDIR").unwrap_or_default()
    );
    eprintln!(
        "stub: session={} earlier-attempt={}",
        prompt.contains("## This session"),
        prompt.contains("An earlier attempt at this unit")
    );
    let slug = sdd_slug(&branch, &sdd);
    let claim_seq: i64 = branch
        .rsplit('/')
        .next()
        .and_then(|t| t.strip_prefix('c'))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    let mut mode = std::env::var("STUB_MODE").unwrap_or_else(|_| "build".to_string());
    if std::env::var("STUB_BLOCK_UNIT").is_ok_and(|u| u == unit) {
        mode = "block".to_string();
    }
    if std::env::var("STUB_HANG_UNIT").is_ok_and(|u| u == unit) {
        mode = "hang".to_string();
    }
    // `<unit>:<text>` — applies to that unit until its prompt has the text
    let unless = |var: &str| {
        std::env::var(var).is_ok_and(|spec| {
            spec.split_once(':')
                .is_some_and(|(u, text)| u == unit && !prompt.contains(text))
        })
    };
    if unless("STUB_BLOCK_UNLESS") {
        mode = "block".to_string();
    }
    if unless("STUB_HANG_UNLESS") {
        mode = "hang".to_string();
    }
    if let Ok(spec) = std::env::var("STUB_RENDEZVOUS")
        && let Some((dir, n)) = spec.rsplit_once(':')
        && mode == "build"
        && !rendezvous(Path::new(dir), &unit, n.parse().unwrap_or(1))
    {
        report(
            &unit,
            "released",
            &branch,
            claim_seq,
            None,
            "rendezvous timed out: the other workers never ran alongside",
        );
        return;
    }

    if !unit.is_empty() {
        log_event("start", &slug, &unit, claim_seq);
    }
    if mode == "build"
        && let Some(ms) = sleep_ms(&slug, &unit)
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    match mode.as_str() {
        "hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        },
        "garbage" => {
            println!("this is not a report");
            println!("still not json");
        }
        "block" => {
            let q = std::env::var("STUB_Q").unwrap_or_else(|_| "Q-999".to_string());
            report(
                &unit,
                "blocked",
                &branch,
                claim_seq,
                Some(&q),
                "stub blocked on a spec gap",
            );
        }
        _ => build(&unit, &branch, &sdd, &slug, claim_seq),
    }
    if !unit.is_empty() {
        log_event("end", &slug, &unit, claim_seq);
    }
}

/// The SDD's slug: the `<slug>` of an `sdd/<slug>/<unit>/c<seq>` branch, else
/// the last component of the job's SDD path without its `.sdd.md`.
fn sdd_slug(branch: &str, sdd: &str) -> String {
    if let Some(rest) = branch.strip_prefix("sdd/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 3 {
            return parts[..parts.len() - 2].join("/");
        }
    }
    let last = sdd.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    last.strip_suffix(".sdd.md").unwrap_or(last).to_string()
}

/// `STUB_SLEEP_MAP` entry for this unit, else `STUB_SLEEP_MS`.
fn sleep_ms(slug: &str, unit: &str) -> Option<u64> {
    if let Ok(path) = std::env::var("STUB_SLEEP_MAP") {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut by_unit = None;
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [s, u, ms] if *s == slug && *u == unit => {
                    if let Ok(ms) = ms.parse() {
                        return Some(ms);
                    }
                }
                [u, ms] if !u.starts_with('#') && *u == unit => by_unit = ms.parse().ok(),
                _ => {}
            }
        }
        if by_unit.is_some() {
            return by_unit;
        }
    }
    std::env::var("STUB_SLEEP_MS").ok()?.parse().ok()
}

/// `STUB_LOG`: one `O_APPEND` write of one line.
fn log_event(event: &str, slug: &str, unit: &str, claim_seq: i64) {
    let Ok(path) = std::env::var("STUB_LOG") else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("{event} {slug} {unit} {claim_seq} {ms}\n");
    if let Ok(mut f) = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// Leave a marker, then wait (at most 20 s) until `n` workers have.
fn rendezvous(dir: &Path, unit: &str, n: usize) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(unit), "here\n");
    for _ in 0..400 {
        let present = std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0);
        if present >= n {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

fn parse_job(prompt: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut inside = false;
    for line in prompt.lines() {
        if line.trim() == "```vitrify-job" {
            inside = true;
            continue;
        }
        if inside {
            if line.trim() == "```" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    map
}

fn sh(args: &[&str]) -> (bool, String) {
    let out = Command::new(args[0]).args(&args[1..]).output();
    match out {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, e.to_string()),
    }
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Set the modification time of every file under `path` to now.
fn touch_tree(path: &Path) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            touch_tree(&entry.path());
        }
    } else if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// `STUB_STEPS`: announce the point, then wait for the test to release it.
/// Returns false, having reported `released`, if it never does; true at once
/// when the switch is unset.
fn step(point: &str, unit: &str, branch: &str, claim_seq: i64) -> bool {
    let Ok(dir) = std::env::var("STUB_STEPS") else {
        return true;
    };
    let dir = PathBuf::from(dir);
    let marker = dir.join(format!("{unit}-c{claim_seq}.{point}"));
    let go = dir.join(format!("{unit}-c{claim_seq}.{point}.go"));
    eprintln!("stub: at {point}");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&marker, "here\n");
    for _ in 0..2400 {
        if go.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    report(
        unit,
        "released",
        branch,
        claim_seq,
        None,
        &format!("steps timed out at {point}"),
    );
    false
}

fn build(unit: &str, branch: &str, sdd: &str, slug: &str, claim_seq: i64) {
    if !step("start", unit, branch, claim_seq) {
        return;
    }
    let solution = PathBuf::from(std::env::var("STUB_REF").unwrap_or_default());
    let map_path = PathBuf::from(
        std::env::var("STUB_MAP")
            .unwrap_or_default()
            .replace("{sdd}", slug),
    );
    let vitrify = std::env::var("VITRIFY_BIN").unwrap_or_else(|_| "vitrify".to_string());

    // STUB_MAP: [units] U-001 = ["src/a.txt", ...]
    let map_text = std::fs::read_to_string(&map_path).unwrap_or_default();
    let table: toml::Table = map_text.parse().unwrap_or_default();
    let paths: Vec<String> = table
        .get("units")
        .and_then(|u| u.as_table())
        .and_then(|t| t.get(unit))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        report(
            unit,
            "released",
            branch,
            claim_seq,
            None,
            "no solution mapping for unit",
        );
        return;
    }
    // the draft rehearsal: a first attempt that gets it wrong
    let drafting = std::env::var("STUB_DRAFT_UNIT").is_ok_and(|u| u == unit)
        && !paths.iter().any(|rel| Path::new(rel).exists());
    for rel in &paths {
        if drafting {
            let target = PathBuf::from(rel);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&target, "draft\n");
            continue;
        }
        if let Err(e) = copy_tree(&solution.join(rel), &PathBuf::from(rel)) {
            report(
                unit,
                "released",
                branch,
                claim_seq,
                None,
                &format!("copy failed: {e}"),
            );
            return;
        }
    }
    // a copy can keep the solution's modification time (a clone on macOS
    // does): written now, as an agent would, so a stepped worker's files
    // read as its own edits rather than as carried over
    if std::env::var("STUB_STEPS").is_ok() && !drafting {
        for rel in &paths {
            touch_tree(Path::new(rel));
        }
    }
    if !step("stamp", unit, branch, claim_seq) {
        return;
    }
    // stamp Built with traces (the one hashing implementation); the dirty
    // worktree is the handoff — committing it is the integrator's act
    let (ok, out) = sh(&["git", "rev-parse", "--short", "HEAD"]);
    let head = if ok {
        out.trim().to_string()
    } else {
        "0000000".into()
    };
    // `--sdd` as the job block gives it: relative to the worktree root, so
    // the verb finds this SDD among however many the repository holds
    let mut record = vec![
        vitrify.as_str(),
        "trace",
        "record",
        "--unit",
        unit,
        "--stamp",
        "built",
        "--commit",
        &head,
    ];
    if !sdd.is_empty() {
        record.extend(["--sdd", sdd]);
    }
    let (ok, out) = sh(&record);
    if !ok {
        report(
            unit,
            "released",
            branch,
            claim_seq,
            None,
            &format!("trace record failed: {}", out.lines().last().unwrap_or("")),
        );
        return;
    }
    if !step("report", unit, branch, claim_seq) {
        return;
    }
    report(
        unit,
        "built",
        branch,
        claim_seq,
        None,
        if drafting { "stub draft" } else { "stub built" },
    );
}

fn report(unit: &str, outcome: &str, branch: &str, claim_seq: i64, q: Option<&str>, notes: &str) {
    // the report line — its fixed key order, exactly once, last
    let jstr = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    let qv = match q {
        Some(q) => jstr(q),
        None => "null".to_string(),
    };
    let line = format!(
        "{{\"unit\":{},\"outcome\":{},\"branch\":{},\"claim_seq\":{},\"q\":{},\"notes\":{}}}",
        jstr(unit),
        jstr(outcome),
        jstr(branch),
        claim_seq,
        qv,
        jstr(notes)
    );
    if std::env::var("STUB_STREAM").is_ok() {
        // a harness event stream: the session as it happened, the report in
        // the closing `result` event
        let text = format!("Working on {unit}.\n\n{line}");
        println!("{{\"type\":\"system\",\"subtype\":\"init\"}}");
        println!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}}}",
            jstr("Working on it.")
        );
        println!(
            "{{\"type\":\"result\",\"subtype\":\"success\",\"result\":{}}}",
            jstr(&text)
        );
    } else {
        println!("{line}");
    }
}
