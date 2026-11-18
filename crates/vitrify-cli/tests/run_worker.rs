//! Worker spawn mechanics (process group, log capture),
//! all four stub-worker modes, report extraction (envelope, stream, raw),
//! missing-binary refusal.

use std::path::{Path, PathBuf};
use std::process::Command;

use std::sync::Mutex;

use vitrify_cli::run::worker;

/// Serializes tests that mutate process env (STUB_* is inherited by spawns).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn stub_bin() -> String {
    // stub-worker lives in another workspace crate: resolve it next to the
    // vitrify binary, building it on first use.
    let vit = PathBuf::from(env!("CARGO_BIN_EXE_vitrify"));
    let stub = vit.parent().unwrap().join("stub-worker");
    if !stub.exists() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "stub-worker", "--quiet"])
            .status()
            .unwrap();
        assert!(status.success(), "failed to build stub-worker");
    }
    stub.to_string_lossy().to_string()
}

fn vitrify_bin() -> String {
    env!("CARGO_BIN_EXE_vitrify").to_string()
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

/// calc repo with U-002 claimed and a work worktree on its branch.
struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    /// the home `vitrify init` set up, which the worker's own `vitrify`
    /// calls need too
    home: PathBuf,
    worktree: PathBuf,
    prompt: String,
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    copy_dir(&repo_root().join("fixtures/example"), &root);
    let root = root.canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);

    let home = tmp.path().join("vhome");
    set_up_home(&home);
    let run = |args: &[&str]| {
        let out = Command::new(vitrify_bin())
            .args(args)
            .current_dir(&root)
            .env("VITRIFY_HOME", &home)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "vitrify {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let claim: serde_json::Value =
        serde_json::from_str(&run(&["claim", "--unit", "U-002", "--holder", "w1"])).unwrap();
    let commit = claim["commit"].as_str().unwrap().to_string();
    let worktree = root.join(".vitrify/worktrees/U-002-c1");
    std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
    git(
        &root,
        &[
            "worktree",
            "add",
            worktree.to_str().unwrap(),
            "-b",
            "sdd/U-002/c1",
            &commit,
        ],
    );
    let prompt = run(&["brief", "build-unit", "--unit", "U-002", "--holder", "w1"]);
    Rig {
        root,
        home,
        worktree: worktree.canonicalize().unwrap(),
        prompt,
        _tmp: tmp,
    }
}

fn wait_and_read(handle: &mut worker::WorkerHandle) -> (bool, String) {
    let status = handle.child.wait().unwrap();
    let out = std::fs::read_to_string(&handle.out_path).unwrap_or_default();
    (status.success(), out)
}

#[test]
fn build_mode_builds_stamps_and_reports_a_dirty_handoff() {
    let _guard = ENV_LOCK.lock().unwrap();
    let r = rig();
    // solution tree: the example's reference implementation for calc/errors.py
    let solution = r.root.join("solution-tree");
    std::fs::create_dir_all(solution.join("calc")).unwrap();
    std::fs::copy(
        repo_root().join("fixtures/example/reference/errors.py"),
        solution.join("calc/errors.py"),
    )
    .unwrap();
    let map = r.root.join("stub-map.toml");
    std::fs::write(&map, "[units]\nU-002 = [\"calc/errors.py\"]\n").unwrap();

    let logs = r.root.join(".vitrify/logs/test");
    // environment is inherited: set the stub controls on our process
    unsafe {
        std::env::set_var("STUB_MODE", "build");
        std::env::set_var("STUB_REF", &solution);
        std::env::set_var("STUB_MAP", &map);
        std::env::set_var("VITRIFY_BIN", vitrify_bin());
        std::env::set_var("VITRIFY_HOME", &r.home);
    }
    let mut handle =
        worker::spawn(&[stub_bin()], &r.prompt, &r.worktree, &logs, "w1", "U-002").unwrap();
    let (ok, out) = wait_and_read(&mut handle);
    assert!(
        ok,
        "stub stderr: {}",
        std::fs::read_to_string(&handle.err_path).unwrap_or_default()
    );

    let report = worker::extract_report(&out).expect("report line");
    assert_eq!(report.outcome, "built");
    assert_eq!(report.unit, "U-002");
    // the job block's branch, verbatim: it carries the SDD's slug
    assert_eq!(report.branch, "sdd/calc/U-002/c1");
    assert_eq!(report.claim_seq, 1);

    // the worktree carries the artifact and the Built stamp, left dirty —
    // workers never commit; the handoff commit is the integrator's act
    assert!(r.worktree.join("calc/errors.py").exists());
    let unit_text =
        std::fs::read_to_string(r.worktree.join("specs/calc/units/U-002-errors.md")).unwrap();
    assert!(unit_text.contains("status: Built"));
    assert!(unit_text.contains("trace:\n"));
    let subject = git(&r.worktree, &["log", "-1", "--format=%s"]);
    assert_eq!(subject, "sdd: claim U-002 (c1) by w1");
    let dirty = git(&r.worktree, &["status", "--porcelain"]);
    assert!(!dirty.is_empty(), "handoff worktree should be dirty");
    unsafe {
        std::env::remove_var("STUB_MODE");
        std::env::remove_var("STUB_REF");
        std::env::remove_var("STUB_MAP");
        std::env::remove_var("VITRIFY_HOME");
    }
}

#[test]
fn block_mode_reports_blocked_with_q() {
    let _guard = ENV_LOCK.lock().unwrap();
    let r = rig();
    let logs = r.root.join(".vitrify/logs/test");
    unsafe {
        std::env::set_var("STUB_MODE", "block");
    }
    let mut handle =
        worker::spawn(&[stub_bin()], &r.prompt, &r.worktree, &logs, "w2", "U-002").unwrap();
    let (ok, out) = wait_and_read(&mut handle);
    assert!(ok);
    let report = worker::extract_report(&out).unwrap();
    assert_eq!(report.outcome, "blocked");
    assert_eq!(report.q.as_deref(), Some("Q-999"));
    unsafe {
        std::env::remove_var("STUB_MODE");
    }
}

#[test]
fn garbage_mode_falls_back_to_released_unparseable() {
    let out = "this is not a report\nstill not json\n";
    let (report, parsed) = worker::report_or_released(out, "U-007");
    assert!(!parsed);
    assert_eq!(report.outcome, "released");
    assert_eq!(report.notes, "unparseable report");
    assert_eq!(report.unit, "U-007");
}

#[test]
fn hang_mode_killed_by_group_kill() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let logs = tmp.path().join("logs");
    unsafe {
        std::env::set_var("STUB_MODE", "hang");
    }
    let mut handle = worker::spawn(
        &[stub_bin()],
        "no job block",
        tmp.path(),
        &logs,
        "wh",
        "U-001",
    )
    .unwrap();
    unsafe {
        std::env::remove_var("STUB_MODE");
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        handle.child.try_wait().unwrap().is_none(),
        "should be hanging"
    );
    worker::kill_group(handle.pid);
    let status = handle.child.wait().unwrap();
    assert!(!status.success());
    // the process group is gone: kill -0 on the group fails
    let probe = Command::new("/bin/kill")
        .args(["-0", "--", &format!("-{}", handle.pid)])
        .status()
        .unwrap();
    assert!(!probe.success(), "process group still alive");
}

#[test]
fn extraction_envelope_stream_and_raw() {
    let line = r#"{"unit":"U-001","outcome":"built","branch":"sdd/U-001/c1","claim_seq":1,"q":null,"notes":"n"}"#;

    // raw stdout, last non-empty line
    let raw = format!("chatter\n{line}\n\n");
    assert_eq!(worker::extract_report(&raw).unwrap().outcome, "built");

    // the whole output one JSON envelope: a stream of one `result` event
    let envelope = serde_json::json!({"type": "result", "result": format!("preamble\n{line}")});
    let report = worker::extract_report(&envelope.to_string()).unwrap();
    assert_eq!(report.unit, "U-001");

    // NDJSON event stream: assistant text in the message content
    let stream = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"system","subtype":"init"}),
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text": line}]}}),
    );
    let report = worker::extract_report(&stream).unwrap();
    assert_eq!(report.claim_seq, 1);

    // invalid outcome rejected
    let bad =
        r#"{"unit":"U-001","outcome":"maybe","branch":"b","claim_seq":1,"q":null,"notes":""}"#;
    assert!(worker::extract_report(bad).is_none());

    // report wrapped in a markdown code fence — fence lines are not content
    let fenced = format!("prose about the build\n\n```json\n{line}\n```\n");
    assert_eq!(worker::extract_report(&fenced).unwrap().outcome, "built");
    let envelope =
        serde_json::json!({"type": "result", "result": format!("summary\n\n```json\n{line}\n```")});
    assert_eq!(
        worker::extract_report(&envelope.to_string()).unwrap().unit,
        "U-001"
    );
}

#[test]
fn a_stream_cut_short_or_interleaved_still_yields_its_report() {
    let line = r#"{"unit":"U-001","outcome":"built","branch":"sdd/x/U-001/c1","claim_seq":1,"q":null,"notes":"n"}"#;
    let result = serde_json::json!({"type":"result","subtype":"success","result": format!("done\n\n{line}")});
    let event = serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}});

    // a line that is not an event among the events (a stray warning on
    // stdout): the stream is still read as one
    let noisy = format!("{event}\nWarning: something on stdout\n{result}\n");
    assert_eq!(worker::extract_report(&noisy).unwrap().unit, "U-001");

    // the final write cut off mid-line after the result event
    let cut = format!("{event}\n{result}\n{{\"type\":\"system\",\"sub");
    assert_eq!(worker::extract_report(&cut).unwrap().outcome, "built");

    // a session killed before it reported: the log is kept, there is no
    // report — and none is invented
    let killed = format!("{event}\n{{\"type\":\"assistant\",\"mess");
    assert!(worker::extract_report(&killed).is_none());
    let (report, parsed) = worker::report_or_released(&killed, "U-001");
    assert!(!parsed);
    assert_eq!(report.outcome, "released");
}

#[test]
fn a_worker_gets_the_private_tmpdir_it_is_given() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let private = tmp.path().join("private");
    std::fs::create_dir_all(&private).unwrap();
    unsafe {
        std::env::set_var("STUB_MODE", "garbage");
    }
    let mut handle = worker::spawn_in(
        &[stub_bin()],
        "no job block",
        tmp.path(),
        &tmp.path().join("logs"),
        "wt",
        "U-001",
        Some(&private),
    )
    .unwrap();
    unsafe {
        std::env::remove_var("STUB_MODE");
    }
    handle.child.wait().unwrap();
    let err = std::fs::read_to_string(&handle.err_path).unwrap();
    assert!(
        err.contains(&format!("stub: TMPDIR={}", private.display())),
        "{err}"
    );
}

#[test]
fn missing_probed_binary_is_a_named_error() {
    let tmp = tempfile::tempdir().unwrap();
    let Err(err) = worker::spawn(
        &["/nonexistent/harness-bin".to_string()],
        "p",
        tmp.path(),
        &tmp.path().join("logs"),
        "w1",
        "U-001",
    ) else {
        panic!("spawn should fail for a missing binary");
    };
    assert_eq!(err.exit_code(), 2);
    assert!(err.to_string().contains("probed worker binary not found"));
}

/// `vitrify init` with no harness CLI on PATH: the home is set up — every
/// setting at its default, the content materialized — and no model is
/// called.
fn set_up_home(home: &Path) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vitrify"))
        .arg("init")
        .env("VITRIFY_HOME", home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "vitrify init: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
