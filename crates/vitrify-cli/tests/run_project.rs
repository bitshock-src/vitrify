//! Project runs: a plain `vitrify run` in a repository of several SDDs
//! builds them all, one lane per SDD, under one scheduler — on the seeded
//! `fixtures/seeded/project-*` repositories (each README states its
//! scenario) and on a synthetic project, with the stub worker.

mod synth;

use std::collections::BTreeSet;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
}

fn stub_bin() -> PathBuf {
    // built once per test process (a stale or relinking binary otherwise)
    static STUB: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    STUB.get_or_init(|| {
        let stub = PathBuf::from(vitrify_bin())
            .parent()
            .unwrap()
            .join("stub-worker");
        assert!(
            Command::new(env!("CARGO"))
                .args(["build", "-p", "stub-worker", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        stub
    })
    .clone()
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let (p, t) = (entry.path(), dst.join(entry.file_name()));
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

/// `~/.vitrify/config.toml` with a probe record naming the stub worker.
fn write_config(home: &Path) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "{}\n[probe]\nvitrify_version = \"{}\"\ngit = {{ present = true, version = \"2\" }}\n\n[probe.claude]\npresent = true\nheadless = [\"{}\"]\n",
            vitrify_core::content::CONFIG.body,
            env!("CARGO_PKG_VERSION"),
            stub_bin().display()
        ),
    )
    .unwrap();
}

struct Rig {
    tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    /// Where the stub worker reads its canned files and maps from.
    support: PathBuf,
}

/// The project of `fixtures/seeded/<fixture>`, committed on main.
fn rig_of(fixture: &str) -> Rig {
    let r = empty_rig();
    copy_dir(
        &repo_root()
            .join("fixtures/seeded")
            .join(fixture)
            .join("project"),
        &r.root,
    );
    for entry in std::fs::read_dir(repo_root().join("fixtures/seeded").join(fixture))
        .unwrap()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "project" || name == "README.md" {
            continue;
        }
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &r.support.join(&name));
        } else {
            std::fs::copy(entry.path(), r.support.join(&name)).unwrap();
        }
    }
    commit_seed(&r.root);
    r
}

fn empty_rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, home, support) = (base.join("repo"), base.join("vhome"), base.join("support"));
    for d in [&root, &support, &base.join("t")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_config(&home);
    Rig {
        tmp,
        root,
        home,
        support,
    }
}

fn commit_seed(root: &Path) {
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@test"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "seed"]);
}

impl Rig {
    fn command(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
        let mut c = Command::new(vitrify_bin());
        c.args(args)
            .current_dir(&self.root)
            .env("VITRIFY_HOME", &self.home)
            .env("STUB_REF", self.support.join("solution"))
            .env("STUB_MAP", self.support.join("stub-map-{sdd}.toml"))
            .env("VITRIFY_BIN", vitrify_bin())
            .env("STUB_MODE", "build")
            .env("TMPDIR", self.tmp.path().join("t"))
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

    fn start(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Background {
        let out = self.tmp.path().join(format!(
            "run-{}.out",
            std::fs::read_dir(self.tmp.path()).unwrap().count()
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

    fn run_json(&self) -> serde_json::Value {
        std::fs::read_to_string(self.root.join(".vitrify/run.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    /// The latest job's logs (`.vitrify/logs/` itself before any job is
    /// recorded).
    fn job_logs(&self) -> PathBuf {
        let job = self.run_json()["job_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.root.join(".vitrify/logs").join(job)
    }

    /// The job's `events.jsonl`, one value per line.
    fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.job_logs().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn on_main(&self, path: &str) -> String {
        git(&self.root, &["show", &format!("main:{path}")])
    }

    fn edit(&self, rel: &str, from: &str, to: &str) {
        let path = self.root.join(rel);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{rel} has no `{from}`");
        std::fs::write(&path, text.replace(from, to)).unwrap();
    }
}

struct Background {
    child: Child,
    out: PathBuf,
    err: PathBuf,
}

impl Background {
    fn signal(&self, sig: &str) {
        let ok = Command::new("/bin/kill")
            .args([sig, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {sig}");
    }

    fn finish(mut self) -> (std::process::ExitStatus, String, String) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (
                    status,
                    std::fs::read_to_string(&self.out).unwrap_or_default(),
                    std::fs::read_to_string(&self.err).unwrap_or_default(),
                );
            }
            assert!(Instant::now() < deadline, "the run did not end");
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

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ------------------------------------------------------------- pipeline

/// `STUB_LOG` lines: (event, sdd, unit, ms).
fn stub_log(path: &Path) -> Vec<(String, String, String, u128)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f.first()?.to_string(),
                f.get(1)?.to_string(),
                f.get(2)?.to_string(),
                f.get(4)?.parse().ok()?,
            ))
        })
        .collect()
}

/// The longest chain of worker sessions each starting after the one before
/// it ended: the makespan in unit-times, when every unit takes as long.
fn sequential_rounds(log: &[(String, String, String, u128)]) -> usize {
    let mut sessions: Vec<(u128, u128)> = Vec::new();
    for (event, sdd, unit, at) in log {
        if event == "start"
            && let Some((_, _, _, end)) = log
                .iter()
                .find(|(e, s, u, t)| e == "end" && s == sdd && u == unit && t >= at)
        {
            sessions.push((*at, *end));
        }
    }
    sessions.sort();
    let mut best: Vec<usize> = vec![1; sessions.len()];
    for i in 0..sessions.len() {
        for j in 0..i {
            if sessions[j].1 <= sessions[i].0 {
                best[i] = best[i].max(best[j] + 1);
            }
        }
    }
    best.into_iter().max().unwrap_or(0)
}

/// The `[slug] dispatch U-###` lines, as `slug U-###`, in order.
fn dispatched(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| {
            let (lane, rest) = l.strip_prefix('[')?.split_once("] dispatch ")?;
            Some(format!("{lane} {}", rest.split_whitespace().next()?))
        })
        .collect()
}

fn position(stdout: &str, line: &str) -> usize {
    stdout
        .lines()
        .position(|l| l.starts_with(line))
        .unwrap_or_else(|| panic!("no `{line}` in:\n{stdout}"))
}

/// Every `pass` the run recorded: never more workers than the cap, and a
/// slot idle only when nothing ready waited for one.
fn assert_passes(r: &Rig, cap: i64) {
    let passes: Vec<serde_json::Value> = r
        .events()
        .into_iter()
        .filter(|e| e["event"] == "pass")
        .collect();
    assert!(!passes.is_empty());
    for p in &passes {
        assert!(p["working"].as_i64().unwrap() <= cap, "{p}");
        assert!(p["idle"] == 0 || p["waiting"] == 0, "{p}");
    }
}

#[test]
fn a_plain_run_builds_every_sdd_on_the_critical_path_first() {
    // core U-001 → core U-002 → api U-002 → api U-003 is the critical path;
    // two workers finish in its four unit-times
    let r = rig_of("project-pipeline");
    let log = r.tmp.path().join("stub.log");
    let log_s = log.display().to_string();
    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[("STUB_SLEEP_MS", "3000"), ("STUB_LOG", &log_s)],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let header: Vec<&str> = stdout.lines().take(5).collect();
    assert!(
        header[0].starts_with("Project build job — 2 SDDs: "),
        "{stdout}"
    );
    assert_eq!(
        header[1],
        "Plan: regenerate 6 units · adopt 0 built attempts · re-check 0 units"
    );
    assert!(header[2].starts_with("Critical path: ") && header[2].ends_with("(4 units)"));
    assert!(
        header[3].starts_with("Capacity: workers 2 · gate jobs "),
        "{stdout}"
    );
    let job = header[0]
        .rsplit_once("  [")
        .unwrap()
        .1
        .trim_end_matches(']');
    assert_eq!(
        header[4],
        format!("Watch: vitrify run status --job {job} --follow")
    );
    assert!(
        stdout.contains(
            "Complete.\n  api      Complete     3/3 Verified · built 3 · §5 Test passing\n  core     Complete     3/3 Verified · built 3 · §5 Test passing"
        ),
        "{stdout}"
    );
    // four unit-times on an idle machine; one more is the tolerance for a
    // loaded one, where dispatch can lag a unit's whole duration
    let rounds = sequential_rounds(&stub_log(&log));
    assert!((4..=5).contains(&rounds), "{rounds}: {:?}", stub_log(&log));

    // nothing is dispatched before what it depends on has landed
    let after = |line: &str, deps: &[&str]| {
        for dep in deps {
            assert!(
                position(&stdout, dep) < position(&stdout, line),
                "{line} before {dep}:\n{stdout}"
            );
        }
    };
    after("[api] dispatch U-001", &["[core] integrated U-001"]);
    after(
        "[api] dispatch U-002",
        &["[core] integrated U-002", "[api] integrated U-001"],
    );
    after("[core] dispatch U-002", &["[core] integrated U-001"]);
    assert_passes(&r, 2);

    // project.json: the plan, the lanes as they ended, the path as it ran
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.job_logs().join("project.json")).unwrap())
            .unwrap();
    let lanes = record["lanes"].as_array().unwrap();
    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0]["slug"], "api");
    assert_eq!(lanes[0]["state"], "complete");
    assert_eq!(lanes[0]["root"], "specs/api");
    // the counts as the run ended, not as it planned
    for lane in lanes {
        assert_eq!(
            (&lane["units"], &lane["verified"]),
            (&3.into(), &3.into()),
            "{record}"
        );
    }
    let records = r.root.join(lanes[0]["records"].as_str().unwrap());
    assert!(records.join("gate.json").is_file(), "{record}");
    assert_eq!(record["critical_path_ran"].as_array().unwrap().len(), 4);
    assert!(record["ended_at"].is_string());
}

#[test]
fn one_worker_dispatches_in_a_fixed_order() {
    let r = rig_of("project-pipeline");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "1"], &[("STUB_SLEEP_MS", "1500")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        dispatched(&stdout),
        [
            "core U-001",
            "api U-001",
            "core U-002",
            "api U-002",
            "core U-003",
            "api U-003"
        ],
        "{stdout}"
    );
}

#[test]
fn a_dry_run_prints_the_same_plan_for_the_same_repository() {
    let r = rig_of("project-pipeline");
    let head = git(&r.root, &["rev-parse", "main"]);
    let (code, first, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "{first}{stderr}");
    let (_, second, _) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(first, second);
    assert!(
        first.starts_with("Project build job — 2 SDDs: 1 to build · 1 idle  [dry-run]\n"),
        "{first}"
    );
    assert!(first.contains("\nLanes:\n"), "{first}");
    assert!(
        first.ends_with("dry-run: preflight green; stopping before dispatch\nStart: vitrify run\n"),
        "{first}"
    );
    assert_eq!(
        git(&r.root, &["rev-parse", "main"]),
        head,
        "a dry run publishes nothing"
    );
}

#[test]
fn a_unit_reading_a_pin_as_an_interface_builds_beside_its_producer() {
    let r = rig_of("project-interface-only");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_SLEEP_MS", "1000")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let landed = position(&stdout, "[core] integrated U-001");
    assert!(
        position(&stdout, "[api] dispatch U-001") < landed,
        "{stdout}"
    );
    assert!(stdout.contains("Complete.\n"), "{stdout}");
}

#[test]
fn pins_both_ways_with_an_acyclic_unit_graph_complete() {
    let r = rig_of("project-mutual");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete.\n  left "), "{stdout}");
}

#[test]
fn a_cycle_is_refused_with_every_closing_path() {
    let r = rig_of("project-cycle");
    let (code, stdout, stderr) = r.run(&["run"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    let cycles = stderr
        .lines()
        .filter(|l| l.starts_with("preflight failed: [L12] dependency cycle: "))
        .count();
    assert_eq!(cycles, 2, "{stderr}");
    assert!(
        stderr.contains("alpha U-001 → beta U-001 → alpha U-001"),
        "{stderr}"
    );
    assert!(stderr.contains("behind a cycle: alpha U-002"), "{stderr}");
}

#[test]
fn a_schema_2_0_sdd_that_consumes_pins_refuses_a_project_run_but_not_its_own() {
    let r = rig_of("project-schema20");
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 4, "{stderr}");
    assert!(stderr.contains("preflight failed: SDD old ("), "{stderr}");
    // one SDD at a time still works: old reads what base builds
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/base"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/old"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("SDD build job — old @"), "{stdout}");
}

#[test]
fn upstream_adds_the_sdds_a_lane_depends_on() {
    let r = rig_of("project-pipeline");
    // core depends on nothing: one lane, built as ever
    let (code, stdout, _) = r.run(
        &["run", "--sdd", "specs/core", "--upstream", "--dry-run"],
        &[],
    );
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.starts_with("SDD build job — core @"), "{stdout}");
    // api depends on core: both are lanes
    let (code, stdout, stderr) = r.run(
        &["run", "--sdd", "specs/api", "--upstream", "--cap", "2"],
        &[],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.starts_with("Project build job — 2 SDDs: "),
        "{stdout}"
    );
    assert!(stdout.contains("  core     Complete"), "{stdout}");
    // and it needs --sdd
    let (code, _, stderr) = r.run(&["run", "--upstream"], &[]);
    assert_eq!(code, 2, "{stderr}");
}

#[test]
fn a_blocked_unit_holds_up_only_what_depends_on_it() {
    let r = rig_of("project-flaky");
    let counts = r.tmp.path().join("flaky");
    std::fs::create_dir_all(&counts).unwrap();
    let counts_s = counts.display().to_string();
    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[
            ("FLAKY_DIR", &counts_s),
            ("FLAKY_FAILS", "0"),
            ("STUB_BLOCK_UNIT", "U-002"),
        ],
    );
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("[side] integrated U-001"), "{stdout}");
    assert!(stdout.contains("[flaky] integrated U-001"), "{stdout}");
    assert!(stdout.contains("\nBlocked.\n"), "{stdout}");
    assert!(
        stdout.contains("  flaky    Blocked      U-002 open Q-999"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  side     Complete     1/1 Verified"),
        "{stdout}"
    );
}

#[test]
fn an_interrupted_project_run_resumes_without_redoing_what_landed() {
    let r = rig_of("project-pipeline");
    let bg = r.start(&["run", "--cap", "2"], &[("STUB_SLEEP_MS", "1500")]);
    wait_for("core U-001 to land", || {
        std::fs::read_to_string(&bg.out)
            .unwrap_or_default()
            .contains("[core] integrated U-001")
    });
    bg.signal("-INT");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(2),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stdout.contains("[core] dispatch U-001"), "{stdout}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(
        r.on_main("specs/api/units/U-003-three.md")
            .contains("status: Verified")
    );
}

#[test]
fn a_synthetic_project_of_six_sdds_completes() {
    let r = empty_rig();
    let project = synth::generate(&synth::Spec::new(6, 8, 11));
    project.write(&r.root, &r.support);
    synth::commit_all(&r.root);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "4"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.starts_with("Project build job — 6 SDDs: "),
        "{stdout}"
    );
    assert_eq!(stdout.matches("] integrated ").count(), 48, "{stdout}");
    assert_passes(&r, 4);
}

// ------------------------------------------------ the cascade across SDDs

/// project-cascade built; then core U-001's text changes and the change is
/// marked, so the next run rebuilds core U-001 from `solution`.
fn cascade_rebuilding_core_one(
    solution: &str,
    prepare: &dyn Fn(&Rig),
) -> (Rig, i32, String, String, Vec<String>) {
    let r = rig_of("project-cascade");
    prepare(&r);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    r.edit("specs/core/sdd.md", "version: 1.0.0", "version: 1.1.0");
    r.edit(
        "specs/core/units/U-001-data.md",
        "with the frozen content.",
        "with the frozen content, again.",
    );
    git(
        &r.root,
        &["commit", "-q", "-am", "core CH-001: U-001.R1 again"],
    );
    let (code, out, err) = r.run(&["audit", "--apply-stale", "--sdd", "specs/core"], &[]);
    assert!(
        std::fs::read_to_string(r.root.join("specs/core/units/U-001-data.md"))
            .unwrap()
            .contains("status: Stale"),
        "{code}\n{out}{err}"
    );
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "core CH-001: staleness applied"],
    );
    let before = rechecked(&r);
    let reference = r.support.join(solution).display().to_string();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_REF", &reference)]);
    let mut after = rechecked(&r);
    for b in before {
        if let Some(i) = after.iter().position(|a| *a == b) {
            after.remove(i);
        }
    }
    (r, code, stdout, stderr, after)
}

/// The units the journals record as re-checked, as `slug U-###`, sorted.
/// (The helper above returns those the second run added.)
fn rechecked(r: &Rig) -> Vec<String> {
    let mut out = Vec::new();
    for slug in ["api", "core"] {
        for line in r.on_main(&format!("specs/{slug}/journal.ndjson")).lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v["event"] == "verified" && v["actor"] == "recheck" {
                out.push(format!("{slug} {}", v["unit"].as_str().unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_regenerated_core_unit_rechecks_exactly_its_dependents_in_every_sdd() {
    // rebuilt into other bytes that still satisfy every row
    let (_r, code, stdout, stderr, added) = cascade_rebuilding_core_one("solution-again", &|r| {
        copy_dir(
            &r.support.join("solution"),
            &r.support.join("solution-again"),
        );
        std::fs::write(
            r.support.join("solution-again/core/data.txt"),
            "v1\nagain\n",
        )
        .unwrap();
    });
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(dispatched(&stdout), ["core U-001"], "{stdout}");
    // core U-002 in core's gate; api's units after the merge, in order
    assert_eq!(added, ["api U-001", "api U-002", "core U-002"], "{stdout}");
    // after core's merge, api's two in one batch: U-002 depends on U-001,
    // in the same SDD, and they are checked together
    assert!(stdout.contains("[api] re-check U-001, U-002\n"), "{stdout}");
    assert!(stdout.contains("  api      Complete"), "{stdout}");
}

#[test]
fn a_core_unit_regenerated_into_the_same_bytes_rechecks_nothing() {
    let (_r, code, stdout, stderr, added) = cascade_rebuilding_core_one("solution-regen", &|_| {});
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(dispatched(&stdout), ["core U-001"], "{stdout}");
    assert!(added.is_empty(), "{added:?}\n{stdout}");
    assert!(!stdout.contains("] re-check"), "{stdout}");
    // since the newest recorded check of api: its last verification or
    // suite pass
    let journal = _r.on_main("specs/api/journal.ndjson");
    let since = journal
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|e| e["event"] == "verified" || e["suite"] == "pass")
        .and_then(|e| e["commit"].as_str().map(str::to_string))
        .unwrap();
    assert!(
        stdout.contains(&format!(
            "  api      Up to date   nothing changed since {since}\n"
        )),
        "{since}\n{stdout}"
    );
}

#[test]
fn an_upstream_change_that_breaks_a_downstream_row_rots_it_and_names_the_culprit() {
    let (_r, code, stdout, stderr, added) = cascade_rebuilding_core_one("solution-v2", &|_| {});
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("[core] integrated U-001 (c2)"), "{stdout}");
    assert!(
        stdout.contains("[api] U-001 rotten: row 2 fails on mainline at "),
        "{stdout}"
    );
    assert!(
        stdout.contains("[api] U-001 row 2 broke at ")
            && stdout.contains("(core U-001 integrated) — culprit found"),
        "{stdout}"
    );
    // its dependent is not re-checked while it fails; nothing is rebuilt
    assert!(!stdout.contains("[api] re-check U-002"), "{stdout}");
    assert_eq!(dispatched(&stdout), ["core U-001"], "{stdout}");
    assert!(stdout.contains("\nBlocked.\n"), "{stdout}");
    assert!(
        stdout.contains("  api      Blocked      U-001 rotten: row 2 broke at "),
        "{stdout}"
    );
    assert!(stdout.contains("  core     Complete"), "{stdout}");
    assert!(added.contains(&"core U-002".to_string()), "{stdout}");
}

#[test]
fn a_lane_cap_of_one_builds_one_lane_at_a_time() {
    // interface-only: api U-001 and core U-001 are both ready at once, but
    // with one lane active at a time they do not build side by side
    let r = rig_of("project-interface-only");
    let log = r.tmp.path().join("stub.log");
    let log_s = log.display().to_string();
    let bg = r.start(
        &["run", "--cap", "2"],
        &[
            ("VITRIFY_RUN_LANE_CAP", "1"),
            ("STUB_SLEEP_MS", "800"),
            ("STUB_LOG", &log_s),
        ],
    );
    // the checkouts on disk, watched for the whole run: never two lanes'
    let worktrees = r.home.join("worktrees");
    let mut most = 0;
    let mut checkouts_seen = 0;
    let mut bg = bg;
    while bg.child.try_wait().unwrap().is_none() {
        let lanes_on_disk = std::fs::read_dir(&worktrees)
            .map(|d| {
                d.flatten()
                    .flat_map(|repo| {
                        std::fs::read_dir(repo.path())
                            .into_iter()
                            .flatten()
                            .flatten()
                    })
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        ["mainline-", "base-", "probe-"]
                            .iter()
                            .find_map(|p| name.strip_prefix(p).map(str::to_string))
                    })
                    .collect::<BTreeSet<String>>()
            })
            .unwrap_or_default();
        most = most.max(lanes_on_disk.len());
        checkouts_seen += lanes_on_disk.len();
        std::thread::sleep(Duration::from_millis(20));
    }
    let (status, stdout, stderr) = bg.finish();
    let code = status.code().unwrap_or(-1);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(checkouts_seen > 0, "a checkout was seen");
    assert_eq!(most, 1, "two lanes' checkouts on disk at once");
    assert!(
        stdout.contains("Capacity: workers 2 · gate jobs "),
        "{stdout}"
    );
    assert!(stdout.contains(" · lanes 1\n"), "{stdout}");
    // no two workers of different SDDs ever ran at once
    let log = stub_log(&log);
    let mut sessions: Vec<(String, u128, u128)> = Vec::new();
    for (event, sdd, unit, at) in &log {
        if event == "start"
            && let Some((_, _, _, end)) = log
                .iter()
                .find(|(e, s, u, t)| e == "end" && s == sdd && u == unit && t >= at)
        {
            sessions.push((sdd.clone(), *at, *end));
        }
    }
    for a in &sessions {
        for b in &sessions {
            if a.0 != b.0 {
                assert!(a.2 <= b.1 || b.2 <= a.1, "{a:?} overlaps {b:?}");
            }
        }
    }
}

// ------------------------------------------------------------ throughput

/// The `gate` events of lane `slug` at step `step` whose batch holds `unit`.
fn gate_steps(r: &Rig, slug: &str, step: &str, unit: &str) -> usize {
    r.events()
        .iter()
        .filter(|e| {
            e["event"] == "gate"
                && e["lane"] == slug
                && e["step"] == step
                && e["batch"]
                    .as_array()
                    .is_some_and(|b| b.iter().any(|u| u == unit))
        })
        .count()
}

/// A row that waits (at most 60 s) for `marker`, then checks `file`.
fn waiting_row(marker: &Path, file: &str) -> String {
    format!(
        "`sh -c 'i=0; while [ ! -e {m} ] && [ $i -lt 600 ]; do i=$((i+1)); sleep 0.1; done; test -e {m} && test -f {file}'`",
        m = marker.display()
    )
}

#[test]
fn a_lane_that_loses_the_race_to_a_disjoint_commit_publishes_without_gating_again() {
    // alpha U-001's gate waits until beta U-002's gate has run — by then
    // beta U-001 has landed, and alpha's gate base is behind mainline.
    // Nothing beta changed is anything alpha's gate checked or read.
    let r = rig_of("run-multi");
    let marker = r.tmp.path().join("beta-two-gated");
    r.edit(
        "specs/alpha/units/U-001-one.md",
        "`test -f a/one.txt`",
        &waiting_row(&marker, "a/one.txt"),
    );
    // (each SDD's U-002 depends on its U-001 already)
    r.edit(
        "specs/beta/units/U-002-two.md",
        "`test -f b/two.txt`",
        &format!("`touch {} && test -f b/two.txt`", marker.display()),
    );
    git(&r.root, &["commit", "-q", "-am", "alpha waits for beta"]);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "4"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        position(&stdout, "[beta] integrated U-001")
            < position(&stdout, "[alpha] integrated U-001"),
        "{stdout}"
    );
    // one gate for alpha U-001, its base behind mainline when it published
    assert_eq!(gate_steps(&r, "alpha", "rows", "U-001"), 1, "{stderr}");
    assert!(!stderr.contains("changed in substance"), "{stderr}");
}

/// run-multi's alpha, with U-001's row waiting for a marker; a commit lands
/// on mainline while its gate waits. Returns the run's stdout and stderr.
fn alpha_gate_meets_a_commit(r: &Rig, path: &str) -> (String, String) {
    let marker = r.tmp.path().join("go");
    r.edit(
        "specs/alpha/units/U-001-one.md",
        "`test -f a/one.txt`",
        &waiting_row(&marker, "a/one.txt"),
    );
    git(&r.root, &["commit", "-q", "-am", "alpha U-001 waits"]);
    let map = r.support.join("stub-map-alpha.toml").display().to_string();
    let bg = r.start(&["run", "--sdd", "specs/alpha"], &[("STUB_MAP", &map)]);
    wait_for("alpha U-001's rows", || {
        gate_steps(r, "alpha", "rows", "U-001") > 0
    });
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(r.root.join(dir)).unwrap();
    }
    std::fs::write(r.root.join(path), "changed beside the gate\n").unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "a commit during the gate"]);
    std::fs::write(&marker, "").unwrap();
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    (stdout, stderr)
}

#[test]
fn a_commit_outside_what_the_gate_read_does_not_gate_again() {
    let r = rig_of("run-multi");
    // a file in beta's scope: nothing alpha's gate checked or read
    let (_, stderr) = alpha_gate_meets_a_commit(&r, "b/notes.txt");
    assert_eq!(gate_steps(&r, "alpha", "rows", "U-001"), 1, "{stderr}");
}

#[test]
fn a_commit_to_what_the_gate_read_gates_again() {
    let r = rig_of("run-multi");
    // a path outside every scope: every unit's input
    let (_, stderr) = alpha_gate_meets_a_commit(&r, "NOTES.txt");
    assert_eq!(gate_steps(&r, "alpha", "rows", "U-001"), 2, "{stderr}");
    assert!(
        stderr.contains("changed in substance during the gate"),
        "{stderr}"
    );
}

#[test]
fn one_pass_claims_its_units_in_one_commit() {
    let r = rig_of("run-mini");
    let two = std::fs::read_to_string(r.root.join("specs/mini/units/U-002-two.md")).unwrap();
    std::fs::write(
        r.root.join("specs/mini/units/U-003-three.md"),
        two.replace("U-002", "U-003")
            .replace("two.txt", "three.txt"),
    )
    .unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "U-003"]);
    std::fs::write(r.support.join("solution/src/three.txt"), "three\n").unwrap();
    let map = r.support.join("stub-map-three.toml");
    std::fs::write(
        &map,
        "[units]\nU-001 = [\"src/one.txt\"]\nU-002 = [\"src/two.txt\"]\nU-003 = [\"src/three.txt\"]\n",
    )
    .unwrap();
    let map = map.display().to_string();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "3"], &[("STUB_MAP", &map)]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    let claims: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("sdd: claim"))
        .collect();
    assert_eq!(
        claims,
        ["sdd: claim U-001 (c1), U-002 (c1), U-003 (c1)"],
        "{log}"
    );
    // each unit keeps its own claim event and token
    let journal = r.on_main("specs/mini/journal.ndjson");
    for unit in ["U-001", "U-002", "U-003"] {
        assert!(
            journal.contains(&format!(r#""event":"claim","unit":"{unit}","actor":"#)),
            "{journal}"
        );
    }
    assert_eq!(journal.matches(r#""event":"claim""#).count(), 3);
}

#[test]
fn units_of_several_sdds_are_claimed_in_one_commit() {
    // interface-only: core U-001 and api U-001 are both ready at the start
    let r = rig_of("project-interface-only");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(
        log.lines()
            .any(|l| l == "sdd: claim core U-001 (c1), api U-001 (c1)"),
        "{log}"
    );
}

/// project-flaky, with a third SDD, `extra`, still Draft.
fn flaky_with_a_draft_sdd() -> Rig {
    let r = rig_of("project-flaky");
    copy_dir(&r.root.join("specs/side"), &r.root.join("specs/extra"));
    for f in ["specs/extra/sdd.md", "specs/extra/units/U-001-one.md"] {
        let text = std::fs::read_to_string(r.root.join(f)).unwrap();
        std::fs::write(r.root.join(f), text.replace("side", "extra")).unwrap();
    }
    r.edit("specs/extra/sdd.md", "status: Active", "status: Draft");
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "extra, a draft"]);
    std::fs::create_dir_all(r.support.join("solution/extra")).unwrap();
    std::fs::write(r.support.join("solution/extra/one.txt"), "extra one\n").unwrap();
    std::fs::write(
        r.support.join("stub-map-extra.toml"),
        "[units]\nU-001 = [\"extra/one.txt\"]\n",
    )
    .unwrap();
    r
}

#[test]
fn a_waiting_project_run_takes_in_a_new_sdd_and_resumes_when_the_spec_changes() {
    let r = flaky_with_a_draft_sdd();
    let counts = r.tmp.path().join("flaky");
    std::fs::create_dir_all(&counts).unwrap();
    let counts_s = counts.display().to_string();
    let bg = r.start(
        &["run", "--cap", "2", "--wait"],
        &[
            ("FLAKY_DIR", &counts_s),
            ("FLAKY_FAILS", "0"),
            ("STUB_BLOCK_UNLESS", "U-002:now unblocked"),
        ],
    );
    let out = |bg: &Background| std::fs::read_to_string(&bg.out).unwrap_or_default();
    wait_for("the run to wait", || {
        out(&bg).contains("waiting: the run resumes when the repository changes")
    });
    assert!(
        out(&bg).contains("  flaky    Blocked      U-002 open Q-999"),
        "{}",
        out(&bg)
    );

    // an SDD becomes Active: it joins as a lane, and builds
    r.edit("specs/extra/sdd.md", "status: Draft", "status: Active");
    git(&r.root, &["commit", "-q", "-am", "extra is Active"]);
    wait_for("extra to land", || {
        out(&bg).contains("[extra] integrated U-001")
    });
    assert!(out(&bg).contains("[extra] joins the run"), "{}", out(&bg));

    // the question is answered in the spec: the parked unit builds
    r.edit(
        "specs/flaky/units/U-002-two.md",
        "with the frozen content.",
        "with the frozen content, now unblocked.",
    );
    git(&r.root, &["commit", "-q", "-am", "flaky U-002 answered"]);
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("[flaky] integrated U-002"), "{stdout}");
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
}

#[test]
fn a_cycle_made_mid_run_dispatches_nothing_on_it_and_names_it() {
    let r = rig_of("project-flaky");
    let counts = r.tmp.path().join("flaky");
    std::fs::create_dir_all(&counts).unwrap();
    let counts_s = counts.display().to_string();
    let bg = r.start(
        &["run", "--cap", "2", "--wait"],
        &[
            ("FLAKY_DIR", &counts_s),
            ("FLAKY_FAILS", "0"),
            ("STUB_BLOCK_UNLESS", "U-002:never written"),
        ],
    );
    let out = |bg: &Background| std::fs::read_to_string(&bg.out).unwrap_or_default();
    let err = |bg: &Background| std::fs::read_to_string(&bg.err).unwrap_or_default();
    wait_for("the run to wait", || out(&bg).contains("waiting: "));
    // U-001 now depends on U-002, which depends on U-001
    r.edit(
        "specs/flaky/units/U-001-one.md",
        "depends_on: []",
        "depends_on: [U-002]",
    );
    git(&r.root, &["commit", "-q", "-am", "a cycle"]);
    wait_for("the cycle to be named", || {
        err(&bg).contains("a dependency cycle on mainline: flaky U-001 → flaky U-002 → flaky U-001")
    });
    std::thread::sleep(Duration::from_secs(3));
    bg.signal("-INT");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(2),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(!stdout.contains("[flaky] dispatch U-001 (c2)"), "{stdout}");
    assert_eq!(
        stdout.matches("[flaky] dispatch U-002").count(),
        1,
        "{stdout}"
    );
}

#[test]
fn never_more_than_gate_jobs_commands_run_at_once() {
    // three slow rows in each of two lanes' gates, which run side by side:
    // with two gate jobs, no more than two rows ever run at once
    let r = rig_of("run-multi");
    let running = r.tmp.path().join("running");
    std::fs::create_dir_all(&running).unwrap();
    let seen = r.tmp.path().join("seen");
    for (sdd, file) in [("alpha", "a/one.txt"), ("beta", "b/one.txt")] {
        let rows: Vec<String> = (1..=3)
            .map(|k| {
                format!(
                    "| R1 | `sh -c 'touch {d}/r.$$; set -- {d}/r.*; echo $# >> {s}; sleep 0.6; rm -f {d}/r.$$; test -f {file}; : {sdd}{k}'` | exit 0 |",
                    d = running.display(),
                    s = seen.display()
                )
            })
            .collect();
        r.edit(
            &format!("specs/{sdd}/units/U-001-one.md"),
            &format!("| R1 | `test -f {file}` | exit 0 |"),
            &rows.join("\n"),
        );
    }
    git(&r.root, &["commit", "-q", "-am", "slow rows"]);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "4", "--gate-jobs", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let counts: Vec<usize> = std::fs::read_to_string(&seen)
        .unwrap()
        .lines()
        .map(|l| l.trim().parse().unwrap())
        .collect();
    assert!(counts.len() >= 6, "{counts:?}");
    assert_eq!(counts.iter().max(), Some(&2), "{counts:?}");
}

#[test]
fn recheck_in_a_project_run_rechecks_every_unit_and_suite_once() {
    let r = rig_of("project-pipeline");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let events = |slug: &str, what: &str| {
        r.on_main(&format!("specs/{slug}/journal.ndjson"))
            .matches(what)
            .count()
    };
    let before: Vec<usize> = ["core", "api"]
        .iter()
        .flat_map(|s| {
            [
                events(s, r#""actor":"recheck""#),
                events(s, r#""suite":"pass""#),
            ]
        })
        .collect();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2", "--recheck"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stdout.contains("] dispatch "), "{stdout}");
    // upstream first, each SDD's units in one batch
    assert!(
        position(&stdout, "[core] re-check U-001, U-002, U-003")
            < position(&stdout, "[api] re-check U-001, U-002, U-003"),
        "{stdout}"
    );
    let after: Vec<usize> = ["core", "api"]
        .iter()
        .flat_map(|s| {
            [
                events(s, r#""actor":"recheck""#),
                events(s, r#""suite":"pass""#),
            ]
        })
        .collect();
    // three units and one suite pass recorded in each SDD
    assert_eq!(
        [
            after[0] - before[0],
            after[1] - before[1],
            after[2] - before[2],
            after[3] - before[3]
        ],
        [3, 1, 3, 1],
        "{stdout}"
    );
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
}

#[test]
fn a_dry_run_with_a_lane_blocked_at_preflight_says_so_and_exits_2() {
    let r = rig_of("project-pipeline");
    // an uncommitted change to api's spec: api's own preflight fails
    let path = r.root.join("specs/api/sdd.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\n")).unwrap();
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("\n  api   blocked"), "{stdout}");
    assert!(
        stderr.contains("[api] preflight failed: SDD not committed at the mainline head"),
        "{stderr}"
    );
    assert!(
        stdout.contains("dry-run: preflight failed for api — the run would end Blocked"),
        "{stdout}"
    );
    assert!(!stdout.contains("preflight green"), "{stdout}");
}

#[test]
fn a_rotten_unit_rebuilt_green_is_no_longer_rotten() {
    // the rotten scenario, under --wait: then api U-001's row is fixed in
    // the spec, the unit is rebuilt, lands green, and the run completes
    let r = rig_of("project-cascade");
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    r.edit("specs/core/sdd.md", "version: 1.0.0", "version: 1.1.0");
    r.edit(
        "specs/core/units/U-001-data.md",
        "with the frozen content.",
        "with the frozen content, again.",
    );
    git(&r.root, &["commit", "-q", "-am", "core CH-001"]);
    let _ = r.run(&["audit", "--apply-stale", "--sdd", "specs/core"], &[]);
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "core CH-001: staleness applied"],
    );
    let v2 = r.support.join("solution-v2").display().to_string();
    let bg = r.start(&["run", "--cap", "2", "--wait"], &[("STUB_REF", &v2)]);
    let out = |bg: &Background| std::fs::read_to_string(&bg.out).unwrap_or_default();
    wait_for("the run to wait on the rotten unit", || {
        out(&bg).contains("waiting: the run resumes when the repository changes")
    });
    assert!(
        out(&bg).contains("  api      Blocked      U-001 rotten: "),
        "{}",
        out(&bg)
    );

    // the row now asks for what core ships: api U-001 is rebuilt
    r.edit("specs/api/sdd.md", "version: 1.0.0", "version: 1.1.0");
    r.edit(
        "specs/api/units/U-001-out.md",
        "grep -qx 'v1' core/data.txt",
        "grep -qx 'v2' core/data.txt",
    );
    git(
        &r.root,
        &["commit", "-q", "-am", "api CH-001: U-001 reads v2"],
    );
    let _ = r.run(&["audit", "--apply-stale", "--sdd", "specs/api"], &[]);
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "api CH-001: staleness applied"],
    );
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("[api] integrated U-001 (c2)"), "{stdout}");
    let report = &stdout[stdout.rfind("Complete.").unwrap()..];
    assert!(!report.contains("rotten"), "{report}");
    assert!(report.contains("  api      Complete"), "{report}");
}

#[test]
fn a_pin_in_a_blocked_chain_shows_what_it_resolved_to() {
    // api alone: its units wait on core's, outside this run
    let r = rig_of("project-pipeline");
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/api"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains(
            "  U-003: dependency chain blocked: U-002 (Ready) ← U-001 (Ready) ← C-001 → core U-001, outside this run"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("C-001 ("), "{stdout}");
}
