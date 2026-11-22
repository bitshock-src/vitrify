//! Edges across SDDs: `locate --graph` on a seeded project, and a pin in
//! `depends_on` gating `claim` and `locate --units` until the units that
//! produce its authority are Built.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
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

fn git(dir: &Path, args: &[&str]) {
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
}

/// A seeded project's `project/` tree committed on main in a temp repository.
fn project(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    copy_dir(
        &repo_root()
            .join("fixtures/seeded")
            .join(fixture)
            .join("project"),
        &root,
    );
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    (tmp, root)
}

fn vitrify(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vitrify"))
        .args(args)
        .current_dir(root)
        .env("VITRIFY_HOME", home())
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Set `unit`'s status in `file` and commit it on main.
fn set_status(root: &Path, file: &str, from: &str, to: &str) {
    let p = root.join(file);
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(
        &p,
        text.replacen(&format!("status: {from}"), &format!("status: {to}"), 1),
    )
    .unwrap();
    git(root, &["commit", "-q", "-am", &format!("{file}: {to}")]);
}

#[test]
fn locate_graph_shows_the_sdd_graph_and_the_unit_graph() {
    let (_tmp, root) = project("project-pipeline");
    let out = vitrify(&root, &["locate", "--graph"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    insta::assert_snapshot!("locate_graph_pipeline", stdout(&out));
    let out = vitrify(&root, &["locate", "--graph", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    insta::assert_snapshot!("locate_graph_pipeline_json", stdout(&out));
}

#[test]
fn locate_graph_names_every_cycle_and_the_units_behind_one() {
    let (_tmp, root) = project("project-cycle");
    let out = vitrify(&root, &["locate", "--graph"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("alpha → beta  via C-002\n"), "{text}");
    assert!(text.contains("beta → alpha  via C-002\n"), "{text}");
    assert!(
        text.ends_with(
            "cycle: alpha U-001 → beta U-001 → alpha U-001\n\
             cycle: delta U-001 → gamma U-001 → delta U-001\n\
             waiting behind a cycle: alpha U-002\n"
        ),
        "{text}"
    );
    let v: serde_json::Value =
        serde_json::from_slice(&vitrify(&root, &["locate", "--graph", "--json"]).stdout).unwrap();
    assert_eq!(v["order"].as_array().unwrap().len(), 0);
    assert_eq!(v["cycles"].as_array().unwrap().len(), 2);
    assert_eq!(v["waiting"][0]["unit"], "U-002");
}

#[test]
fn locate_graph_is_not_for_one_sdd() {
    let (_tmp, root) = project("project-pipeline");
    for args in [
        &["locate", "--graph", "--units"][..],
        &["locate", "--graph", "--sdd", "specs/api"],
    ] {
        assert_eq!(vitrify(&root, args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn a_pin_in_depends_on_gates_claim_until_its_producers_are_built() {
    let (_tmp, root) = project("project-pipeline");
    let api_units = |root: &Path| -> serde_json::Value {
        let out = vitrify(root, &["locate", "--units", "--sdd", "specs/api", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        serde_json::from_slice(&out.stdout).unwrap()
    };

    // core U-001 is Ready: api U-001 waits on it through the C-001 pin
    let v = api_units(&root);
    assert_eq!(v["units"][0]["claimable"], false);
    assert_eq!(v["units"][0]["depends_on"]["C-001"], "Ready");
    let out = vitrify(
        &root,
        &[
            "claim",
            "--unit",
            "U-001",
            "--holder",
            "w1",
            "--sdd",
            "specs/api",
        ],
    );
    assert_eq!(out.status.code(), Some(4), "{}", stdout(&out));
    assert!(
        stderr(&out).contains(
            "error: U-001 not claimable (status=Ready, unmet deps: C-001 (core U-001=Ready))"
        ),
        "{}",
        stderr(&out)
    );

    // built: the pin is met, the claim publishes
    set_status(&root, "specs/core/units/U-001-pkg.md", "Ready", "Built");
    let v = api_units(&root);
    assert_eq!(v["units"][0]["claimable"], true);
    assert_eq!(v["units"][0]["depends_on"]["C-001"], "Built");
    // api U-002 still waits on core U-002 through C-002, and on api U-001
    assert_eq!(v["units"][1]["claimable"], false);
    let out = vitrify(
        &root,
        &[
            "claim",
            "--unit",
            "U-001",
            "--holder",
            "w1",
            "--sdd",
            "specs/api",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = vitrify(&root, &["locate", "--units", "--sdd", "specs/api"]);
    assert!(
        stdout(&text).starts_with("U-001    Claimed "),
        "{}",
        stdout(&text)
    );
}

/// A vitrify home `vitrify init` has set up, shared by this file's tests.
fn home() -> &'static Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .arg("init")
            .env("VITRIFY_HOME", home.path())
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "vitrify init: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        home
    })
    .path()
}
