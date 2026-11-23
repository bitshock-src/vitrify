//! The seven checks reproduce each seeded fixture's finding set;
//! clean on pristine calc; state-writing flags.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;

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

fn scratch(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded").join(fixture),
        tmp.path(),
    );
    let canon = tmp.path().canonicalize().unwrap();
    (tmp, canon)
}

/// vitrify in `dir`, with a temp root of its own that goes when the command
/// does (check 4 runs the fixture's commands, each in a temp directory).
struct Vitrify {
    cmd: Command,
    _tmp: tempfile::TempDir,
}

impl std::ops::Deref for Vitrify {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.cmd
    }
}

impl std::ops::DerefMut for Vitrify {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.cmd
    }
}

fn vitrify(dir: &Path) -> Vitrify {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("vitrify").unwrap();
    cmd.current_dir(dir)
        .env("TMPDIR", tmp.path())
        .env("VITRIFY_HOME", home());
    Vitrify { cmd, _tmp: tmp }
}

fn finding_set(json: &str) -> Vec<(i64, String)> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let mut set: Vec<(i64, String)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["check"].as_i64().unwrap(),
                f["subject"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    set.sort();
    set.dedup();
    set
}

fn expected_set(fixture_root: &Path) -> Vec<(i64, String)> {
    finding_set(&std::fs::read_to_string(fixture_root.join("expected.json")).unwrap())
}

#[test]
fn seeded_fixtures_reproduce_their_finding_sets() {
    for fixture in [
        "audit-orphan",
        "audit-tests",
        "audit-ghost",
        "audit-escape",
        "audit-rotten",
        "audit-trace-drift",
        "audit-projection",
        "audit-lease",
    ] {
        let (_tmp, root) = scratch(fixture);
        let out = vitrify(&root).args(["audit", "--json"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{fixture}: expected exit 2");
        let got = finding_set(&String::from_utf8(out.stdout).unwrap());
        assert_eq!(got, expected_set(&root), "{fixture}");
    }
}

#[test]
fn skip_rotten_suppresses_check_4() {
    let (_tmp, root) = scratch("audit-rotten");
    vitrify(&root)
        .args(["audit", "--skip-rotten"])
        .assert()
        .success()
        .stdout("audit clean (7/7 checks)\n");
}

#[test]
fn pristine_calc_is_clean_matching_golden() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    let golden =
        std::fs::read_to_string(repo_root().join("fixtures/golden/calc/audit.json")).unwrap();
    vitrify(&root)
        .args(["audit", "--skip-rotten", "--journal", "--json"])
        .assert()
        .success()
        .stdout(golden);
    // --journal appended one audit event
    let journal = std::fs::read_to_string(root.join("specs/calc/journal.ndjson")).unwrap();
    let last = journal.lines().last().unwrap();
    assert!(
        last.contains(r#""event":"audit","actor":"audit","findings":0"#),
        "last journal line: {last}"
    );
    assert!(last.starts_with(r#"{"seq":5,"#));
}

#[test]
fn fix_leases_formalizes_release_and_keeps_seq() {
    let (_tmp, root) = scratch("audit-lease");
    vitrify(&root)
        .args(["audit", "--fix-leases"])
        .assert()
        .code(2) // the lapse is still reported as a finding
        .stderr(predicate::str::contains(
            "1 lapsed lease(s) returned to Ready — commit this",
        ));
    let unit = std::fs::read_to_string(root.join("specs/x/units/U-001-the-unit.md")).unwrap();
    assert!(unit.contains("status: Ready"));
    assert!(unit.contains("claimed_by: —"));
    assert!(unit.contains("claim_expires: —"));
    assert!(unit.contains("claim_seq: 1"), "claim_seq kept");
    let journal = std::fs::read_to_string(root.join("specs/x/journal.ndjson")).unwrap();
    let last = journal.lines().last().unwrap();
    assert!(
        last.contains(r#""event":"release","unit":"U-001","actor":"audit","claim_seq":1"#),
        "{last}"
    );
    // ledger regenerated in the worktree
    assert!(root.join("specs/x/ledger.md").exists());
    // a second audit is clean
    vitrify(&root).args(["audit"]).assert().success();
}

#[test]
fn apply_stale_marks_and_journals() {
    let (_tmp, root) = scratch("audit-trace-drift");
    vitrify(&root)
        .args(["audit", "--apply-stale"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "1 unit(s) marked Stale — commit this",
        ));
    let unit = std::fs::read_to_string(root.join("specs/x/units/U-001-the-unit.md")).unwrap();
    assert!(unit.contains("status: Stale"));
    let journal = std::fs::read_to_string(root.join("specs/x/journal.ndjson")).unwrap();
    let last = journal.lines().last().unwrap();
    assert!(
        last.contains(r#""event":"stale","unit":"U-001","actor":"audit""#),
        "{last}"
    );
    assert!(last.contains("recorded trace differs"));
    // a journal event declares the Stale mark → audit is clean
    vitrify(&root).args(["audit"]).assert().success();
}

#[test]
fn orphan_scan_follows_gitignore_not_ecosystem_names() {
    // in a repository the governed tree is what git would commit: ignored
    // files are never orphans, and no directory name is special
    let (_tmp, root) = scratch("audit-orphan");
    let write = |rel: &str, content: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    };
    write(".gitignore", "src/build/\n");
    write("src/build/out.o", "ignored build output\n");
    write("src/node_modules/pkg/index.js", "not ignored here\n");
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");

    let out = vitrify(&root).args(["audit", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        finding_set(&String::from_utf8(out.stdout).unwrap()),
        vec![
            (1, "src/node_modules/pkg/index.js".to_string()),
            (1, "src/orphan.txt".to_string()),
        ]
    );
}

#[test]
fn text_output_format() {
    let (_tmp, root) = scratch("audit-orphan");
    vitrify(&root)
        .args(["audit"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "[check 1] src/orphan.txt: file under scope_roots is neither owned nor unmanaged",
        ));
}

/// A vitrify home `vitrify init` has set up — every command needs one —
/// shared by this file's tests, which only read it.
fn home() -> &'static Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_vitrify"))
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
