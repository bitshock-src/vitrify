//! The tier-2 end-to-end battery (`#[ignore]`; permanent CI stage).
//! Drives a full `vitrify run` build of the calc example (`fixtures/example/`)
//! with the deterministic stub worker: protocol reset → Complete with the
//! evidence triple, zero spec-body edits, and the kill-a-worker fencing
//! rehearsal. `schema_version: 2.0` behavior itself is pinned by
//! the golden corpus (`fixtures/golden/`) in the tier-1 suites.

use std::path::{Path, PathBuf};
use std::process::Command;

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

const UNIT_FILES: [&str; 5] = [
    "specs/calc/units/U-001-foundation.md",
    "specs/calc/units/U-002-errors.md",
    "specs/calc/units/U-003-tokenizer.md",
    "specs/calc/units/U-004-evaluator.md",
    "specs/calc/units/U-005-cli.md",
];

// ------------------------------------------------------------ tier-2 e2e

fn calc_stub_map(dir: &Path) -> PathBuf {
    let map = dir.join("stub-map.toml");
    std::fs::write(
        &map,
        "[units]\nU-001 = [\"calc/__init__.py\"]\nU-002 = [\"calc/errors.py\"]\nU-003 = [\"calc/tokenizer.py\"]\nU-004 = [\"calc/evaluator.py\"]\nU-005 = [\"calc/__main__.py\"]\n",
    )
    .unwrap();
    map
}

fn calc_solution(dir: &Path) -> PathBuf {
    let sol = dir.join("solution");
    let reference = repo_root().join("fixtures/example/reference");
    std::fs::create_dir_all(sol.join("calc")).unwrap();
    for (src, dst) in [
        ("errors.py", "errors.py"),
        ("tokenizer.py", "tokenizer.py"),
        ("evaluator.py", "evaluator.py"),
        ("__main__.py", "__main__.py"),
    ] {
        std::fs::copy(reference.join(src), sol.join("calc").join(dst)).unwrap();
    }
    // U-001's artifact comes from the example itself
    std::fs::copy(
        repo_root().join("fixtures/example/calc/__init__.py"),
        sol.join("calc/__init__.py"),
    )
    .unwrap();
    sol
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

struct CalcRig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    tmp_path: PathBuf,
}

fn calc_rig() -> CalcRig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&root).unwrap();
    copy_dir(&repo_root().join("fixtures/example"), &root);
    let root = root.canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    std::fs::create_dir_all(&home).unwrap();
    write_config(&home, &stub_bin());
    let tmp_path = tmp.path().to_path_buf();
    // the run's temp root: inside this rig, gone with it
    std::fs::create_dir_all(tmp_path.join("t")).unwrap();
    CalcRig {
        root,
        home,
        tmp_path,
        _tmp: tmp,
    }
}

impl CalcRig {
    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String, String) {
        let sol = calc_solution(&self.tmp_path);
        let map = calc_stub_map(&self.tmp_path);
        let mut c = Command::new(vitrify_bin());
        c.args(args)
            .current_dir(&self.root)
            .env("VITRIFY_HOME", &self.home)
            .env("STUB_REF", &sol)
            .env("STUB_MAP", &map)
            .env("VITRIFY_BIN", vitrify_bin())
            .env("STUB_MODE", "build")
            .env("TMPDIR", self.tmp_path.join("t"));
        for (k, v) in extra_env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }
}

fn spec_bodies(root: &Path) -> Vec<(String, String)> {
    // non-front-matter regions of every spec file (the zero-spec-edit check)
    let mut out = Vec::new();
    let mut stack = vec![root.join("specs")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == "ledger.md" || name == "journal.ndjson" || name == "decisions.md" {
                continue; // derived / protocol files
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let body = match text.strip_prefix("---\n").and_then(|r| r.find("\n---\n")) {
                Some(i) => text["---\n".len() + i + "\n---\n".len()..].to_string(),
                None => text,
            };
            out.push((
                p.strip_prefix(root).unwrap().to_string_lossy().to_string(),
                body,
            ));
        }
    }
    out.sort();
    out
}

/// Full calc generation via `vitrify run` + stub worker, Verification tables
/// live, terminating Complete with the evidence triple — and the
/// acceptance test: zero spec edits, evidence triple quoted.
#[test]
#[ignore]
fn calc_full_build_from_protocol_reset_to_complete() {
    let r = calc_rig();
    // (b) reset protocol fields only: statuses → Ready, stamps/traces/journal
    // cleared by scripted edit — zero edits to any spec body
    let before = spec_bodies(&r.root);
    for f in UNIT_FILES {
        let p = r.root.join(f);
        let text = std::fs::read_to_string(&p).unwrap();
        let mut out = Vec::new();
        let mut in_trace = false;
        for line in text.lines() {
            if in_trace && (line.starts_with(' ') || line.starts_with('\t')) {
                continue;
            }
            in_trace = false;
            let l = if let Some((k, _)) = line.split_once(':') {
                match k {
                    "status" => "status: Ready".to_string(),
                    "claimed_by" => "claimed_by: —".to_string(),
                    "claim_expires" => "claim_expires: —".to_string(),
                    "claim_seq" => "claim_seq: 0".to_string(),
                    "built" => "built: —".to_string(),
                    "verified" => "verified: —".to_string(),
                    "trace" => {
                        in_trace = true;
                        "trace: —".to_string()
                    }
                    _ => line.to_string(),
                }
            } else {
                line.to_string()
            };
            out.push(l);
        }
        std::fs::write(&p, out.join("\n") + "\n").unwrap();
    }
    std::fs::write(r.root.join("specs/calc/journal.ndjson"), "").unwrap();
    std::fs::remove_file(r.root.join("specs/calc/journal.ndjson")).unwrap();
    // regenerate the ledger for the reset state and commit
    let (code, _, stderr) = r.run(&["ledger", "regen"], &[]);
    assert_eq!(code, 0, "{stderr}");
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "protocol reset"]);

    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("Progress: 5/5 Verified"), "{stdout}");
    assert!(stdout.contains("audit: exit 0 (clean)"));
    assert!(stdout.contains("replay: exit 0 (consistent)"));

    // zero spec-body edits in either direction (the zero-spec-edit guarantee)
    let after = spec_bodies(&r.root);
    assert_eq!(before, after, "spec bodies changed during the build");

    // (a) on the fully-Verified tree, a second run preflights green and
    // terminates Complete immediately — no dispatch
    let (code, stdout, stderr) = r.run(&["run"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."));
    assert!(
        !stdout.contains("dispatch "),
        "second run must not dispatch"
    );
}

/// Kill-a-worker rehearsal: a built branch whose claim lapsed and was
/// re-claimed must be rejected at the gate; the re-claimed build lands.
#[test]
#[ignore]
fn kill_a_worker_stale_branch_rejected() {
    let r = calc_rig();
    let sol = calc_solution(&r.tmp_path);

    // worker 1 claims U-002 with an instantly-lapsing lease, builds its
    // branch green... and "dies" before reporting
    let (code, stdout, stderr) = r.run(
        &[
            "claim",
            "--unit",
            "U-002",
            "--holder",
            "dead-worker",
            "--lease-minutes",
            "0",
        ],
        &[],
    );
    assert_eq!(code, 0, "{stderr}");
    let claim: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let commit = claim["commit"].as_str().unwrap();
    use vitrify_cli::layout::Layout;
    let layout = Layout::resolve(&r.root, Some(&r.home), "").unwrap();
    let wt = layout.unit_worktree("calc", "U-002", 1);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        &r.root,
        &[
            "worktree",
            "add",
            wt.to_str().unwrap(),
            "-b",
            "sdd/calc/U-002/c1",
            commit,
        ],
    );
    std::fs::copy(sol.join("calc/errors.py"), wt.join("calc/errors.py")).unwrap();
    let out = Command::new(vitrify_bin())
        .args(["trace", "record", "--unit", "U-002", "--stamp", "built"])
        .current_dir(&wt)
        .env("VITRIFY_HOME", &r.home)
        .output()
        .unwrap();
    assert!(out.status.success());
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-q", "-m", "sdd: build U-002 (c1)"]);

    // the orchestrator reclaims the lapsed unit (new token c2) and builds it
    // — in its own worktree, on its own branch: the dead worker's are not
    // touched, only read (its files are the new attempt's starting point)
    let (code, stdout, stderr) = r.run(&["run", "--cap", "2"], &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("Complete."), "{stdout}");
    assert!(stdout.contains("dispatch U-002 (c2)"), "{stdout}");
    assert!(
        wt.join("calc/errors.py").exists(),
        "the dead worker's worktree was removed"
    );
    let u2 = git(&r.root, &["show", "main:specs/calc/units/U-002-errors.md"]);
    assert!(
        u2.contains("claim_seq: 2"),
        "reclaim must supersede c1:\n{u2}"
    );

    // the stale c1 branch is rejected at the gate even though it is green
    use vitrify_cli::git::Git;
    use vitrify_cli::run::integrate::{GateMemo, IntegrateRequest, Integrator, Outcome};
    use vitrify_cli::run::mainline::Mainline;
    let integrator = Integrator::new(
        Mainline {
            primary: Git::new(&r.root),
            layout,
            sdd_rel: PathBuf::from("specs/calc"),
            slug: "calc".to_string(),
            branch: "main".to_string(),
            push: false,
        },
        1,
    );
    let outcome = integrator
        .integrate(
            &IntegrateRequest {
                unit: "U-002".to_string(),
                token_seq: 1,
                builder_holder: "dead-worker".to_string(),
                integrator_id: "test".to_string(),
            },
            &mut GateMemo::default(),
            None,
        )
        .unwrap();
    let Outcome::RejectedFencing { reason } = outcome else {
        panic!("stale branch must be rejected, got {outcome:?}");
    };
    assert!(reason.contains("superseded token"), "{reason}");
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
