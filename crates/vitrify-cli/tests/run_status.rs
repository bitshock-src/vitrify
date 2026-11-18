//! `vitrify run status` against live runs of run-mini, stepped through by
//! the stub worker: each stage of a worker, a quiet worker, a run that
//! died, the gate's step and command, `--follow` in both forms until the run
//! ends, and the replay of a finished run. Exact values are checked through
//! `--json`; the human view by tokens on a unit's line, because column
//! padding varies.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
}

fn stub_bin() -> PathBuf {
    // build once per test process: `cargo test` alone does not refresh
    // target/debug/stub-worker after source edits, and even a no-op
    // `cargo build` re-links the binary — racing tests already spawning it
    static STUB: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    STUB.get_or_init(|| {
        let vit = PathBuf::from(vitrify_bin());
        let stub = vit.parent().unwrap().join("stub-worker");
        assert!(
            Command::new(env!("CARGO"))
                .args(["build", "-p", "stub-worker", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        assert!(stub.exists(), "stub-worker binary missing after build");
        stub
    })
    .clone()
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let t = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

/// run-mini project committed on main, with a config.toml whose probe
/// record points the orchestrator at the stub worker.
fn rig() -> Rig {
    rig_with(|_| {})
}

/// The row U-001's gate runs while `$STATUS_HOLD` stands: the test decides
/// how long the gate stays at that command.
const HELD_ROW: &str =
    r#"`test -f src/one.txt && while [ -e "$STATUS_HOLD" ]; do sleep 0.05; done`"#;

/// run-mini with U-001's Verification row held (`HELD_ROW`): a gate that
/// stays in its rows step until the test removes the hold file.
fn held_rig() -> Rig {
    rig_with(|root| {
        let unit = root.join("specs/mini/units/U-001-one.md");
        let text = std::fs::read_to_string(&unit).unwrap();
        assert!(text.contains("`test -f src/one.txt`"), "{text}");
        std::fs::write(&unit, text.replace("`test -f src/one.txt`", HELD_ROW)).unwrap();
    })
}

/// run-mini, changed by `edit` before the seed commit.
fn rig_with(edit: impl FnOnce(&Path)) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&root).unwrap();
    copy_dir(&repo_root().join("fixtures/seeded/run-mini/project"), &root);
    let root = root.canonicalize().unwrap();
    edit(&root);
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);

    std::fs::create_dir_all(&home).unwrap();
    // the run's temp root: inside this rig, gone with it
    std::fs::create_dir_all(tmp.path().join("t")).unwrap();
    write_config(&home, &stub_bin());
    Rig {
        root,
        home,
        _tmp: tmp,
    }
}

impl Rig {
    fn command(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
        let mut c = Command::new(vitrify_bin());
        c.args(args)
            .current_dir(&self.root)
            .env("VITRIFY_HOME", &self.home)
            .env(
                "STUB_REF",
                repo_root().join("fixtures/seeded/run-mini/solution"),
            )
            .env(
                "STUB_MAP",
                repo_root().join("fixtures/seeded/run-mini/stub-map.toml"),
            )
            .env("VITRIFY_BIN", vitrify_bin())
            .env("STUB_MODE", "build")
            .env("TMPDIR", self._tmp.path().join("t"))
            // an idle run looks at mainline again every second
            .env("VITRIFY_RUN_POLL_SECONDS", "1");
        for (k, v) in extra_env {
            c.env(k, v);
        }
        c
    }

    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String, String) {
        let out = self.command(args, extra_env).output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// Start vitrify in the background, its output going to files.
    fn start(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Background {
        let out = self._tmp.path().join(format!(
            "run-{}.out",
            std::fs::read_dir(self._tmp.path()).unwrap().count()
        ));
        let err = out.with_extension("err");
        let child = self
            .command(args, extra_env)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(std::fs::File::create(&err).unwrap())
            .spawn()
            .unwrap();
        Background { child, out, err }
    }

    fn run_json(&self) -> Value {
        std::fs::read_to_string(self.root.join(".vitrify/run.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    }

    /// A scratch path in this rig, outside the repository.
    fn scratch(&self, name: &str) -> PathBuf {
        self._tmp.path().join(name)
    }

    /// `run status` with `args`: (exit code, stdout, stderr).
    fn status(&self, args: &[&str]) -> (i32, String, String) {
        let mut all = vec!["run", "status"];
        all.extend_from_slice(args);
        self.run(&all, &[])
    }

    /// `run status --json` with `args`, parsed; `Null` when it fails, so a
    /// poll can simply try again.
    fn status_json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let (code, stdout, _) = self.status(&all);
        if code != 0 {
            return Value::Null;
        }
        serde_json::from_str(&stdout).unwrap_or(Value::Null)
    }
}

/// A vitrify process in the background. Dropping it kills the process if it
/// still runs, so a failed assert never leaves a run or a follower behind.
struct Background {
    child: Child,
    out: PathBuf,
    err: PathBuf,
}

impl Background {
    fn stdout(&self) -> String {
        std::fs::read_to_string(&self.out).unwrap_or_default()
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.err).unwrap_or_default()
    }

    fn signal(&self, sig: &str) {
        let ok = Command::new("/bin/kill")
            .args([sig, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {sig}");
    }

    /// Wait for the process to end (at most 60 s).
    fn finish(mut self) -> (std::process::ExitStatus, String, String) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (status, self.stdout(), self.stderr());
            }
            if Instant::now() > deadline {
                panic!(
                    "the process did not end\nstdout:\n{}\nstderr:\n{}",
                    self.stdout(),
                    self.stderr()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Kills, when dropped, every worker process group a run of this rig
/// recorded that is still alive: workers outlive a run killed outright, and
/// a stepped stub would wait two minutes for a release that never comes.
struct Reaper<'a>(&'a Rig);

impl Drop for Reaper<'_> {
    fn drop(&mut self) {
        let logs = self.0.root.join(".vitrify/logs");
        let records = std::fs::read_dir(logs)
            .map(|d| d.flatten().map(|e| e.path().join("run.json")).collect())
            .unwrap_or_else(|_| Vec::new());
        for path in records {
            let Some(record) = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            for w in record["workers"].as_array().into_iter().flatten() {
                if let Some(pid) = w["pid"].as_u64()
                    && group_alive(pid)
                {
                    kill_group(pid);
                }
            }
        }
    }
}

/// Poll until `cond` holds (at most 60 s).
fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn group_alive(pid: u64) -> bool {
    Command::new("/bin/kill")
        .args(["-0", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn kill_group(pid: u64) {
    let _ = Command::new("/bin/kill")
        .args(["-9", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

/// The worker `run.json` shows running for `unit`, if any: (pid, holder).
fn running_worker(run_json: &Value, unit: &str) -> Option<(u64, String)> {
    run_json["workers"].as_array()?.iter().find_map(|w| {
        (w["unit"] == unit && w["outcome"] == "running").then(|| {
            (
                w["pid"].as_u64().unwrap(),
                w["holder"].as_str().unwrap().to_string(),
            )
        })
    })
}

/// The stub's pause points, in the order it reaches them.
const POINTS: [&str; 3] = ["start", "stamp", "report"];

/// Has the stub announced `<unit>-c1` at `point` in the steps directory?
fn at(steps: &Path, unit: &str, point: &str) -> bool {
    steps.join(format!("{unit}-c1.{point}")).exists()
}

/// Let `<unit>-c1` past `point` (or, created in advance, never stop there).
fn release(steps: &Path, unit: &str, point: &str) {
    std::fs::create_dir_all(steps).unwrap();
    std::fs::write(steps.join(format!("{unit}-c1.{point}.go")), "go\n").unwrap();
}

/// The snapshot's worker object for attempt c1 of `unit`, or `Null`.
fn worker<'a>(snap: &'a Value, unit: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    snap["workers"]
        .as_array()
        .and_then(|ws| ws.iter().find(|w| w["unit"] == unit && w["claim_seq"] == 1))
        .unwrap_or(&NULL)
}

/// The first line of the human view that starts with `unit`: its row in the
/// table of units in progress, which comes before the Finished rows.
fn unit_line<'a>(text: &'a str, unit: &str) -> Option<&'a str> {
    text.lines()
        .find(|l| l.split_whitespace().next() == Some(unit))
}

/// Lines of `text` after the line `marker`.
fn after<'a>(text: &'a str, marker: &str) -> Vec<&'a str> {
    text.lines()
        .skip_while(|l| l.trim() != marker)
        .skip(1)
        .collect()
}

/// Lines counted as the status verb counts an untracked file's: its `\n`s,
/// plus one for a last line without one.
fn line_count(text: &str) -> u64 {
    let n = text.matches('\n').count() as u64;
    n + u64::from(!text.is_empty() && !text.ends_with('\n'))
}

// ---------------------------------------------------------------- one-shot

#[test]
fn no_run_recorded_is_not_found() {
    let r = rig();
    for args in [&[][..], &["--json"][..]] {
        let (code, stdout, stderr) = r.status(args);
        assert_eq!(code, 4, "{args:?}: {stdout}{stderr}");
        assert!(
            stderr.contains(
                "error: no run recorded in this repository → start and watch one: vitrify brief run"
            ),
            "{args:?}: {stderr}"
        );
        assert!(stdout.is_empty(), "{args:?}: {stdout}");
    }
    let (code, stdout, stderr) = r.status(&["--job", "run-nope"]);
    assert_eq!(code, 4, "{stdout}{stderr}");
    // a job ID, never a path out of the logs directory
    let (code, stdout, stderr) = r.status(&["--job", "../x"]);
    assert_eq!(code, 2, "{stdout}{stderr}");
    assert!(stderr.contains("error: --job takes a job ID"), "{stderr}");
}

#[test]
fn usage_errors_exit_2() {
    let r = rig();
    for args in [
        &["run", "status", "--since-start"][..],
        &["run", "status", "--cap", "2"][..],
        &["run", "--cap", "2", "status"][..],
    ] {
        let (code, stdout, stderr) = r.run(args, &[]);
        assert_eq!(code, 2, "{args:?}: {stdout}{stderr}");
    }
}

#[test]
fn a_live_worker_is_seen_through_each_stage() {
    let r = rig();
    let _reap = Reaper(&r);
    let steps = r.scratch("steps");
    // only U-001 stops; U-002 runs straight through once it starts
    for point in POINTS {
        release(&steps, "U-002", point);
    }
    let steps_env = steps.to_str().unwrap();
    let bg = r.start(&["run", "--cap", "1"], &[("STUB_STEPS", steps_env)]);

    wait_for("U-001 to pause at start", || at(&steps, "U-001", "start"));
    let snap = r.status_json(&[]);
    let w = worker(&snap, "U-001");
    assert_eq!(snap["state"], "alive", "{snap:#}");
    assert_eq!(w["stage"], "reading", "{snap:#}");
    assert_eq!(w["files_changed"], 0, "{snap:#}");
    assert_eq!(w["alive"], true, "{snap:#}");
    let (code, stdout, stderr) = r.status(&[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let line = unit_line(&stdout, "U-001").unwrap_or_else(|| panic!("{stdout}"));
    assert!(line.contains("c1"), "{line}");
    assert!(line.contains("reading"), "{line}");
    assert!(line.contains("no changes"), "{line}");

    // the stub said something, then nothing: quiet, not an empty log
    let mut last = String::new();
    wait_for("U-001 to be flagged quiet", || {
        last = r.status(&["--quiet-after", "1"]).1;
        last.contains("! U-001 c1: no output for")
    });
    assert!(!last.contains("no output at all"), "{last}");

    release(&steps, "U-001", "start");
    wait_for("U-001 to pause at stamp", || at(&steps, "U-001", "stamp"));
    let snap = r.status_json(&[]);
    let w = worker(&snap, "U-001");
    let artifact =
        std::fs::read_to_string(repo_root().join("fixtures/seeded/run-mini/solution/src/one.txt"))
            .unwrap();
    assert_eq!(w["stage"], "editing", "{snap:#}");
    assert_eq!(w["files_changed"], 1, "{snap:#}");
    assert_eq!(w["carried_over"], 0, "{snap:#}");
    assert_eq!(w["insertions"], line_count(&artifact), "{snap:#}");
    let stdout = r.status(&[]).1;
    let line = unit_line(&stdout, "U-001").unwrap_or_else(|| panic!("{stdout}"));
    assert!(line.contains("editing"), "{line}");
    assert!(line.contains("1 file +"), "{line}");

    release(&steps, "U-001", "stamp");
    wait_for("U-001 to pause at report", || at(&steps, "U-001", "report"));
    let snap = r.status_json(&[]);
    assert_eq!(worker(&snap, "U-001")["stage"], "stamped", "{snap:#}");
    assert_eq!(worker(&snap, "U-001")["stamped"], true, "{snap:#}");
    let stdout = r.status(&[]).1;
    let line = unit_line(&stdout, "U-001").unwrap_or_else(|| panic!("{stdout}"));
    assert!(line.contains("stamped"), "{line}");

    release(&steps, "U-001", "report");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let (code, stdout, stderr) = r.status(&[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let header = stdout.lines().next().unwrap_or_default();
    assert!(header.contains("ended: complete"), "{stdout}");
    assert!(stdout.contains("No unit in progress."), "{stdout}");
    let finished = after(&stdout, "Finished");
    assert!(
        finished.iter().any(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            t.starts_with(&["U-001", "c1", "verified"])
        }),
        "{stdout}"
    );
    let snap = r.status_json(&[]);
    assert_eq!(snap["state"], "ended", "{snap:#}");
    assert_eq!(snap["terminated"]["state"], "complete", "{snap:#}");
    assert_eq!(worker(&snap, "U-001")["outcome"], "verified", "{snap:#}");
}

#[test]
fn a_run_that_died_is_shown_with_how_to_stop_its_worker() {
    let r = rig();
    let _reap = Reaper(&r);
    let bg = r.start(&["run", "--cap", "2"], &[("STUB_HANG_UNIT", "U-001")]);
    wait_for("U-001's worker to start", || {
        running_worker(&r.run_json(), "U-001").is_some()
    });
    let (pid, _) = running_worker(&r.run_json(), "U-001").unwrap();
    // killed outright: it records no end, and its worker outlives it
    bg.signal("-KILL");
    let (status, _, _) = bg.finish();
    assert_eq!(status.code(), None);

    let snap = r.status_json(&[]);
    assert_eq!(snap["state"], "died", "{snap:#}");
    let (code, stdout, stderr) = r.status(&[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains(&format!("stop it with: kill -9 -- -{pid}")),
        "{stdout}"
    );
    kill_group(pid);
}

#[test]
fn the_gate_s_step_and_command_are_shown_while_it_runs() {
    let r = held_rig();
    let _reap = Reaper(&r);
    let hold = r.scratch("hold");
    std::fs::write(&hold, "hold\n").unwrap();
    let bg = r.start(
        &["run", "--cap", "2"],
        &[("STATUS_HOLD", hold.to_str().unwrap())],
    );

    let mut snap = Value::Null;
    wait_for("U-001's row to hold the gate", || {
        snap = r.status_json(&[]);
        snap["gate"]["step"] == "rows" && worker(&snap, "U-001")["stage"] == "gate"
    });
    assert!(
        snap["gate"]["batch"]
            .as_array()
            .is_some_and(|b| b.iter().any(|u| u == "U-001")),
        "{snap:#}"
    );
    // U-002's row may still be the oldest for a moment: wait for U-001's
    let mut stdout = String::new();
    wait_for("U-001's line to name its row", || {
        stdout = r.status(&[]).1;
        unit_line(&stdout, "U-001")
            .is_some_and(|l| l.contains("rows") && l.contains("test -f src/one.txt"))
    });

    std::fs::remove_file(&hold).unwrap();
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ------------------------------------------------------------------ follow

/// What a stepped follow waits to see, in order.
#[derive(Debug, Clone, Copy)]
enum Seen {
    Editing,
    Stamped,
    Integrating,
    GateRows,
    Verified,
    Ended,
}

/// Has the human follower printed `what`? Only lines after the one-shot
/// view count.
fn seen_human(stdout: &str, what: Seen) -> bool {
    // the marker names the job, so it is matched by its start
    after_prefix(stdout, "--- following").iter().any(|l| {
        let t: Vec<&str> = l.split_whitespace().collect();
        let unit = t.get(1..3) == Some(&["U-001", "c1"][..]);
        match what {
            Seen::Editing => unit && l.contains("reading → editing"),
            Seen::Stamped => unit && l.contains("editing → stamped"),
            Seen::Integrating => unit && l.contains("reported built"),
            Seen::GateRows => {
                t.get(1) == Some(&"gate") && l.contains("rows") && l.contains("U-001")
            }
            Seen::Verified => unit && t.get(3) == Some(&"verified"),
            Seen::Ended => t.get(1) == Some(&"run") && l.contains("ended: complete"),
        }
    })
}

/// The JSON follower's lines, each parsed (a line still being written is
/// left out).
fn events(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Has the JSON follower printed `what`?
fn seen_json(stdout: &str, what: Seen) -> bool {
    events(stdout).iter().any(|e| {
        let unit = e["unit"] == "U-001" && e["claim_seq"] == 1;
        match what {
            Seen::Editing => {
                unit && e["event"] == "stage" && e["from"] == "reading" && e["to"] == "editing"
            }
            Seen::Stamped => {
                unit && e["event"] == "stage" && e["from"] == "editing" && e["to"] == "stamped"
            }
            Seen::Integrating => unit && e["event"] == "integrating",
            Seen::GateRows => {
                e["event"] == "gate"
                    && e["step"] == "rows"
                    && e["batch"]
                        .as_array()
                        .is_some_and(|b| b.iter().any(|u| u == "U-001"))
            }
            Seen::Verified => unit && e["event"] == "outcome" && e["outcome"] == "verified",
            Seen::Ended => e["event"] == "run-ended" && e["state"] == "complete",
        }
    })
}

/// Follow a held run while stepping U-001 through it, one release at a
/// time, each only once the follower has printed what came before. Returns
/// the follower's exit status and stdout.
fn stepped_follow(json: bool) -> (std::process::ExitStatus, String) {
    let r = held_rig();
    let _reap = Reaper(&r);
    let steps = r.scratch("steps");
    let hold = r.scratch("hold");
    std::fs::write(&hold, "hold\n").unwrap();
    for point in POINTS {
        release(&steps, "U-002", point);
    }
    let run = r.start(
        &["run", "--cap", "2"],
        &[
            ("STUB_STEPS", steps.to_str().unwrap()),
            ("STATUS_HOLD", hold.to_str().unwrap()),
        ],
    );
    wait_for("U-001 to pause at start", || at(&steps, "U-001", "start"));

    let mut args = vec!["run", "status", "--follow", "--interval", "1"];
    if json {
        args.push("--json");
    }
    let follower = r.start(&args, &[]);
    let seen = |what: Seen| {
        let stdout = follower.stdout();
        if json {
            seen_json(&stdout, what)
        } else {
            seen_human(&stdout, what)
        }
    };
    wait_for("the follower to attach", || {
        let stdout = follower.stdout();
        if json {
            !events(&stdout).is_empty()
        } else {
            stdout.contains("--- following")
        }
    });
    for (point, what) in [
        ("start", Seen::Editing),
        ("stamp", Seen::Stamped),
        ("report", Seen::Integrating),
    ] {
        release(&steps, "U-001", point);
        wait_for(&format!("the follower to show {what:?}"), || seen(what));
    }
    wait_for("the follower to show the held rows", || {
        seen(Seen::GateRows)
    });
    std::fs::remove_file(&hold).unwrap();
    wait_for("the follower to show U-001 verified", || {
        seen(Seen::Verified)
    });
    let (status, stdout, stderr) = follower.finish();
    assert!(
        if json {
            seen_json(&stdout, Seen::Ended)
        } else {
            seen_human(&stdout, Seen::Ended)
        },
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let (run_status, run_out, run_err) = run.finish();
    assert_eq!(
        run_status.code(),
        Some(0),
        "stdout:\n{run_out}\nstderr:\n{run_err}"
    );
    (status, stdout)
}

#[test]
fn follow_prints_each_change_until_the_run_ends() {
    let (status, stdout) = stepped_follow(false);
    assert_eq!(status.code(), Some(0), "{stdout}");
    let lines = after_prefix(&stdout, "--- following");
    let verified = lines
        .iter()
        .position(|l| l.contains("U-001") && l.contains("verified"))
        .unwrap_or_else(|| panic!("{stdout}"));
    let ended = lines
        .iter()
        .position(|l| l.contains("ended: complete"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(verified < ended, "{stdout}");
    assert_eq!(ended, lines.len() - 1, "the end is the last line: {stdout}");
}

#[test]
fn follow_json_prints_one_event_per_line() {
    let (status, stdout) = stepped_follow(true);
    assert_eq!(status.code(), Some(0), "{stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    let events: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    assert_eq!(events[0]["event"], "snapshot", "{stdout}");
    assert_eq!(events[0]["replay"], false, "{stdout}");
    let last = events.last().unwrap();
    assert_eq!(last["event"], "run-ended", "{stdout}");
    assert_eq!(last["state"], "complete", "{stdout}");
    assert!(events.iter().all(|e| e["replay"] == false), "{stdout}");
}

#[test]
fn follow_exits_2_when_the_run_ends_blocked() {
    let r = rig();
    let _reap = Reaper(&r);
    let steps = r.scratch("steps");
    // U-001 blocks at once; U-002 keeps the run alive until the follower
    // has attached
    let run = r.start(
        &["run", "--cap", "2"],
        &[
            ("STUB_STEPS", steps.to_str().unwrap()),
            ("STUB_BLOCK_UNIT", "U-001"),
        ],
    );
    wait_for("U-002 to pause at start", || at(&steps, "U-002", "start"));
    let follower = r.start(&["run", "status", "--follow", "--interval", "1"], &[]);
    wait_for("the follower to attach", || {
        follower.stdout().contains("--- following")
    });
    for point in POINTS {
        release(&steps, "U-002", point);
    }
    let (status, stdout, stderr) = follower.finish();
    assert_eq!(
        status.code(),
        Some(2),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("ended: blocked"), "{stdout}");
    let (run_status, run_out, run_err) = run.finish();
    assert_eq!(
        run_status.code(),
        Some(2),
        "stdout:\n{run_out}\nstderr:\n{run_err}"
    );
}

/// Lines of `text` after the first line that starts with `prefix`.
fn after_prefix<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    text.lines()
        .skip_while(|l| !l.starts_with(prefix))
        .skip(1)
        .collect()
}

// ------------------------------------------------------------------ replay

#[test]
fn since_start_replays_a_finished_run_then_shows_it() {
    let r = rig();
    let _reap = Reaper(&r);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let replay = || r.status(&["--follow", "--since-start", "--json", "--job", &job]);
    let dispatched = |evs: &[Value], unit: &str| {
        evs.iter()
            .any(|e| e["event"] == "dispatched" && e["unit"] == unit)
    };
    let outcome = |evs: &[Value], unit: &str| {
        evs.iter()
            .any(|e| e["event"] == "outcome" && e["unit"] == unit && e["outcome"] == "verified")
    };

    // from the event log
    let (code, stdout, stderr) = replay();
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let evs: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    let snapshot = evs
        .iter()
        .position(|e| e["event"] == "snapshot")
        .unwrap_or_else(|| panic!("{stdout}"));
    let replayed = &evs[..snapshot];
    assert!(!replayed.is_empty(), "{stdout}");
    assert!(replayed.iter().all(|e| e["replay"] == true), "{stdout}");
    assert_eq!(evs[snapshot]["replay"], false, "{stdout}");
    for unit in ["U-001", "U-002"] {
        assert!(dispatched(replayed, unit), "{unit}: {stdout}");
        assert!(outcome(replayed, unit), "{unit}: {stdout}");
    }

    // without it: nothing to replay, and the view as before
    std::fs::remove_file(r.root.join(".vitrify/logs").join(&job).join("events.jsonl")).unwrap();
    let (code, stdout, stderr) = replay();
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let evs: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    assert!(evs.iter().all(|e| e["replay"] == false), "{stdout}");
    assert_eq!(evs[0]["event"], "snapshot", "{stdout}");
    assert!(!stderr.contains("run.json"), "{stderr}");
}

// ------------------------------------------------------------------ report

#[test]
fn status_prints_the_report_of_a_blocked_run() {
    let r = rig();
    let _reap = Reaper(&r);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_BLOCK_UNIT", "U-001")]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();

    // the header names the command that watches this run, after Parameters:
    let lines: Vec<&str> = stdout.lines().collect();
    let params = lines
        .iter()
        .position(|l| l.starts_with("Parameters: "))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert_eq!(
        lines[params + 1],
        format!(
            "Watch: vitrify run status --job {job} [--follow] · once it ends, the same command prints the report · procedure: vitrify brief run"
        )
    );

    // the report as the run printed it: from `Blocked.` to the journal tail
    let printed: Vec<String> = stdout
        .lines()
        .skip_while(|l| *l != "Blocked.")
        .map(str::to_string)
        .collect();
    assert!(printed.len() > 3, "{stdout}");
    assert!(printed[1].starts_with("  Progress: "), "{stdout}");
    assert!(
        printed
            .iter()
            .any(|l| l.starts_with("  U-001: ") && l.contains("Q-999")),
        "{stdout}"
    );
    assert!(printed.iter().any(|l| l == "  journal tail:"), "{stdout}");

    // kept: report.txt and the record
    let kept = std::fs::read_to_string(r.root.join(".vitrify/logs").join(&job).join("report.txt"))
        .unwrap();
    assert_eq!(kept, format!("{}\n", printed.join("\n")));
    let record = r.run_json();
    let want: Vec<Value> = printed.iter().map(|l| Value::from(l.as_str())).collect();
    assert_eq!(record["report"], Value::Array(want.clone()));
    assert_eq!(record["terminated"]["state"], "blocked");
    assert_eq!(
        record["terminated"]["detail"],
        format!("{} — the report names each blocker", printed[1].trim()),
        "{record}"
    );

    // `run status` prints it under Report:
    let (code, text, stderr) = r.status(&[]);
    assert_eq!(code, 0, "{text}{stderr}");
    let shown: Vec<&str> = after(&text, "Report:");
    assert_eq!(
        shown[..printed.len()],
        printed.iter().map(String::as_str).collect::<Vec<_>>()[..],
        "{text}"
    );

    // and as `report`, right after `terminated`
    let (code, json_text, stderr) = r.status(&["--json"]);
    assert_eq!(code, 0, "{json_text}{stderr}");
    let snap: Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(snap["report"], Value::Array(want));
    let terminated_at = json_text.find("\n  \"terminated\"").unwrap();
    let report_at = json_text.find("\n  \"report\"").unwrap();
    let progress_at = json_text.find("\n  \"progress\"").unwrap();
    assert!(
        terminated_at < report_at && report_at < progress_at,
        "{json_text}"
    );
}

#[test]
fn a_live_run_has_no_report_yet() {
    let r = rig();
    let _reap = Reaper(&r);
    let steps = r.scratch("steps");
    let run = r.start(
        &["run", "--cap", "1"],
        &[("STUB_STEPS", steps.to_str().unwrap())],
    );
    wait_for("U-001 to pause at start", || at(&steps, "U-001", "start"));
    assert_eq!(r.run_json()["report"], Value::Null);
    let snap = r.status_json(&[]);
    assert_eq!(snap["report"], Value::Null, "{snap}");
    let (_, text, _) = r.status(&[]);
    assert!(!text.contains("Report:"), "{text}");
    drop(run);
}

// --------------------------------------------------------------- dry run

#[test]
fn a_dry_run_ends_with_the_command_that_starts_the_run() {
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--dry-run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        stdout.lines().last(),
        Some("Start: vitrify run --cap 2"),
        "{stdout}"
    );
    // a dry run starts nothing to watch
    assert!(!stdout.contains("Watch: "), "{stdout}");

    let (code, stdout, stderr) = r.run(
        &[
            "run",
            "--sdd",
            "specs/mini",
            "--wait",
            "--dry-run",
            "--gate-jobs",
            "1",
        ],
        &[],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        stdout.lines().last(),
        Some("Start: vitrify run --gate-jobs 1 --wait --sdd specs/mini"),
        "{stdout}"
    );
}

#[test]
fn each_preflight_failure_names_its_fix() {
    let r = rig();
    // an uncommitted spec change, and no working harness recorded
    let sdd = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd).unwrap();
    std::fs::write(&sdd, format!("{text}\n")).unwrap();
    std::fs::write(
        r.home.join("config.toml"),
        vitrify_core::content::CONFIG.body,
    )
    .unwrap();
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.lines().any(|l| l
            .starts_with("preflight failed: SDD not committed at the mainline head")
            && l.ends_with(" → commit or stash the changes under specs/mini")),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "preflight failed: ~/.vitrify/config.toml records no working harness → vitrify probe"
        ),
        "{stderr}"
    );
    for line in stderr
        .lines()
        .filter(|l| l.starts_with("preflight failed: "))
    {
        assert!(line.contains(" → "), "{line}");
    }

    // no SDD at all
    let empty = r.scratch("empty");
    std::fs::create_dir_all(&empty).unwrap();
    git(&empty, &["init", "-q", "-b", "main"]);
    let out = r
        .command(&["run", "--dry-run"], &[])
        .current_dir(&empty)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find(|l| l.starts_with("preflight failed: no SDD found in repository"))
        .unwrap_or_else(|| panic!("{stderr}"));
    assert!(
        line.contains(" → ") && line.contains("vitrify brief authoring"),
        "{line}"
    );
}

/// `~/.vitrify/config.toml` as `vitrify init` writes it, its probe record
/// naming `worker` as claude's headless spelling.
fn write_config(home: &Path, worker: &Path) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "{}\n[probe]\nvitrify_version = \"{}\"\ngit = {{ present = true, version = \"2\" }}\n\n[probe.claude]\npresent = true\nheadless = [\"{}\"]\n",
            vitrify_core::content::CONFIG.body,
            env!("CARGO_PKG_VERSION"),
            worker.display()
        ),
    )
    .unwrap();
}
