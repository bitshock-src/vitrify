//! The full orchestrator loop on run-mini with the stub
//! worker — Complete with the evidence triple; planted-Q variant Blocked
//! naming the Q and the dependent chain; a write-set violation released and
//! named; --dry-run; preflight refusals, and the Test command not gating
//! the start. Then what a run must survive: a repository with several SDDs,
//! an interrupt, a run that died holding a claim, a first attempt that
//! fails, a spec that changes while it builds.

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
    rig_of("run-mini")
}

/// The project of a `fixtures/seeded/<fixture>` committed on main.
fn rig_of(fixture: &str) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&root).unwrap();
    copy_dir(
        &repo_root()
            .join("fixtures/seeded")
            .join(fixture)
            .join("project"),
        &root,
    );
    let root = root.canonicalize().unwrap();
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
        self.command_from(Command::new(vitrify_bin()), args, extra_env)
    }

    /// `c` — vitrify, or something that execs it — in this rig's repository
    /// and environment.
    fn command_from(&self, mut c: Command, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
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

    /// Start a run in the background, its output going to files.
    fn start(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Background {
        self.start_command(self.command(args, extra_env))
    }

    fn start_command(&self, mut command: Command) -> Background {
        let out = self._tmp.path().join(format!(
            "run-{}.out",
            std::fs::read_dir(self._tmp.path()).unwrap().count()
        ));
        let err = out.with_extension("err");
        let child = command
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

    /// Where this repository's worktrees live: under the vitrify home, not
    /// in the working tree.
    fn worktrees(&self) -> PathBuf {
        let root = self.home.join("worktrees");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        assert_eq!(dirs.len(), 1, "one repository, one worktree root: {dirs:?}");
        dirs.pop().unwrap()
    }

    fn unit_on_main(&self, path: &str) -> String {
        git(&self.root, &["show", &format!("main:{path}")])
    }
}

/// A `vitrify run` in the background.
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

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn signal(&self, sig: &str) {
        let ok = Command::new("/bin/kill")
            .args([sig, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {sig}");
    }

    /// Wait for the run to end (at most 60 s).
    fn finish(mut self) -> (std::process::ExitStatus, String, String) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (status, self.stdout(), self.stderr());
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!(
                    "the run did not end\nstdout:\n{}\nstderr:\n{}",
                    self.stdout(),
                    self.stderr()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
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
fn running_worker(run_json: &serde_json::Value, unit: &str) -> Option<(u64, String)> {
    run_json["workers"].as_array()?.iter().find_map(|w| {
        (w["unit"] == unit && w["outcome"] == "running").then(|| {
            (
                w["pid"].as_u64().unwrap(),
                w["holder"].as_str().unwrap().to_string(),
            )
        })
    })
}

#[test]
fn complete_with_the_evidence_triple() {
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // job header printed before dispatch
    assert!(stdout.contains("SDD build job — mini @ 1.0.0"));
    assert!(stdout.contains("Done-condition: every non-Retired unit Verified @ 1.0.0"));
    assert!(stdout.contains("Plan: [P:a] U-001, U-002"));
    assert!(stdout.contains("dispatch U-001 (c1) → w1"), "{stdout}");
    assert!(stdout.contains("dispatch U-002 (c1) → w2"), "{stdout}");
    assert!(
        stdout.contains("Branches: sdd/mini/<unit>/c<seq>"),
        "{stdout}"
    );

    // the evidence triple, quoted
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"));
    assert!(stdout.contains("audit: exit 0 (clean)"));
    assert!(stdout.contains("replay: exit 0 (consistent)"));
    assert!(stdout.contains("§5 Test: exit 0 (true)"), "{stdout}");
    assert!(stdout.contains("journal tail:"));

    // repo state: both Verified at mainline, artifacts shipped
    for (unit_file, artifact) in [
        ("specs/mini/units/U-001-one.md", "src/one.txt"),
        ("specs/mini/units/U-002-two.md", "src/two.txt"),
    ] {
        let text = git(&r.root, &["show", &format!("main:{unit_file}")]);
        assert!(text.contains("status: Verified"), "{unit_file}");
        let blob = git(&r.root, &["show", &format!("main:{artifact}")]);
        assert!(!blob.is_empty());
    }
    let journal = git(&r.root, &["show", "main:specs/mini/journal.ndjson"]);
    for ev in [
        "\"event\":\"claim\"",
        "\"event\":\"built\"",
        "\"event\":\"verified\"",
    ] {
        assert!(journal.contains(ev), "journal missing {ev}");
    }
    // worktrees and branches pruned
    let branches = git(&r.root, &["branch", "--list", "sdd/*"]);
    assert!(branches.is_empty(), "stale branches: {branches}");

    // run.json written with the terminated record
    let run_json = r.run_json();
    assert_eq!(run_json["terminated"]["state"], "complete");
    assert_eq!(run_json["sdd"], "specs/mini");
    assert_eq!(run_json["reports"].as_array().unwrap().len(), 2);
    let workers = run_json["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 2);
    assert!(workers.iter().all(|w| w["outcome"] == "verified"));
    assert!(workers.iter().all(|w| w["ended_at"].is_string()));
    // the harness it ran, and for each attempt whether it was a retry and
    // why it ended (nothing to say for a verified one)
    assert_eq!(run_json["harness"], "claude");
    assert!(workers.iter().all(|w| w["retry"].is_boolean()));
    assert!(
        workers
            .iter()
            .all(|w| w.as_object().unwrap().contains_key("detail"))
    );
    assert!(workers.iter().all(|w| w["detail"].is_null()));

    // nothing was built or gated inside the working tree: the worktrees and
    // the integration checkout live under the vitrify home
    assert!(!r.root.join(".vitrify/worktrees").exists());
    assert!(!r.root.join(".vitrify/mainline").exists());
    let wt = r.worktrees();
    assert!(wt.join("mainline/src/one.txt").exists());
    assert!(
        workers[0]["worktree"]
            .as_str()
            .unwrap()
            .starts_with(wt.to_str().unwrap())
    );
    // the run's own scratch is gone: no job lock, no leftover transaction
    let leftovers: Vec<String> = std::fs::read_dir(r.root.join(".vitrify"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("tx-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(
        std::fs::read_dir(r.root.join(".vitrify/jobs"))
            .unwrap()
            .count(),
        0
    );
    // and the repository still looks clean to git
    assert!(git(&r.root, &["status", "--porcelain"]).is_empty());
}

#[test]
fn handoff_worktree_is_committed_by_the_orchestrator() {
    // workers never run `git commit`: they report built with the work left
    // in place, and the orchestrator commits the handoff (its fixed message)
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"));
    assert!(
        stderr.contains("committed handoff worktree for U-001 (c1)"),
        "{stderr}"
    );
    // the handoff commits carry the fixed message and the artifacts shipped
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(log.contains("sdd: build U-001 (c1)"), "{log}");
    for artifact in ["src/one.txt", "src/two.txt"] {
        assert!(!git(&r.root, &["show", &format!("main:{artifact}")]).is_empty());
    }
}

#[test]
fn planted_q_terminates_blocked_naming_q_and_chain() {
    let r = rig();
    // add a dependent unit so the Blocked report names a chain
    let dep_unit = "---\nunit: U-003\ntitle: dependent\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [U-001]\nparallel_group: —\nowns: [src/three.txt]\nconsumes: []\nbuilt: —\nverified: —\ntrace: —\n---\n\n# U-003\n\n## Requirements\n\n- **U-003.R1** — THE SYSTEM SHALL ship `src/three.txt`.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f src/three.txt` | exit 0 |\n";
    std::fs::write(r.root.join("specs/mini/units/U-003-three.md"), dep_unit).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "add dependent unit"]);

    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[("STUB_BLOCK_UNIT", "U-001"), ("STUB_Q", "Q-042")],
    );
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Blocked."), "{stdout}");
    // the planted Q is named for its unit
    assert!(stdout.contains("U-001: open Q-042"), "{stdout}");
    // the dependent chain roots in the blocked unit
    assert!(
        stdout.contains("U-003: dependency chain blocked: U-001"),
        "{stdout}"
    );
    // the other unit still landed
    assert!(stdout.contains("1/3 Verified"), "{stdout}");
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(u2.contains("status: Verified"));
    // the Q landed in the Decision Log as a mainline commit
    let decisions = git(&r.root, &["show", "main:specs/mini/decisions.md"]);
    assert!(decisions.contains("**Q-042**"), "{decisions}");
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(log.contains("sdd: record Q-042"));
    // … as an entry that says it is open: a reader that looks for the word
    // must not take a question the run has just raised for a settled one
    assert!(
        decisions.contains(", open) — reported by run-") && decisions.contains("-w1 on U-001: "),
        "{decisions}"
    );
    let (code, stdout, stderr) = r.run(&["questions", "--open"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.starts_with("Q-042  open      blocks U-001  reported by run-"),
        "{stdout}"
    );
    assert!(stdout.ends_with("1 open, 0 resolved\n"), "{stdout}");
}

#[test]
fn a_planted_q_takes_the_place_of_a_log_that_said_none() {
    let r = rig();
    // the Decision Log the author wrote says no question is open
    let log = "# Decisions & Open Questions — mini\n\n## Decision Log\n\n- **D-001** (2026-11-01, SDD 1.0.0) — a decision.\n\n## Open Questions\n\nNone.\n\n## Notes\n\nkept as written\n";
    std::fs::write(r.root.join("specs/mini/decisions.md"), log).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "decision log"]);

    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[("STUB_BLOCK_UNIT", "U-001"), ("STUB_Q", "Q-042")],
    );
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let decisions = r.unit_on_main("specs/mini/decisions.md");
    let (before, after) = decisions
        .split_once("## Open Questions\n\n- **Q-042** (")
        .unwrap_or_else(|| panic!("{decisions}"));
    assert!(before.ends_with("a decision.\n\n"), "{decisions}");
    assert!(
        after.contains(", open) — reported by run-") && after.contains("-w1 on U-001: "),
        "{decisions}"
    );
    // the placeholder is gone; the sections around it are as written
    assert!(!decisions.contains("None."), "{decisions}");
    // (`git show`, as the helper runs it, trims the last newline)
    assert!(
        after.ends_with("\n\n## Notes\n\nkept as written"),
        "{decisions}"
    );
    let (code, stdout, stderr) = r.run(&["questions", "--open"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.starts_with("Q-042  open      blocks U-001"),
        "{stdout}"
    );
}

#[test]
fn dry_run_prints_plan_and_stops() {
    let r = rig();
    let (code, stdout, _) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("SDD build job — mini @ 1.0.0"));
    assert!(stdout.contains("Plan: [P:a] U-001, U-002"));
    assert!(stdout.contains("Roles: worker model="), "{stdout}");
    assert!(stdout.contains("retry model="), "{stdout}");
    assert!(stdout.contains("dry-run: preflight green; stopping before dispatch"));
    // nothing dispatched, nothing claimed
    let unit = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(unit.contains("status: Ready"));
}

#[test]
fn a_repository_overrides_role_profiles_key_by_key() {
    let r = rig();
    let (code, stdout, _) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("Roles: worker model=opus effort=medium"),
        "{stdout}"
    );
    // the repository's own config.toml: the worker's model, nothing else
    std::fs::create_dir_all(r.root.join(".vitrify")).unwrap();
    std::fs::write(
        r.root.join(".vitrify/config.toml"),
        "schema = 1\n\n[harness.claude.worker]\nmodel = \"sonnet\"\n",
    )
    .unwrap();
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("Roles: worker model=sonnet effort=medium"),
        "{stdout}"
    );
    assert!(stdout.contains("retry model=opus effort=high"), "{stdout}");

    // the probe record describes the machine: refused there
    std::fs::write(
        r.root.join(".vitrify/config.toml"),
        "[probe.claude]\npresent = true\n",
    )
    .unwrap();
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("`probe` describes the machine and belongs in ~/.vitrify/config.toml"),
        "{stderr}"
    );
}

#[test]
fn a_dry_run_publishes_nothing() {
    // a lapsed lease is something preflight puts right — in a run. A dry
    // run says so and leaves mainline, and the checkout, exactly as they are
    let r = rig();
    let out = r
        .command(
            &[
                "claim",
                "--unit",
                "U-002",
                "--holder",
                "ghost",
                "--lease-minutes",
                "0",
            ],
            &[],
        )
        .output()
        .unwrap();
    assert!(out.status.success());
    let head = git(&r.root, &["rev-parse", "main"]);

    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("preflight: 1 lapsed lease(s) — a run releases them before it starts"),
        "{stderr}"
    );
    assert!(stdout.contains("dry-run: preflight green"), "{stdout}");
    assert_eq!(git(&r.root, &["rev-parse", "main"]), head);
    assert!(git(&r.root, &["status", "--porcelain"]).is_empty());
    assert!(
        r.unit_on_main("specs/mini/units/U-002-two.md")
            .contains("claimed_by: ghost")
    );

    // the run itself releases it, and builds the unit
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("preflight: 1 lapsed lease(s) fixed and published"),
        "{stderr}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
}

#[test]
fn preflight_refuses_non_active_sdd_and_missing_probe() {
    let r = rig();
    // Draft SDD
    let sdd_path = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd_path).unwrap();
    std::fs::write(&sdd_path, text.replace("status: Active", "status: Draft")).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "draft"]);
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: SDD status is Draft, not Active → finish authoring and set it Active: vitrify brief authoring"
        ),
        "{stderr}"
    );

    // restore Active but drop the probe record
    let text = std::fs::read_to_string(&sdd_path).unwrap();
    std::fs::write(&sdd_path, text.replace("status: Draft", "status: Active")).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "active"]);
    std::fs::write(
        r.home.join("config.toml"),
        vitrify_core::content::CONFIG.body,
    )
    .unwrap();
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: ~/.vitrify/config.toml records no working harness → vitrify probe"
        ),
        "{stderr}"
    );

    // and without the file at all, the machine is not set up
    std::fs::remove_file(r.home.join("config.toml")).unwrap();
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 4);
    assert_eq!(
        stderr,
        "error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init\n"
    );
}

#[test]
fn preflight_refuses_a_broken_role_profile() {
    let r = rig();
    let (code, _, _) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0);
    // break the per-role profiles in config.toml
    let path = r.home.join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let broken = text.replacen(
        "[harness.claude.worker]\nmodel = \"opus\"",
        "[harness.claude.worker]\nmodel = 3",
        1,
    );
    assert_ne!(broken, text);
    std::fs::write(&path, broken).unwrap();
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: ~/.vitrify/config.toml: `harness.claude.worker.model` must be a string → correct the line the error names, then vitrify probe"
        ),
        "{stderr}"
    );
}

#[test]
fn preflight_does_not_gate_on_the_test_command() {
    // whether tests exist or pass before the build is the SDD's test posture:
    // a failing Test command, or none at all, does not block the run
    let r = rig();
    let sdd_path = r.root.join("specs/mini/sdd.md");
    let original = std::fs::read_to_string(&sdd_path).unwrap();
    for test_line in ["- Test:      `false`", "- Test:      none"] {
        std::fs::write(
            &sdd_path,
            original.replace("- Test:      `true`", test_line),
        )
        .unwrap();
        git(&r.root, &["add", "-A"]);
        git(&r.root, &["commit", "-q", "-m", "test posture"]);
        let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
        assert_eq!(code, 0, "{test_line}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        assert!(stdout.contains("dry-run: preflight green"), "{stdout}");
    }
}

#[test]
fn preflight_refuses_a_failing_lint_command() {
    let r = rig();
    let sdd_path = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd_path).unwrap();
    std::fs::write(
        &sdd_path,
        text.replace("- Lint:      `true`", "- Lint:      `false`"),
    )
    .unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "red lint"]);
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: §5 lint command failed: false → run it from the repository root and fix what it reports"
        ),
        "{stderr}"
    );
}

#[test]
fn write_set_violation_releases_the_claim_and_names_the_blocker() {
    // U-001's "worker" also writes src/extra.txt, a path no unit owns: every
    // attempt is rejected before merging, U-002 still lands, and the Blocked
    // report names the write-set check
    let r = rig();
    let sol = r._tmp.path().join("overreach-solution");
    copy_dir(&repo_root().join("fixtures/seeded/run-mini/solution"), &sol);
    std::fs::write(sol.join("src/extra.txt"), "overreach\n").unwrap();
    let map = r._tmp.path().join("overreach-map.toml");
    std::fs::write(
        &map,
        "[units]\nU-001 = [\"src/one.txt\", \"src/extra.txt\"]\nU-002 = [\"src/two.txt\"]\n",
    )
    .unwrap();

    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[
            ("STUB_REF", sol.to_str().unwrap()),
            ("STUB_MAP", map.to_str().unwrap()),
        ],
    );
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Blocked."), "{stdout}");
    assert!(stdout.contains("1/2 Verified"), "{stdout}");
    assert!(
        stdout.contains("U-001: failing gate `write-set check`: src/extra.txt: not owned by U-001"),
        "{stdout}"
    );
    assert!(
        stderr.contains("write-set violation integrating U-001: src/extra.txt"),
        "{stderr}"
    );
    // nothing of U-001's landed; its claim was released each time
    let tree = git(&r.root, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(!tree.contains("src/extra.txt"), "{tree}");
    assert!(!tree.contains("src/one.txt"), "{tree}");
    let u1 = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(u1.contains("status: Ready"), "{u1}");
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(u2.contains("status: Verified"), "{u2}");
    // run.json says why each of U-001's attempts failed
    let run_json = r.run_json();
    let failed: Vec<&serde_json::Value> = run_json["workers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["unit"] == "U-001")
        .collect();
    assert!(!failed.is_empty(), "{run_json}");
    for w in failed {
        assert_ne!(w["outcome"], "verified", "{w}");
        assert!(w["detail"].is_string(), "{w}");
    }
}

#[test]
fn a_run_appends_its_event_log() {
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let path = r.root.join(format!(".vitrify/logs/{job}/events.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();
    let events: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();

    let first = events.first().unwrap();
    assert_eq!(first["event"], "run-started", "{text}");
    assert_eq!(first["sdd"], "specs/mini", "{text}");
    assert_eq!(first["harness"], "claude", "{text}");
    assert_eq!(first["cap"], 2, "{text}");
    let last = events.last().unwrap();
    assert_eq!(last["event"], "run-ended", "{text}");
    assert_eq!(last["state"], "complete", "{text}");

    for unit in ["U-001", "U-002"] {
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "dispatched" && e["unit"] == unit && e["retry"] == false),
            "no dispatch of {unit}: {text}"
        );
        assert!(
            events.iter().any(|e| e["event"] == "outcome"
                && e["unit"] == unit
                && e["outcome"] == "verified"
                && e["claim_seq"] == 1
                && e["duration_s"].is_i64()),
            "no verified outcome for {unit}: {text}"
        );
    }
    // the integrator's steps are in it too, with the units each one gated
    assert!(
        events.iter().any(|e| e["event"] == "gate"
            && e["step"] == "rows"
            && e["batch"].as_array().is_some_and(|b| !b.is_empty())),
        "no gate event for the rows: {text}"
    );
    assert!(
        !events
            .iter()
            .any(|e| e["event"] == "gate" && e["step"] == "idle"),
        "{text}"
    );
}

#[test]
fn preflight_refuses_uncommitted_spec_changes() {
    let r = rig();
    std::fs::write(
        r.root.join("specs/mini/sdd.md"),
        std::fs::read_to_string(r.root.join("specs/mini/sdd.md")).unwrap() + "\nedited\n",
    )
    .unwrap();
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("not committed at the mainline head"),
        "{stderr}"
    );
}

// ------------------------------------------------- several SDDs, one repo

fn multi_env(which: &str) -> [(&'static str, String); 2] {
    let fixture = repo_root().join("fixtures/seeded/run-multi");
    [
        ("STUB_REF", fixture.join("solution").display().to_string()),
        (
            "STUB_MAP",
            fixture
                .join(format!("stub-map-{which}.toml"))
                .display()
                .to_string(),
        ),
    ]
}

fn borrowed<'a>(env: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    env.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

#[test]
fn a_repository_with_several_sdds_builds_each_by_its_sdd_flag() {
    let r = rig_of("run-multi");

    // no --sdd: refused before anything starts, the candidates listed
    let (code, _, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("multiple SDDs found; pass --sdd <root>"),
        "{stderr}"
    );
    assert!(stderr.contains("specs/alpha") && stderr.contains("specs/beta"));

    // alpha, with its U-001 blocked: the SDD named in preflight is the SDD
    // of every later step — the claim, the worker's own verbs, the gate
    let alpha = multi_env("alpha");
    let mut env = borrowed(&alpha);
    env.push(("STUB_BLOCK_UNIT", "U-001"));
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/alpha", "--cap", "2"], &env);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("SDD build job — alpha @ 1.0.0"), "{stdout}");
    assert!(
        stdout.contains("Branches: sdd/alpha/<unit>/c<seq>"),
        "{stdout}"
    );
    assert!(stdout.contains("integrated U-002 (c1)"), "{stdout}");
    assert!(stdout.contains("U-001: open Q-999"), "{stdout}");
    assert!(
        r.unit_on_main("specs/alpha/units/U-002-two.md")
            .contains("status: Verified")
    );
    // the blocked attempt's work is kept on a branch that carries the slug
    let kept = git(&r.root, &["rev-parse", "sdd/alpha/U-001/c1"]);
    assert!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/alpha/U-001/c1"])
            .starts_with("sdd: abandoned attempt U-001 (c1) — blocked")
    );
    // beta was never touched
    for unit in ["U-001-one", "U-002-two"] {
        assert!(
            r.unit_on_main(&format!("specs/beta/units/{unit}.md"))
                .contains("status: Ready")
        );
    }

    // beta has a U-001 too, and claims it as c1 — its own names, and what
    // alpha left is exactly where it was
    let beta = multi_env("beta");
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/beta"], &borrowed(&beta));
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("SDD build job — beta @ 1.0.0"), "{stdout}");
    assert!(stdout.contains("dispatch U-001 (c1)"), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    let reports = r.run_json()["reports"].to_string();
    assert!(reports.contains("sdd/beta/U-001/c1"), "{reports}");
    assert_eq!(git(&r.root, &["rev-parse", "sdd/alpha/U-001/c1"]), kept);
    for path in ["b/one.txt", "b/two.txt", "a/two.txt"] {
        assert!(!git(&r.root, &["show", &format!("main:{path}")]).is_empty());
    }

    // alpha again, unblocked: its U-001 goes as a retry and it completes
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/alpha"], &borrowed(&alpha));
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("dispatch U-001 (c2) → w1  [retry]"),
        "{stdout}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    // each SDD's own audit is clean
    for sdd in ["specs/alpha", "specs/beta"] {
        let (code, stdout, stderr) = r.run(&["audit", "--sdd", sdd], &[]);
        assert_eq!(code, 0, "{sdd}: {stdout}{stderr}");
    }
}

#[test]
fn a_name_taken_by_something_else_is_left_alone() {
    // the branch a first claim of U-001 would use already exists, with
    // someone's commit on it: the run neither deletes nor reuses it
    let r = rig();
    git(&r.root, &["checkout", "-q", "-b", "sdd/mini/U-001/c1"]);
    std::fs::write(r.root.join("keep.txt"), "not vitrify's\n").unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "someone's work"]);
    git(&r.root, &["checkout", "-q", "main"]);
    let theirs = git(&r.root, &["rev-parse", "sdd/mini/U-001/c1"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains(
            "U-001 (c1): branch sdd/mini/U-001/c1 already exists and is not this run's — left untouched"
        ),
        "{stderr}"
    );
    // the claim that met the taken name was released; the next one (c2) built
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    assert_eq!(git(&r.root, &["rev-parse", "sdd/mini/U-001/c1"]), theirs);
    assert!(!git(&r.root, &["ls-tree", "-r", "--name-only", "main"]).contains("keep.txt"));
}

#[test]
fn a_taken_name_does_not_end_a_run_whose_only_startable_unit_met_it() {
    // U-002 waits for U-001, so U-001 is all a run can start — and the
    // branch its first claim would use is somebody else's. The claim is
    // given back and the next one builds: the run must not call itself
    // Blocked on what it saw before it gave the claim back.
    let r = rig();
    let u2 = r.root.join("specs/mini/units/U-002-two.md");
    let text = std::fs::read_to_string(&u2).unwrap();
    std::fs::write(
        &u2,
        text.replace("depends_on: []", "depends_on: [U-001]")
            .replace("parallel_group: a", "parallel_group: —"),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "U-002 after U-001"]);
    git(&r.root, &["branch", "sdd/mini/U-001/c1"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("branch sdd/mini/U-001/c1 already exists and is not this run's"),
        "{stderr}"
    );
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
}

// ------------------------------------------------------------ interruption

#[test]
fn an_interrupt_stops_the_workers_releases_the_claims_and_keeps_the_record() {
    let r = rig();
    let bg = r.start(&["run", "--cap", "2"], &[("STUB_HANG_UNIT", "U-001")]);
    wait_for("U-002 to land while U-001's worker runs", || {
        running_worker(&r.run_json(), "U-001").is_some()
            && bg.stdout().contains("integrated U-002 (c1)")
    });
    // the run's own record exists while it runs
    let mid = r.run_json();
    assert!(mid["job_id"].as_str().unwrap().starts_with("run-"));
    assert!(mid["terminated"].is_null());
    let (pid, holder) = running_worker(&mid, "U-001").unwrap();
    assert!(group_alive(pid));
    assert!(
        r.unit_on_main("specs/mini/units/U-001-one.md")
            .contains(&format!("claimed_by: {holder}"))
    );

    bg.signal("-TERM");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(15),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Interrupted (SIGTERM)"), "{stdout}");

    // the worker is gone, its claim released, the run's record closed
    assert!(
        !group_alive(pid),
        "the worker's process group outlived the run"
    );
    let u1 = r.unit_on_main("specs/mini/units/U-001-one.md");
    assert!(u1.contains("status: Ready"), "{u1}");
    assert!(u1.contains("claimed_by: —"), "{u1}");
    let end = r.run_json();
    assert_eq!(end["terminated"]["state"], "interrupted");
    assert_eq!(end["terminated"]["detail"], "SIGTERM");
    let w = end["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["unit"] == "U-001")
        .unwrap();
    assert_eq!(w["outcome"], "interrupted");
    // what the worker had is on its branch, with the reason; its worktree
    // is gone
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/mini/U-001/c1"]),
        "sdd: abandoned attempt U-001 (c1) — interrupted"
    );
    assert!(!r.worktrees().join("mini-U-001-c1").exists());
    assert_eq!(
        std::fs::read_dir(r.root.join(".vitrify/jobs"))
            .unwrap()
            .count(),
        0
    );

    // and the next run simply continues
    let (code, stdout, stderr) = r.run(&["run"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("dispatch U-001 (c2) → w1  [retry]"),
        "{stdout}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
}

#[test]
fn an_interrupt_during_the_gate_publishes_nothing() {
    // U-001's Verification row is slow — in the gate. Ctrl-C arrives there.
    let r = rig();
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(
        &unit,
        text.replace(
            "`test -f src/one.txt`",
            "`sh -c 'touch ../gate-started; sleep 60'`",
        ),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "a slow row"]);

    let bg = r.start(&["run", "--cap", "1"], &[]);
    wait_for("the gate to reach the slow row", || {
        std::fs::read_dir(r.home.join("worktrees"))
            .map(|d| d.flatten().any(|e| e.path().join("gate-started").exists()))
            .unwrap_or(false)
    });
    let before = Instant::now();
    bg.signal("-INT");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(2),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        before.elapsed() < Duration::from_secs(30),
        "the gate command was not stopped"
    );
    assert!(stdout.contains("Interrupted (SIGINT)"), "{stdout}");
    // nothing of the half-gated work reached mainline; the claim is free
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(!log.contains("integrate U-001"), "{log}");
    assert!(!log.contains("verify U-001"), "{log}");
    let u1 = r.unit_on_main("specs/mini/units/U-001-one.md");
    assert!(u1.contains("status: Ready"), "{u1}");
    // the finished build is kept for the next attempt
    let kept = git(&r.root, &["log", "-2", "--format=%s", "sdd/mini/U-001/c1"]);
    assert!(
        kept.contains("sdd: abandoned attempt U-001 (c1) — interrupted")
            && kept.contains("sdd: build U-001 (c1)"),
        "{kept}"
    );
    assert!(!git(&r.root, &["show", "sdd/mini/U-001/c1:src/one.txt"]).is_empty());
}

#[test]
fn a_signal_the_run_was_started_ignoring_stays_ignored() {
    // `nohup vitrify run &`: SIGHUP is set to "ignore" before vitrify
    // starts, and the terminal closing must not end the run
    let r = rig();
    let mut sh = Command::new("sh");
    sh.args(["-c", "trap '' HUP; exec \"$0\" \"$@\"", vitrify_bin()]);
    let mut bg =
        r.start_command(r.command_from(sh, &["run", "--cap", "2"], &[("STUB_HANG_UNIT", "U-001")]));
    wait_for("U-002 to land while U-001's worker runs", || {
        running_worker(&r.run_json(), "U-001").is_some()
            && bg.stdout().contains("integrated U-002 (c1)")
    });
    let (pid, _) = running_worker(&r.run_json(), "U-001").unwrap();

    bg.signal("-HUP");
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        bg.running(),
        "the run ended on a signal it was started ignoring\n{}",
        bg.stdout()
    );
    assert!(group_alive(pid), "its worker was stopped");
    assert!(r.run_json()["terminated"].is_null());

    // a signal it was not told to ignore still ends it, cleanly
    bg.signal("-TERM");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(15),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Interrupted (SIGTERM)"), "{stdout}");
    assert!(!group_alive(pid));
}

#[test]
fn a_run_whose_output_nobody_reads_still_builds_and_ends_cleanly() {
    // `vitrify run | head -1`: the reader is gone, every later line fails to
    // be written. That must not take the run down with workers running.
    let r = rig();
    let mut child = r
        .command(&["run", "--cap", "2"], &[])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    for unit in ["U-001-one", "U-002-two"] {
        let text = r.unit_on_main(&format!("specs/mini/units/{unit}.md"));
        assert!(text.contains("status: Verified"), "{unit}: {text}");
    }
    assert_eq!(r.run_json()["terminated"]["state"], "complete");
    assert_eq!(
        std::fs::read_dir(r.root.join(".vitrify/jobs"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn a_run_that_died_is_recovered_by_the_next() {
    let r = rig();
    let bg = r.start(&["run", "--cap", "2"], &[("STUB_HANG_UNIT", "U-001")]);
    wait_for("U-002 to land while U-001's worker runs", || {
        running_worker(&r.run_json(), "U-001").is_some()
            && bg.stdout().contains("integrated U-002 (c1)")
    });
    let (pid, holder) = running_worker(&r.run_json(), "U-001").unwrap();
    // killed outright: no handler runs
    bg.signal("-KILL");
    let (status, _, _) = bg.finish();
    assert_eq!(status.signal(), Some(9));

    // what it left: its own record of what it was doing, a claim with most
    // of an hour to run, and a worker that outlived it
    let dead = r.run_json();
    assert!(dead["terminated"].is_null());
    assert_eq!(running_worker(&dead, "U-001"), Some((pid, holder.clone())));
    let u1 = r.unit_on_main("specs/mini/units/U-001-one.md");
    assert!(u1.contains("status: Claimed"), "{u1}");
    assert!(u1.contains(&format!("claimed_by: {holder}")), "{u1}");
    assert!(group_alive(pid));

    // --dry-run says what a run would do about it, and does nothing
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains(&format!(
            "Recovery: would release U-001 (c1), claimed by {holder} — that run is gone"
        )),
        "{stdout}"
    );
    assert!(
        r.unit_on_main("specs/mini/units/U-001-one.md")
            .contains("status: Claimed")
    );

    // the next run releases the dead claim and builds the unit
    let (code, stdout, stderr) = r.run(&["run"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains(&format!(
            "recovered U-001 (c1): its holder {holder} is gone — claim released"
        )),
        "{stdout}"
    );
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    // it cannot stop a worker it did not start, but it says so
    assert!(
        stderr.contains(&format!("stop it with: kill -9 -- -{pid}")),
        "{stderr}"
    );
    kill_group(pid);
    // the dead run's scratch is swept, its attempt kept on its branch
    let leftovers: Vec<String> = std::fs::read_dir(r.root.join(".vitrify"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("tx-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(
        std::fs::read_dir(r.root.join(".vitrify/jobs"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/mini/U-001/c1"]),
        "sdd: abandoned attempt U-001 (c1) — interrupted"
    );
    let journal = r.unit_on_main("specs/mini/journal.ndjson");
    assert!(
        journal.contains(&format!(
            r#""event":"release","unit":"U-001","actor":"{holder}","claim_seq":1"#
        )),
        "{journal}"
    );
}

#[test]
fn a_live_run_s_claims_are_not_recovered() {
    // two runs side by side: the second must not take the first one's claim
    let r = rig();
    let first = r.start(&["run", "--cap", "1"], &[("STUB_HANG_UNIT", "U-001")]);
    wait_for("U-001's worker to start", || {
        running_worker(&r.run_json(), "U-001").is_some()
    });
    let (pid, holder) = running_worker(&r.run_json(), "U-001").unwrap();
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(!stdout.contains("Recovery:"), "{stdout}");
    assert!(
        r.unit_on_main("specs/mini/units/U-001-one.md")
            .contains(&format!("claimed_by: {holder}"))
    );
    first.signal("-TERM");
    let (status, _, _) = first.finish();
    assert_eq!(status.signal(), Some(15));
    assert!(!group_alive(pid));
}

// ------------------------------------------------------------------ retries

#[test]
fn a_retry_starts_from_the_last_attempt_and_is_told_why_it_failed() {
    let r = rig();
    // judge the content, so a wrong first draft is rejected by the gate
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(
        &unit,
        text.replace("`test -f src/one.txt`", "`grep -qx one src/one.txt`"),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "judge the content"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_DRAFT_UNIT", "U-001")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stderr.contains("gate red integrating U-001: grep -qx one src/one.txt"));
    assert!(
        stdout.contains("dispatch U-001 (c2) → w3  [retry]"),
        "{stdout}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");

    // the second worker was handed the first one's file and its failure
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let logs = r.root.join(".vitrify/logs").join(&job);
    let prompt = std::fs::read_to_string(logs.join(format!("worker-{job}-w3.prompt.md"))).unwrap();
    assert!(prompt.contains("## This session"), "{prompt}");
    assert!(
        prompt.contains("An earlier attempt at this unit did not land: `rejected-gate`"),
        "{prompt}"
    );
    assert!(prompt.contains("branch `sdd/mini/U-001/c1`"));
    assert!(
        prompt.contains("failing gate: `grep -qx one src/one.txt`"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Its files are already in your working copy**, uncommitted: `src/one.txt`"),
        "{prompt}"
    );
    let err = std::fs::read_to_string(logs.join(format!("worker-{job}-w3.err"))).unwrap();
    assert!(
        err.contains("stub: session=true earlier-attempt=true"),
        "{err}"
    );
    // a first attempt carries the orchestrator's note, and no earlier attempt
    let first = std::fs::read_to_string(logs.join(format!("worker-{job}-w1.err"))).unwrap();
    assert!(
        first.contains("stub: session=true earlier-attempt=false"),
        "{first}"
    );
    let first_prompt =
        std::fs::read_to_string(logs.join(format!("worker-{job}-w1.prompt.md"))).unwrap();
    assert!(first_prompt.contains("Your claim is made and your working copy is ready"));
    assert!(first_prompt.contains("sdd: specs/mini\n"), "{first_prompt}");

    // the rejected attempt stays on its branch, with why; the one that
    // landed is pruned
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/mini/U-001/c1"]),
        "sdd: abandoned attempt U-001 (c1) — rejected-gate"
    );
    assert!(git(&r.root, &["branch", "--list", "sdd/mini/U-001/c2"]).is_empty());
    assert_eq!(r.unit_on_main("src/one.txt"), "one");
}

#[test]
fn a_unit_that_keeps_failing_is_judged_once_and_then_set_aside() {
    // U-001 can never pass its gate. Two failed attempts: the judge is
    // asked — once — and says retry. Two more: the unit is set aside, and
    // the run ends with everything else built.
    let r = rig();
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("`test -f src/one.txt`", "`false`")).unwrap();
    git(&r.root, &["commit", "-q", "-am", "a row that never passes"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_VERDICT", "retry")]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        stdout
            .matches("escalation verdict: retry (stub judge)")
            .count(),
        1,
        "{stdout}"
    );
    assert!(
        stderr.contains(
            "U-001: 4 attempts did not land (last: rejected-gate) — set aside for this run"
        ),
        "{stderr}"
    );
    assert!(stdout.contains("dispatch U-001 (c4)"), "{stdout}");
    assert!(!stdout.contains("dispatch U-001 (c5)"), "{stdout}");
    assert!(stdout.contains("integrated U-002 (c1)"), "{stdout}");
    assert!(stdout.contains("Blocked."), "{stdout}");
    assert!(
        stdout.contains("set aside after 4 attempts that did not land"),
        "{stdout}"
    );
    // nothing of it is left claimed, and each attempt is on its branch
    let u1 = r.unit_on_main("specs/mini/units/U-001-one.md");
    assert!(u1.contains("status: Ready"), "{u1}");
    assert!(u1.contains("claim_seq: 4"), "{u1}");
    for seq in 1..=4 {
        assert_eq!(
            git(
                &r.root,
                &[
                    "log",
                    "-1",
                    "--format=%s",
                    &format!("sdd/mini/U-001/c{seq}")
                ]
            ),
            format!("sdd: abandoned attempt U-001 (c{seq}) — rejected-gate")
        );
    }
}

#[test]
fn an_abort_verdict_stops_the_run_and_what_it_was_doing() {
    // U-001 can never pass its gate and the judge says abort, while U-002's
    // worker is still at it: the run ends Blocked, that worker is stopped,
    // its claim released and its work kept — recorded as cut short by the
    // abort, not as blocked on a question.
    let r = rig();
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("`test -f src/one.txt`", "`false`")).unwrap();
    git(&r.root, &["commit", "-q", "-am", "a row that never passes"]);

    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[("STUB_VERDICT", "abort"), ("STUB_HANG_UNIT", "U-002")],
    );
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("escalation verdict: abort (stub judge)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("escalation verdict abort — stopping: stub judge"),
        "{stdout}"
    );
    let record = r.run_json();
    assert_eq!(record["terminated"]["state"], "blocked");
    assert_eq!(
        record["terminated"]["detail"],
        "escalation verdict abort: stub judge"
    );
    let u2 = record["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["unit"] == "U-002")
        .unwrap();
    assert_eq!(u2["outcome"], "aborted");
    assert!(!group_alive(u2["pid"].as_u64().unwrap()));
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/mini/U-002/c1"]),
        "sdd: abandoned attempt U-002 (c1) — aborted"
    );
    // nothing is left claimed
    for unit in ["U-001-one", "U-002-two"] {
        let text = r.unit_on_main(&format!("specs/mini/units/{unit}.md"));
        assert!(text.contains("status: Ready"), "{unit}: {text}");
    }
}

#[test]
fn workers_never_share_a_tmpdir_while_they_run() {
    // both workers wait for each other before building, so they are
    // certainly alive at the same time
    let r = rig();
    let meet = r._tmp.path().join("meet");
    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2"],
        &[("STUB_RENDEZVOUS", &format!("{}:2", meet.display()))],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let logs = r.root.join(".vitrify/logs").join(&job);
    let tmp_of = |w: &str| {
        let err = std::fs::read_to_string(logs.join(format!("worker-{job}-{w}.err"))).unwrap();
        err.lines()
            .find_map(|l| l.strip_prefix("stub: TMPDIR="))
            .unwrap()
            .to_string()
    };
    let (a, b) = (tmp_of("w1"), tmp_of("w2"));
    assert_ne!(a, b, "one temp directory per running worker");
    for dir in [&a, &b] {
        assert!(dir.contains("/vfy-"), "{dir}");
        // slots persist: the next worker takes one over
        assert!(Path::new(dir).is_dir(), "{dir}");
    }
}

#[test]
fn a_streamed_session_is_logged_as_it_happens_and_its_report_found() {
    // a streaming spelling writes one event per line; the report is the
    // last line of the final event's text
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_STREAM", "1")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    assert!(
        stderr.contains("report [w1] U-001: built — stub built"),
        "{stderr}"
    );
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let out = std::fs::read_to_string(
        r.root
            .join(".vitrify/logs")
            .join(&job)
            .join(format!("worker-{job}-w1.out")),
    )
    .unwrap();
    assert!(
        out.lines().count() >= 3 && out.contains(r#""type":"result""#),
        "{out}"
    );
}

// ---------------------------------------------------------------- dispatch

#[test]
fn every_claimable_unit_is_dispatched_whatever_its_group() {
    // three independent units: one in group a, one in group b, one in no
    // group. Each worker waits for the other two before it builds.
    let r = rig();
    let u2 = r.root.join("specs/mini/units/U-002-two.md");
    let text = std::fs::read_to_string(&u2).unwrap();
    std::fs::write(&u2, text.replace("parallel_group: a", "parallel_group: b")).unwrap();
    let u3 = std::fs::read_to_string(r.root.join("specs/mini/units/U-001-one.md"))
        .unwrap()
        .replace("U-001", "U-003")
        .replace("one.txt", "three.txt")
        .replace("parallel_group: a", "parallel_group: —");
    std::fs::write(r.root.join("specs/mini/units/U-003-three.md"), u3).unwrap();
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "three units, three groupings"],
    );
    let sol = r._tmp.path().join("solution");
    copy_dir(&repo_root().join("fixtures/seeded/run-mini/solution"), &sol);
    std::fs::write(sol.join("src/three.txt"), "three\n").unwrap();
    let map = r._tmp.path().join("map.toml");
    std::fs::write(
        &map,
        "[units]\nU-001 = [\"src/one.txt\"]\nU-002 = [\"src/two.txt\"]\nU-003 = [\"src/three.txt\"]\n",
    )
    .unwrap();
    let meet = r._tmp.path().join("meet");

    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "3"],
        &[
            ("STUB_REF", sol.to_str().unwrap()),
            ("STUB_MAP", map.to_str().unwrap()),
            ("STUB_RENDEZVOUS", &format!("{}:3", meet.display())),
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Progress: 3/3 Verified"), "{stdout}");
    // all three were started before any was integrated
    let first_integrated = stdout.find("integrated ").unwrap();
    for unit in ["U-001", "U-002", "U-003"] {
        let at = stdout.find(&format!("dispatch {unit} (c1)")).unwrap();
        assert!(at < first_integrated, "{stdout}");
    }
}

#[test]
fn a_freed_slot_is_refilled_while_other_workers_still_run() {
    // cap 2, three independent units: U-001 hangs until U-003 has been
    // dispatched — which can only happen if U-002's slot is refilled while
    // U-001 is still running
    let r = rig();
    let u3 = std::fs::read_to_string(r.root.join("specs/mini/units/U-001-one.md"))
        .unwrap()
        .replace("U-001", "U-003")
        .replace("one.txt", "three.txt");
    std::fs::write(r.root.join("specs/mini/units/U-003-three.md"), u3).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "a third unit"]);
    let sol = r._tmp.path().join("solution");
    copy_dir(&repo_root().join("fixtures/seeded/run-mini/solution"), &sol);
    std::fs::write(sol.join("src/three.txt"), "three\n").unwrap();
    let map = r._tmp.path().join("map.toml");
    std::fs::write(
        &map,
        "[units]\nU-001 = [\"src/one.txt\"]\nU-002 = [\"src/two.txt\"]\nU-003 = [\"src/three.txt\"]\n",
    )
    .unwrap();

    let bg = r.start(
        &["run", "--cap", "2"],
        &[
            ("STUB_REF", sol.to_str().unwrap()),
            ("STUB_MAP", map.to_str().unwrap()),
            ("STUB_HANG_UNIT", "U-001"),
        ],
    );
    wait_for(
        "U-003 to be built and integrated beside the hanging U-001",
        || bg.stdout().contains("integrated U-003 (c1)"),
    );
    let (pid, _) = running_worker(&r.run_json(), "U-001").expect("U-001 still running");
    assert!(group_alive(pid));
    bg.signal("-TERM");
    let (status, stdout, _) = bg.finish();
    assert_eq!(status.signal(), Some(15));
    assert!(stdout.contains("integrated U-002 (c1)"), "{stdout}");
}

// ------------------------------------------------- the spec changes mid-run

fn amend_requirement(r: &Rig, unit_file: &str, from: &str, to: &str, message: &str) {
    let path = r.root.join(unit_file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from));
    std::fs::write(&path, text.replace(from, to)).unwrap();
    git(&r.root, &["commit", "-q", "-am", message]);
}

#[test]
fn a_blocked_unit_resumes_when_its_spec_is_fixed() {
    // U-001 cannot be built until its requirement says "notarized"; the run
    // waits, the fix lands on mainline, the run carries on — no restart
    let r = rig();
    let bg = r.start(
        &["run", "--wait", "--cap", "2"],
        &[
            ("STUB_BLOCK_UNLESS", "U-001:notarized"),
            ("STUB_Q", "Q-007"),
        ],
    );
    wait_for("the run to be waiting on the open question", || {
        bg.stdout()
            .contains("waiting: the run resumes when the spec changes")
    });
    let so_far = bg.stdout();
    assert!(so_far.contains("Blocked (waiting)."), "{so_far}");
    assert!(so_far.contains("U-001: open Q-007"), "{so_far}");
    assert!(so_far.contains("integrated U-002 (c1)"), "{so_far}");
    assert!(
        r.unit_on_main("specs/mini/decisions.md")
            .contains("**Q-007**")
    );

    amend_requirement(
        &r,
        "specs/mini/units/U-001-one.md",
        "with the frozen content",
        "with the frozen content, notarized",
        "mini CH-001: U-001.R1 says notarized (resolves Q-007)",
    );

    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("U-001: its reading set changed — back in the pool"),
        "{stdout}"
    );
    assert!(
        stdout.contains("dispatch U-001 (c2) → w3  [retry]"),
        "{stdout}"
    );
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
}

#[test]
fn a_spec_change_mid_run_rebuilds_what_it_touched() {
    // U-002 is Verified and U-001 still building when both are rewritten
    let r = rig();
    let bg = r.start(
        &["run", "--cap", "2"],
        &[("STUB_HANG_UNLESS", "U-001:notarized")],
    );
    wait_for("U-002 to land while U-001's worker runs", || {
        running_worker(&r.run_json(), "U-001").is_some()
            && bg.stdout().contains("integrated U-002 (c1)")
    });
    let (pid, _) = running_worker(&r.run_json(), "U-001").unwrap();

    // a Verified unit's text changes: its recorded trace no longer matches,
    // so it is marked Stale (with the cause) and built again
    amend_requirement(
        &r,
        "specs/mini/units/U-002-two.md",
        "with the frozen content",
        "with the frozen content, dated",
        "mini CH-001: U-002.R1 says dated",
    );
    wait_for("U-002 to be rebuilt", || {
        bg.stdout().contains("integrated U-002 (c2)")
    });
    let so_far = bg.stdout();
    assert!(
        so_far.contains(
            "stale U-002: recorded trace differs from current content: unit — it will be rebuilt"
        ),
        "{so_far}"
    );
    assert!(so_far.contains("dispatch U-002 (c2)"), "{so_far}");

    // a unit still being built is rewritten: its worker is building the
    // wrong thing — stopped, and the unit built from the current text
    assert!(group_alive(pid));
    amend_requirement(
        &r,
        "specs/mini/units/U-001-one.md",
        "with the frozen content",
        "with the frozen content, notarized",
        "mini CH-002: U-001.R1 says notarized",
    );
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains(
            "U-001: its inputs changed on main while it was being built — stopping the worker"
        ),
        "{stderr}"
    );
    assert!(!group_alive(pid));
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(stdout.contains("Complete."), "{stdout}");

    // the staleness is on the record, and the record is consistent
    let journal = r.unit_on_main("specs/mini/journal.ndjson");
    assert!(
        journal.contains(r#""event":"stale","unit":"U-002","actor":"audit","cause":"recorded trace differs from current content: unit""#),
        "{journal}"
    );
    let (code, stdout, stderr) = r.run(&["journal", "replay"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    // the rebuilt worker was told why
    let job = r.run_json()["job_id"].as_str().unwrap().to_string();
    let prompt = std::fs::read_to_string(
        r.root
            .join(".vitrify/logs")
            .join(&job)
            .join(format!("worker-{job}-w3.prompt.md")),
    )
    .unwrap();
    assert!(
        prompt.contains("This unit was built and verified before, then marked Stale"),
        "{prompt}"
    );
}

#[test]
fn a_change_entry_rebuilds_only_what_it_touched_and_the_run_is_complete() {
    // built at 1.0.0
    let r = rig();
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // a change entry: the SDD moves to 1.1.0 and U-002's text changes;
    // U-001's reading set is untouched
    let sdd = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd).unwrap();
    assert!(text.contains("version: 1.0.0"));
    std::fs::write(&sdd, text.replace("version: 1.0.0", "version: 1.1.0")).unwrap();
    amend_requirement(
        &r,
        "specs/mini/units/U-002-two.md",
        "with the frozen content",
        "with the frozen content, dated",
        "mini CH-001: U-002.R1 says dated (1.1.0)",
    );
    // the entry's staleness, applied and committed before the build resumes
    let (_, stdout, stderr) = r.run(&["audit", "--apply-stale"], &[]);
    assert!(
        std::fs::read_to_string(r.root.join("specs/mini/units/U-002-two.md"))
            .unwrap()
            .contains("status: Stale"),
        "{stdout}{stderr}"
    );
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "mini CH-001: staleness applied"],
    );

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("dispatch U-002 (c2)"), "{stdout}");
    assert!(!stdout.contains("dispatch U-001"), "{stdout}");

    // U-001 was not rebuilt: it keeps the stamp of the version it was
    // built at, and the build is complete all the same — with the suite run
    let one = r.unit_on_main("specs/mini/units/U-001-one.md");
    assert!(one.contains("status: Verified"), "{one}");
    assert!(one.contains("1.0.0"), "{one}");
    assert!(!one.contains("1.1.0"), "{one}");
    let two = r.unit_on_main("specs/mini/units/U-002-two.md");
    assert!(two.contains("status: Verified"), "{two}");
    assert!(two.contains("1.1.0"), "{two}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(!stdout.contains("Blocked."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    assert!(stdout.contains("§5 Test: exit 0 (true)"), "{stdout}");
}

// ------------------------------------------------------ rows side by side

#[test]
fn verification_rows_run_side_by_side_unless_told_otherwise() {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let r = rig();
    // nothing configured: as many rows at once as the machine has cores —
    // the rule `cargo test` uses for its test threads. A number is that
    // number, 1 the opt-out.
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains(&format!("· gate jobs {cores} (one per core) ·")),
        "{stdout}"
    );
    let (_, stdout, _) = r.run(&["run", "--dry-run", "--gate-jobs", "1"], &[]);
    assert!(stdout.contains("· gate jobs 1 ·"), "{stdout}");
    let (_, stdout, _) = r.run(&["run", "--dry-run"], &[("VITRIFY_RUN_GATE_JOBS", "3")]);
    assert!(stdout.contains("· gate jobs 3 ·"), "{stdout}");
    if cores < 2 {
        return; // one core: nothing can overlap
    }

    // U-001 gets two rows, each of which passes only while the other is
    // running: one at a time, the first would wait for a marker that never
    // comes
    let row = |me: &str, other: &str| {
        format!(
            "sh -c 'touch ../{me}.here; n=0; until [ -e ../{other}.here ]; do n=$((n+1)); [ $n -gt 50 ] && exit 1; sleep 0.1; done'"
        )
    };
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    let single = "| R1 | `test -f src/one.txt` | exit 0 |";
    assert!(text.contains(single), "{text}");
    std::fs::write(
        &unit,
        text.replace(
            single,
            &format!(
                "| R1 | `{}` | exit 0 |\n| R1 | `{}` | exit 0 |",
                row("a", "b"),
                row("b", "a")
            ),
        ),
    )
    .unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "two rows that need each other"],
    );

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stderr.contains("gate red"), "{stderr}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
}

// --------------------------------------------------------- the whole suite

#[test]
fn the_build_is_complete_only_when_the_test_command_passes() {
    let r = rig();
    let sdd_path = r.root.join("specs/mini/sdd.md");
    let original = std::fs::read_to_string(&sdd_path).unwrap();
    std::fs::write(
        &sdd_path,
        original.replace("- Test:      `true`", "- Test:      `false`"),
    )
    .unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "a suite that never passes"],
    );

    // it never gated the start, and — failing from the start — judged no
    // merge; but the build is not Complete on a red suite
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Blocked."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    assert!(
        stdout.contains("§5 Test failing at the main head: `false`"),
        "{stdout}"
    );
    assert!(
        stderr.contains("the §5 Test command already fails at the main head"),
        "{stderr}"
    );
    assert_eq!(r.run_json()["terminated"]["state"], "blocked");

    // no Test command: nothing to hold the build to
    let r = rig();
    let sdd_path = r.root.join("specs/mini/sdd.md");
    std::fs::write(
        &sdd_path,
        original.replace("- Test:      `true`", "- Test:      none"),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "no suite"]);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(!stdout.contains("§5 Test"), "{stdout}");
}

// ------------------------------------------------------------------- origin

#[test]
fn a_run_publishes_through_origin_when_there_is_one() {
    // with a remote, every mainline commit of the run — claims, the batch
    // integration, releases — is published by push, and origin arbitrates
    let r = rig();
    let origin = r._tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &r.root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&r.root, &["push", "-q", "origin", "main"]);

    // U-001's first attempt is rejected by the gate: the release, the
    // retry's claim and its integration all go through origin too
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(
        &unit,
        text.replace("`test -f src/one.txt`", "`grep -qx one src/one.txt`"),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "judge the content"]);
    git(&r.root, &["push", "-q", "origin", "main"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[("STUB_DRAFT_UNIT", "U-001")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Remote: origin"), "{stdout}");
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");

    // origin holds the whole build; the local branch and checkout followed
    for unit in ["U-001-one", "U-002-two"] {
        let text = git(
            &origin,
            &["show", &format!("main:specs/mini/units/{unit}.md")],
        );
        assert!(text.contains("status: Verified"), "{unit}: {text}");
    }
    assert_eq!(
        git(&r.root, &["rev-parse", "main"]),
        git(&origin, &["rev-parse", "main"])
    );
    assert_eq!(
        std::fs::read_to_string(r.root.join("src/one.txt")).unwrap(),
        "one\n"
    );
    assert!(git(&r.root, &["status", "--porcelain"]).is_empty());
    let (code, stdout, stderr) = r.run(&["audit"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
}

#[test]
fn preflight_refuses_local_commits_origin_does_not_hold() {
    // with a remote the run builds origin's head; a spec change committed
    // locally and not pushed would be read by preflight and built by nobody
    let r = rig();
    let origin = r._tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &r.root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&r.root, &["push", "-q", "origin", "main"]);
    amend_requirement(
        &r,
        "specs/mini/units/U-002-two.md",
        "with the frozen content",
        "with the frozen content, dated",
        "mini: U-002.R1 says dated (not pushed)",
    );

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("the local main holds commits origin/main does not"),
        "{stderr}"
    );
    assert!(!stdout.contains("dispatch"), "{stdout}");
    // nothing was published
    assert_eq!(git(&origin, &["log", "-1", "--format=%s", "main"]), "seed");

    // once pushed, the same run builds
    git(&r.root, &["push", "-q", "origin", "main"]);
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
}

#[test]
fn a_run_from_a_build_branch_publishes_there_and_resumes_there() {
    // origin/HEAD names main, as in any clone; a run started from a checkout
    // on a build branch, with no --mainline, builds on that branch — and so
    // does the run after it — while main is left as it was
    let r = rig();
    let origin = r._tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &r.root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&r.root, &["push", "-q", "origin", "main"]);
    git(&r.root, &["remote", "set-head", "origin", "main"]);
    let main_was = git(&r.root, &["rev-parse", "main"]);
    git(&r.root, &["checkout", "-q", "-b", "build/mini"]);
    git(&r.root, &["push", "-q", "-u", "origin", "build/mini"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("Mainline: build/mini   Remote: origin"),
        "{stdout}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");

    // the build is on the build branch, at origin and locally, and the
    // checkout followed it; main moved nowhere
    for unit in ["U-001-one", "U-002-two"] {
        let text = git(
            &origin,
            &["show", &format!("build/mini:specs/mini/units/{unit}.md")],
        );
        assert!(text.contains("status: Verified"), "{unit}: {text}");
    }
    assert_eq!(
        git(&r.root, &["rev-parse", "build/mini"]),
        git(&origin, &["rev-parse", "build/mini"])
    );
    assert_eq!(
        std::fs::read_to_string(r.root.join("src/one.txt")).unwrap(),
        "one\n"
    );
    assert!(git(&r.root, &["status", "--porcelain"]).is_empty());
    assert_eq!(git(&origin, &["rev-parse", "main"]), main_was);
    assert_eq!(git(&r.root, &["rev-parse", "main"]), main_was);

    // the next run resolves the same branch: nothing to build there, and
    // main — whose units are all still Ready — is not started on
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Mainline: build/mini"), "{stdout}");
    assert!(!stdout.contains("dispatch"), "{stdout}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert_eq!(git(&origin, &["rev-parse", "main"]), main_was);
}

#[test]
fn a_run_that_publishes_through_origin_never_falls_back_to_the_local_branch() {
    // A run that began by pushing holds every later commit to that. When
    // origin stops answering, a claim or a release fails — it is not put on
    // a local branch nobody else sees.
    let r = rig();
    let origin = r._tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &r.root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&r.root, &["push", "-q", "origin", "main"]);

    let bg = r.start(&["run", "--cap", "2"], &[("STUB_HANG_UNIT", "U-001")]);
    wait_for("U-002 to land while U-001's worker runs", || {
        running_worker(&r.run_json(), "U-001").is_some()
            && bg.stdout().contains("integrated U-002 (c1)")
    });
    // origin goes away; the run is stopped and tries to release U-001
    let gone = r._tmp.path().join("gone.git");
    std::fs::rename(&origin, &gone).unwrap();
    bg.signal("-TERM");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(15),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("release of U-001 failed"), "{stderr}");
    assert!(!stderr.contains("arbitrating locally"), "{stderr}");
    // the local branch holds nothing origin does not
    assert_eq!(
        git(&r.root, &["rev-parse", "main"]),
        git(&gone, &["rev-parse", "main"])
    );
    // the work is kept all the same
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "sdd/mini/U-001/c1"]),
        "sdd: abandoned attempt U-001 (c1) — interrupted"
    );
}

// ------------------------------------------- the checkout the run started in

#[test]
fn work_in_progress_in_the_primary_checkout_rides_along_with_a_run() {
    let r = rig();
    std::fs::write(r.root.join("NOTES.md"), "notes\n").unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "notes"]);
    // an uncommitted edit, there for the whole run
    std::fs::write(r.root.join("NOTES.md"), "notes, being edited\n").unwrap();

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stderr.contains("could not follow"), "{stderr}");
    // the checkout followed every commit of the run…
    for (file, content) in [("src/one.txt", "one\n"), ("src/two.txt", "two\n")] {
        assert_eq!(std::fs::read_to_string(r.root.join(file)).unwrap(), content);
    }
    assert!(
        std::fs::read_to_string(r.root.join("specs/mini/units/U-001-one.md"))
            .unwrap()
            .contains("status: Verified")
    );
    // …and the edit is the one thing in it that differs from the head
    assert_eq!(git(&r.root, &["status", "--porcelain"]), "M NOTES.md");
    assert_eq!(
        std::fs::read_to_string(r.root.join("NOTES.md")).unwrap(),
        "notes, being edited\n"
    );
}

#[test]
fn a_file_of_the_user_s_in_the_primary_checkout_is_never_overwritten() {
    // an untracked file exactly where the build will put one
    let r = rig();
    let theirs = r.root.join("src/one.txt");
    std::fs::create_dir_all(r.root.join("src")).unwrap();
    std::fs::write(&theirs, "mine, not committed\n").unwrap();
    let marker = r.root.join(".vitrify/primary-behind");

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    // mainline has what was built; the user's file is where and what it was
    assert_eq!(r.unit_on_main("src/one.txt"), "one");
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "mine, not committed\n"
    );
    // the run says so when it happens, and again as its last word
    assert!(
        stderr.contains("Untracked working tree file 'src/one.txt' would be overwritten"),
        "{stderr}"
    );
    assert_eq!(
        stderr
            .matches("warning: the primary checkout could not follow main")
            .count(),
        2,
        "{stderr}"
    );
    assert!(stderr.contains("commit nothing from it"), "{stderr}");
    assert!(marker.exists());

    // a dry run says where things stand, and touches nothing
    let (code, stdout, stderr) = r.run(&["run", "--dry-run"], &[]);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("note: the primary checkout is behind main"),
        "{stderr}"
    );
    assert!(
        stderr.contains("preflight failed: the primary checkout is behind main"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "mine, not committed\n"
    );

    // the file moved aside (out of the SDD's scope roots, where every file
    // is governed): the next run brings the checkout up to date before it
    // reads it
    std::fs::rename(&theirs, r.root.join("one.mine.txt")).unwrap();
    let (code, stdout, stderr) = r.run(&["run"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "one\n");
    assert_eq!(git(&r.root, &["status", "--porcelain"]), "?? one.mine.txt");
    assert!(!marker.exists());
}

#[test]
fn a_run_started_in_a_linked_worktree_builds_mainline_and_leaves_that_worktree_alone() {
    let r = rig();
    let side = r._tmp.path().canonicalize().unwrap().join("side");
    git(
        &r.root,
        &[
            "worktree",
            "add",
            "-q",
            side.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    // `--sdd` as seen from where the verb runs
    let out = r
        .command(&["run", "--cap", "2", "--sdd", "specs/mini"], &[])
        .current_dir(&side)
        .output()
        .unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    // built on mainline, and the primary checkout followed
    for unit in ["U-001-one", "U-002-two"] {
        let text = r.unit_on_main(&format!("specs/mini/units/{unit}.md"));
        assert!(text.contains("status: Verified"), "{unit}: {text}");
    }
    assert_eq!(
        std::fs::read_to_string(r.root.join("src/one.txt")).unwrap(),
        "one\n"
    );
    // nothing in the worktree the verb ran in was edited or added
    assert!(git(&side, &["status", "--porcelain"]).is_empty());
    assert!(
        std::fs::read_to_string(side.join("specs/mini/units/U-001-one.md"))
            .unwrap()
            .contains("status: Ready")
    );
    assert!(!side.join("src").exists());
    assert!(!side.join(".vitrify").exists());
}

// ------------------------------------------------- contracts across SDDs

/// The stub's solution tree and unit map for one SDD of a multi-SDD fixture.
fn fixture_env(fixture: &str, which: &str) -> [(&'static str, String); 2] {
    let fixture = repo_root().join("fixtures/seeded").join(fixture);
    [
        ("STUB_REF", fixture.join("solution").display().to_string()),
        (
            "STUB_MAP",
            fixture
                .join(format!("stub-map-{which}.toml"))
                .display()
                .to_string(),
        ),
    ]
}

/// The prompt the last worker dispatched for `unit` was started with.
fn last_prompt(r: &Rig, unit: &str) -> String {
    let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
    for job in std::fs::read_dir(r.root.join(".vitrify/logs"))
        .unwrap()
        .flatten()
    {
        for entry in std::fs::read_dir(job.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".prompt.md") {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            if text.contains(&format!("unit: {unit}\n")) {
                found.push((entry.metadata().unwrap().modified().unwrap(), text));
            }
        }
    }
    found.sort_by_key(|(at, _)| *at);
    found.pop().map(|(_, text)| text).expect("a prompt")
}

fn edit(r: &Rig, rel: &str, from: &str, to: &str) {
    let path = r.root.join(rel);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{rel} has no `{from}`");
    std::fs::write(&path, text.replace(from, to)).unwrap();
}

#[test]
fn a_stale_pin_refuses_the_run_until_it_is_synced() {
    let r = rig_of("pin-states");
    // the producer has nothing out of step
    let (code, _, stderr) = r.run(&["run", "--sdd", "specs/alpha", "--dry-run"], &[]);
    assert_eq!(code, 0, "{stderr}");

    // the consumer: one pin is behind its authority, one names an
    // authority that is not there — preflight refuses, naming each
    let (code, _, stderr) = r.run(&["run", "--sdd", "specs/beta", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: lint [L9] C-002: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97)"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "preflight failed: lint [L9] C-003: its authority cannot be found: alpha has no contract C-009"
        ),
        "{stderr}"
    );

    // sync what can be synced; drop the contract whose authority is gone
    let (code, stdout, _) = r.run(&["pin", "sync", "--sdd", "specs/beta"], &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(
        stdout.contains("synced beta C-002 ← alpha C-002"),
        "{stdout}"
    );
    std::fs::remove_file(r.root.join("specs/beta/contracts/C-003-ledger.md")).unwrap();
    edit(
        &r,
        "specs/beta/units/U-002-two.md",
        "consumes: [C-003, C-004]",
        "consumes: [C-004]",
    );
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "beta: pins synced"]);

    let beta = fixture_env("pin-states", "beta");
    let (code, stdout, stderr) = r.run(
        &["run", "--sdd", "specs/beta", "--cap", "2"],
        &borrowed(&beta),
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    // U-001's worker read through the synced pin: the producer's rule is in
    // its prompt
    let prompt = last_prompt(&r, "U-001");
    assert!(
        prompt.contains("- IF the file exists, THEN THE SYSTEM SHALL replace it whole."),
        "{prompt}"
    );

    // The producer moves again. Nothing in beta's own files has changed, so
    // its units still match their traces and stay Verified — which is why
    // the next run has to refuse rather than report a finished build.
    edit(
        &r,
        "specs/alpha/contracts/C-002-farewell.md",
        "replace it whole.\n",
        "replace it whole.\n- THE SYSTEM SHALL end the line with a newline.\n",
    );
    git(
        &r.root,
        &["commit", "-q", "-am", "alpha: C-002 gains a rule"],
    );
    let (code, _, _) = r.run(&["audit", "--sdd", "specs/beta", "--skip-rotten"], &[]);
    assert_eq!(code, 0, "the audit sees one SDD and finds it consistent");
    let (code, _, stderr) = r.run(&["run", "--sdd", "specs/beta", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("preflight failed: lint [L9] C-002"),
        "{stderr}"
    );

    // The sync delivers the change to beta's staleness, names the unit for
    // the change entry and the step after it; the marks are applied, and
    // the run rebuilds exactly what the producer's edit reached.
    let (code, stdout, stderr) = r.run(&["pin", "sync", "--sdd", "specs/beta"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("beta: - Staleness applied: U-001 → Stale (consume C-002)"),
        "{stdout}"
    );
    assert!(
        stderr.contains(
            "next: write beta's change entry around its line, then run vitrify change apply --sdd specs/beta"
        ),
        "{stderr}"
    );
    let (code, stdout, _) = r.run(
        &[
            "audit",
            "--sdd",
            "specs/beta",
            "--skip-rotten",
            "--apply-stale",
        ],
        &[],
    );
    assert_eq!(code, 2, "the mismatch is reported as it is marked");
    assert!(
        stdout.contains("[check 5] U-001: recorded trace differs from current content: C-002"),
        "{stdout}"
    );
    git(&r.root, &["add", "-A"]);
    git(
        &r.root,
        &["commit", "-q", "-m", "beta: C-002 synced; U-001 stale"],
    );
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/beta"], &borrowed(&beta));
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("dispatch U-001 (c2)"), "{stdout}");
    assert!(!stdout.contains("dispatch U-002"), "{stdout}");
    assert!(
        last_prompt(&r, "U-001").contains("- THE SYSTEM SHALL end the line with a newline."),
        "the rebuilt unit read the new rule"
    );
}

#[test]
fn a_pinned_contract_is_read_through_its_pin_and_a_moved_authority_sets_its_unit_aside() {
    let r = rig_of("pin-multi");
    // the fixture's second pin is behind its authority
    let (code, _, stderr) = r.run(&["run", "--sdd", "specs/beta", "--dry-run"], &[]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(
            "preflight failed: lint [L9] C-002: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97)"
        ),
        "{stderr}"
    );
    let (code, stdout, _) = r.run(&["pin", "sync", "--sdd", "specs/beta"], &[]);
    assert_eq!(code, 0, "{stdout}");
    git(&r.root, &["commit", "-q", "-am", "beta: C-002 re-pinned"]);

    // U-001's worker holds until its spec says "notarized"; U-002 waits on it
    let beta = fixture_env("pin-multi", "beta");
    let mut env = borrowed(&beta);
    env.push(("STUB_HANG_UNLESS", "U-001:notarized"));
    let bg = r.start(&["run", "--sdd", "specs/beta", "--wait"], &env);
    wait_for("U-001's worker to be running", || {
        running_worker(&r.run_json(), "U-001").is_some()
    });
    // its prompt holds the pinned contract's own entry, and then the body
    // the pin stands for — read from the producer's file, header left out
    let prompt = last_prompt(&r, "U-001");
    assert!(
        prompt.contains("- Pin: alpha C-001 @ ef9dc34b4daf"),
        "{prompt}"
    );
    assert!(
        prompt.contains("specs/alpha/contracts/C-001-greeting.md  ::  Surface onward\n\n## Surface\n\n```text\n# a/one.txt — the greeting file\nalpha one\n```"),
        "{prompt}"
    );
    assert!(!prompt.contains("- Producer: U-001"), "{prompt}");

    // The producer's C-002 changes while beta is being built: beta's pin on
    // it no longer matches. Nothing of beta's own moved, so nothing is
    // staled and U-001's worker is left alone.
    edit(
        &r,
        "specs/alpha/contracts/C-002-farewell.md",
        "replace it whole.\n",
        "replace it whole.\n- THE SYSTEM SHALL end the line with a newline.\n",
    );
    git(
        &r.root,
        &["commit", "-q", "-am", "alpha: C-002 gains a rule"],
    );
    amend_requirement(
        &r,
        "specs/beta/units/U-001-one.md",
        "with the frozen content",
        "with the frozen content, notarized",
        "beta: U-001.R1 says notarized",
    );

    // U-001 lands. U-002 cannot be handed to a worker: the body its pin
    // stands for is not the body there now. It is set aside — claim given
    // back, reason named — and the run goes on waiting, not down.
    wait_for("the run to wait on the stale pin", || {
        bg.stdout()
            .contains("waiting: the run resumes when the spec changes")
    });
    let so_far = bg.stdout();
    assert!(so_far.contains("integrated U-001 (c2)"), "{so_far}");
    assert!(
        so_far.contains(
            "U-002: its reading set does not resolve: pinned contract C-002 does not resolve: its pin 28c9da8acc97 is not the hash of alpha C-002's body"
        ),
        "{so_far}"
    );
    assert!(
        bg.stderr()
            .contains("U-002 (c1): not started — pinned contract C-002 does not resolve"),
        "{}",
        bg.stderr()
    );
    let u2 = r.unit_on_main("specs/beta/units/U-002-two.md");
    assert!(
        u2.contains("status: Ready") && u2.contains("claim_seq: 1"),
        "{u2}"
    );
    assert!(
        git(&r.root, &["branch", "--list", "sdd/beta/U-002/*"]).is_empty(),
        "nothing was built, so nothing was kept"
    );

    // re-pin in the primary checkout — it has followed the run — and commit:
    // the unit's reading set has changed, and it is built from the new body
    let (code, stdout, stderr) = r.run(&["pin", "sync", "--sdd", "specs/beta"], &[]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md"),
        "{stdout}"
    );
    git(
        &r.root,
        &["commit", "-q", "-am", "beta: C-002 re-pinned again"],
    );

    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("U-002: its reading set changed — back in the pool"),
        "{stdout}"
    );
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 2/2 Verified"), "{stdout}");
    assert!(
        last_prompt(&r, "U-002").contains("- THE SYSTEM SHALL end the line with a newline."),
        "U-002 was built from the body its new pin stands for"
    );
    // the trace it was verified against covers the pin, and so the body
    let (code, stdout, _) = r.run(
        &["trace", "verify", "--unit", "U-002", "--sdd", "specs/beta"],
        &[],
    );
    assert_eq!(code, 0, "{stdout}");
}

#[test]
fn a_single_file_sdd_builds_through_its_pin() {
    let r = rig_of("pin-multi");
    let gamma = fixture_env("pin-multi", "gamma");
    let (code, stdout, stderr) = r.run(&["run", "--sdd", "specs/gamma.sdd.md"], &borrowed(&gamma));
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Shape: single"), "{stdout}");
    assert!(stdout.contains("Progress: 1/1 Verified"), "{stdout}");
    let prompt = last_prompt(&r, "U-001");
    assert!(
        prompt.contains("specs/gamma.sdd.md  ::  Interface Contracts / C-001\n\n### C-001 — Greeting file (pinned)"),
        "{prompt}"
    );
    assert!(
        prompt
            .contains("specs/alpha/contracts/C-001-greeting.md  ::  Surface onward\n\n## Surface"),
        "{prompt}"
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
