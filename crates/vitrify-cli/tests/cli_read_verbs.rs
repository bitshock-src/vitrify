//! Locate/readset golden byte equality, exit codes,
//! budget checks, env precedence, and the complete command tree.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(repo_root().join("fixtures/golden/calc").join(name)).unwrap()
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

/// A scratch copy of the vendored calc example; returns its canonical root.
fn scratch_example() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), tmp.path());
    let canon = tmp.path().canonicalize().unwrap();
    (tmp, canon)
}

fn vitrify(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(dir).env("VITRIFY_HOME", home());
    c
}

#[test]
fn locate_units_json_matches_golden_bytes() {
    let (_tmp, root) = scratch_example();
    let expected = golden("locate.json").replace("{ROOT}", root.to_str().unwrap());
    vitrify(&root)
        .args(["locate", "--units", "--json"])
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn readset_json_matches_golden_bytes_per_unit() {
    let (_tmp, root) = scratch_example();
    for uid in ["U-001", "U-002", "U-003", "U-004", "U-005"] {
        let expected =
            golden(&format!("readset-{uid}.json")).replace("{ROOT}", root.to_str().unwrap());
        vitrify(&root)
            .args(["readset", "--unit", uid, "--json"])
            .assert()
            .success()
            .stdout(expected);
    }
}

#[test]
fn locate_text_mode_lists_units() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["locate", "--units"])
        .assert()
        .success()
        .stdout(predicate::str::contains("U-002    Ready"))
        .stdout(predicate::str::contains("claimable"));
    vitrify(&root)
        .args(["locate"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "[bundle]  calc v1.0.0 Active  units=5",
        ));
}

#[test]
fn budget_pass_and_fail() {
    let (_tmp, root) = scratch_example();
    // generous explicit budget passes
    vitrify(&root)
        .args([
            "readset", "--unit", "U-003", "--json", "--budget", "10000000",
        ])
        .assert()
        .success();
    // tiny explicit budget fails with exit 2 naming total and budget
    vitrify(&root)
        .args(["readset", "--unit", "U-003", "--budget", "10"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("over budget 10"))
        .stderr(predicate::str::contains("bytes"));
    // bare --budget uses the config default (64 KiB) — calc fits
    vitrify(&root)
        .args(["readset", "--unit", "U-003", "--budget"])
        .assert()
        .success();
}

#[test]
fn env_precedence_forces_budget_failure() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["readset", "--unit", "U-003", "--budget"])
        .env("VITRIFY_LINT_READSET_BUDGET_BYTES", "1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("over budget 1"));
    // unparseable env value is exit 2 naming the variable
    vitrify(&root)
        .args(["readset", "--unit", "U-003", "--budget"])
        .env("VITRIFY_LINT_READSET_BUDGET_BYTES", "notanumber")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "VITRIFY_LINT_READSET_BUDGET_BYTES",
        ));
}

#[test]
fn exit_4_cases() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["readset", "--unit", "U-099"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("unit not found: U-099"));
    vitrify(&root)
        .args(["locate", "--units", "--sdd", "/nonexistent/path"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("is not an SDD root"));

    // empty repo: no SDD found, and the refusal names where to start
    let empty = tempfile::tempdir().unwrap();
    for args in [&["locate"][..], &["locate", "--units"], &["lint", "--all"]] {
        vitrify(empty.path())
            .args(args)
            .assert()
            .code(4)
            .stderr(
                "error: no SDD found in repository → start one: vitrify brief authoring (or vitrify new sdd inside specs/)\n",
            );
    }
}

#[test]
fn discovery_follows_gitignore_not_ecosystem_names() {
    // in a repository the repo-wide search covers what git would commit: an
    // SDD under an ignored directory is invisible, and no directory name is
    // special
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let write = |rel: &str, content: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    };
    let sdd = |slug: &str| {
        format!("---\nsdd: {slug}\nschema_version: 2.0\nversion: 0.1.0\nstatus: Draft\n---\n")
    };
    write(".gitignore", "vendored/\n");
    write("docs/app/sdd.md", &sdd("app"));
    write("vendored/lib/sdd.md", &sdd("vendored-lib"));
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");

    // the ignored SDD is not a candidate, so the app's resolves without --sdd
    vitrify(&root)
        .args(["locate", "--units", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docs/app"));

    // a node_modules directory git does not ignore is searched like any other
    write("node_modules/pkg/sdd.md", &sdd("pkg"));
    vitrify(&root)
        .args(["locate", "--units"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("multiple SDDs found"));
}

#[test]
fn two_sdds_without_arg_exit_4() {
    let tmp = tempfile::tempdir().unwrap();
    for slug in ["alpha", "beta"] {
        let dir = tmp.path().join("specs").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sdd.md"),
            format!("---\nsdd: {slug}\nschema_version: 2.0\nversion: 0.1.0\nstatus: Draft\n---\n"),
        )
        .unwrap();
    }
    // the candidates are printed as the `--sdd` values that name them
    vitrify(tmp.path())
        .args(["locate", "--units"])
        .assert()
        .code(4)
        .stderr(
            "error: multiple SDDs found; pass --sdd <root>:\n  --sdd specs/alpha\n  --sdd specs/beta\n",
        );
    // … from where the verb runs
    vitrify(&tmp.path().join("specs"))
        .args(["locate", "--units"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("\n  --sdd alpha\n  --sdd beta\n"));
    vitrify(tmp.path())
        .args(["locate", "--units", "--sdd", "specs/alpha", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"shape\": \"bundle\""));
}

#[test]
fn help_lists_all_c011_verbs() {
    let out = Command::cargo_bin("vitrify")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    for verb in [
        "locate",
        "readset",
        "claim",
        "renew",
        "release",
        "trace",
        "ledger",
        "journal",
        "audit",
        "brief",
        "init",
        "probe",
        "run",
        "lint",
        "new",
        "schema",
        "check",
        "change",
        "pin",
        "questions",
    ] {
        assert!(help.contains(verb), "missing verb in --help: {verb}");
    }
}

#[test]
fn usage_errors_exit_2() {
    let (_tmp, root) = scratch_example();
    vitrify(&root).args(["readset"]).assert().code(2); // missing --unit
    vitrify(&root).args(["frobnicate"]).assert().code(2); // unknown verb
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
