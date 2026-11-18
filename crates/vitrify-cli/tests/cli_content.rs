//! First-run materialization + manifest, disk-wins
//! overrides, refresh of unmodified defaults, delete-restores, brief/new/schema.
//! Every test forces VITRIFY_HOME to a temp dir — the suite never touches
//! the real ~/.vitrify.

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

struct Env {
    _tmp: tempfile::TempDir,
    cwd: PathBuf,
    home: PathBuf,
}

fn setup(with_example: bool) -> Env {
    let env = setup_bare(with_example);
    set_up_home(&env.home);
    env
}

/// A working directory and a home `vitrify init` has not set up.
fn setup_bare(with_example: bool) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("work");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&cwd).unwrap();
    if with_example {
        copy_dir(&repo_root().join("fixtures/example"), &cwd);
    }
    Env {
        cwd: cwd.canonicalize().unwrap(),
        home,
        _tmp: tmp,
    }
}

fn vitrify(env: &Env) -> Command {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(&env.cwd);
    c.env("VITRIFY_HOME", &env.home);
    c
}

#[test]
fn every_command_refuses_a_home_init_has_not_set_up_and_leaves_it_alone() {
    let env = setup_bare(true);
    for args in [
        &["brief"][..],
        &["brief", "kickoff"],
        &["schema", "show"],
        &["new", "tdd", "calc"],
        &["probe"],
        &["lint"],
        &["check"],
        &["audit"],
        &["locate"],
        &["locate", "--units"],
        &["readset", "--unit", "U-002"],
        &["questions"],
        &["ledger", "progress"],
        &["journal", "replay"],
        &["pin", "list"],
        &["run", "status"],
        &["run", "--dry-run"],
        &["claim", "--unit", "U-002", "--holder", "h1"],
        &["trace", "verify", "--unit", "U-001"],
        &["change", "apply"],
    ] {
        let out = vitrify(&env).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(4), "{args:?}");
        assert_eq!(
            String::from_utf8(out.stderr).unwrap(),
            "error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init\n",
            "{args:?}"
        );
        assert!(!env.home.exists(), "{args:?} created the home");
    }
    assert!(!env.cwd.join("calc.md").exists());
    // help and the version need no home
    vitrify(&env).arg("--help").assert().success();
    vitrify(&env).arg("--version").assert().success();
    vitrify(&env).args(["lint", "--help"]).assert().success();
    assert!(!env.home.exists());
}

#[test]
fn init_materializes_home_with_manifest_digests() {
    let env = setup(false);
    assert!(env.home.join("config.toml").exists());
    assert!(env.home.join("briefs/kickoff.md").exists());
    assert!(env.home.join("briefs/escalation.md").exists());
    assert!(env.home.join("templates/tdd-template.md").exists());
    assert!(env.home.join("schemas/sdd-schema.md").exists());

    let manifest = std::fs::read_to_string(env.home.join(".manifest.toml")).unwrap();
    assert!(manifest.starts_with("schema = 1\n"));
    assert!(manifest.contains(&format!(
        "vitrify_version = \"{}\"",
        env!("CARGO_PKG_VERSION")
    )));
    let table: toml::Table = manifest.parse().unwrap();
    let files = table["files"].as_table().unwrap();
    let recorded = files["briefs/kickoff.md"].as_str().unwrap();
    let disk = std::fs::read_to_string(env.home.join("briefs/kickoff.md")).unwrap();
    assert_eq!(
        recorded,
        format!("sha256:{}", vitrify_core::content::sha256_hex(&disk))
    );
}

#[test]
fn brief_prints_the_asset_and_disk_override_wins() {
    let env = setup(false);
    let out = vitrify(&env).args(["brief", "kickoff"]).output().unwrap();
    assert!(out.status.success());
    let body = String::from_utf8(out.stdout).unwrap();
    assert!(body.contains("# Kickoff — the manual/interactive orchestration procedure"));

    // override the brief on disk; the next invocation serves the override
    std::fs::write(env.home.join("briefs/kickoff.md"), "OVERRIDDEN BODY\n").unwrap();
    vitrify(&env)
        .args(["brief", "kickoff"])
        .assert()
        .success()
        .stdout("OVERRIDDEN BODY\n");
}

#[test]
fn override_survives_version_change_and_unmodified_refreshes() {
    let env = setup(false);
    vitrify(&env).args(["brief", "kickoff"]).assert().success();

    // user overrides one file; another stays as shipped
    std::fs::write(env.home.join("briefs/kickoff.md"), "MY OVERRIDE\n").unwrap();
    // simulate an older binary: rewrite the manifest version and corrupt the
    // shipped copy of integrate.md while keeping its manifest digest in sync
    // (an "unmodified default from an older version")
    let manifest_path = env.home.join(".manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let old_body = "OLD SHIPPED INTEGRATE BODY\n";
    std::fs::write(env.home.join("briefs/integrate.md"), old_body).unwrap();
    let old_digest = vitrify_core::content::sha256_hex(old_body);
    let manifest = manifest
        .replace(
            &format!("vitrify_version = \"{}\"", env!("CARGO_PKG_VERSION")),
            "vitrify_version = \"0.0.1\"",
        )
        .lines()
        .map(|l| {
            if l.starts_with("\"briefs/integrate.md\"") {
                format!("\"briefs/integrate.md\" = \"sha256:{old_digest}\"")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&manifest_path, manifest).unwrap();

    // any content-consuming verb triggers the staleness pass
    vitrify(&env).args(["brief", "onboard"]).assert().success();

    // unmodified default refreshed to the new embedded copy
    let integrate = std::fs::read_to_string(env.home.join("briefs/integrate.md")).unwrap();
    assert!(integrate.contains("# Integrate one Built unit"));
    // the override is untouched
    let kickoff = std::fs::read_to_string(env.home.join("briefs/kickoff.md")).unwrap();
    assert_eq!(kickoff, "MY OVERRIDE\n");
}

#[test]
fn an_unmodified_default_follows_a_rebuild_of_the_same_version() {
    let env = setup(false);
    vitrify(&env).args(["brief", "kickoff"]).assert().success();
    std::fs::write(env.home.join("briefs/kickoff.md"), "MY OVERRIDE\n").unwrap();
    // another build of this same version shipped a different integrate.md,
    // and nobody edited it: the manifest records the digest it was written with
    let old_body = "THIS VERSION'S OTHER BUILD\n";
    std::fs::write(env.home.join("briefs/integrate.md"), old_body).unwrap();
    let old_digest = vitrify_core::content::sha256_hex(old_body);
    let manifest_path = env.home.join(".manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains(&format!(
        "vitrify_version = \"{}\"",
        env!("CARGO_PKG_VERSION")
    )));
    let manifest = manifest
        .lines()
        .map(|l| {
            if l.starts_with("\"briefs/integrate.md\"") {
                format!("\"briefs/integrate.md\" = \"sha256:{old_digest}\"")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&manifest_path, manifest).unwrap();

    vitrify(&env)
        .args(["brief", "integrate"])
        .assert()
        .success()
        .stdout(predicates::str::contains("# Integrate one Built unit"));
    // the stale default is this binary's again; the override is still yours
    let integrate = std::fs::read_to_string(env.home.join("briefs/integrate.md")).unwrap();
    assert!(integrate.contains("# Integrate one Built unit"));
    let kickoff = std::fs::read_to_string(env.home.join("briefs/kickoff.md")).unwrap();
    assert_eq!(kickoff, "MY OVERRIDE\n");
}

#[test]
fn deleted_file_is_restored_on_next_run() {
    let env = setup(false);
    vitrify(&env).args(["brief", "kickoff"]).assert().success();
    std::fs::remove_file(env.home.join("briefs/kickoff.md")).unwrap();
    vitrify(&env).args(["brief", "onboard"]).assert().success();
    assert!(env.home.join("briefs/kickoff.md").exists());
}

#[test]
fn worker_prompt_carries_job_block_reading_set_and_schema() {
    let env = setup(true);
    let out = vitrify(&env)
        .args([
            "brief",
            "build-unit",
            "--unit",
            "U-003",
            "--holder",
            "job1-w2",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let prompt = String::from_utf8(out.stdout).unwrap();
    // filled vitrify-job block
    assert!(prompt.starts_with("```vitrify-job\n"));
    assert!(prompt.contains("unit: U-003\n"));
    assert!(prompt.contains("holder: job1-w2\n"));
    // Ready at seq 0 → next claim is c1; the branch carries the SDD's slug
    assert!(prompt.contains("branch: sdd/calc/U-003/c1\n"));
    assert!(prompt.contains("lease_minutes: 60\n"));
    // the SDD by its path from the checkout root — and on every verb, so
    // the commands work in a repository with several SDDs
    assert!(prompt.contains("sdd: specs/calc\n"), "{prompt}");
    assert!(prompt.contains("vitrify readset --unit U-003 --sdd specs/calc --json"));
    assert!(prompt.contains("vitrify trace record --unit U-003 --stamp built --sdd specs/calc"));
    assert!(
        prompt.contains("-b sdd/calc/U-003/c1 <claim-commit>"),
        "{prompt}"
    );
    // no slot is left unfilled, and nothing is said about a session that
    // has nothing to say
    assert!(!prompt.contains("{session}") && !prompt.contains("{worktree"));
    assert!(!prompt.contains("## This session"));
    // report schema verbatim
    assert!(prompt.contains(
        r#"{"unit":"U-###","outcome":"built|blocked|released","branch":"sdd/<sdd>/U-###/c<seq>","claim_seq":N,"q":"Q-### or null","notes":"<one line>"}"#
    ));
    // boundaries
    assert!(prompt.contains("write only the unit's `owns` paths"));
    // resolved reading set inlined: region text + contract bodies present
    assert!(prompt.contains("## Resolved reading set"));
    assert!(prompt.contains("## Ground Rules"));
    assert!(prompt.contains("C-002"));
    assert!(prompt.contains("### Reading 5")); // GR, TD, unit, C-001, C-002
}

#[test]
fn brief_usage_and_not_found_exits() {
    let env = setup(true);
    // --unit without build-unit → 2
    vitrify(&env)
        .args(["brief", "kickoff", "--unit", "U-001"])
        .assert()
        .code(2);
    // unknown phase → clap usage error 2
    vitrify(&env).args(["brief", "bogus"]).assert().code(2);
    // unknown unit → 4
    vitrify(&env)
        .args(["brief", "build-unit", "--unit", "U-099", "--holder", "w1"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("unit not found: U-099"));
    // --unit without --holder → 2
    vitrify(&env)
        .args(["brief", "build-unit", "--unit", "U-001"])
        .assert()
        .code(2);
}

#[test]
fn new_materializes_and_refuses_overwrite() {
    let env = setup(false);
    vitrify(&env)
        .args(["new", "tdd", "calc"])
        .assert()
        .success();
    assert!(env.cwd.join("calc.md").exists());
    vitrify(&env)
        .args(["new", "tdd", "calc"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("refusing to overwrite"));

    vitrify(&env)
        .args(["new", "sdd", "--single", "calc"])
        .assert()
        .success();
    assert!(env.cwd.join("calc.sdd.md").exists());

    vitrify(&env)
        .args(["new", "sdd", "calcb"])
        .assert()
        .success();
    for f in [
        "sdd.md",
        "units/U-001-foundation.md",
        "contracts/C-001-example.md",
        "changelog.md",
        "decisions.md",
    ] {
        assert!(env.cwd.join("calcb").join(f).exists(), "missing {f}");
    }
    vitrify(&env).args(["new", "sdd", "calcb"]).assert().code(2);
}

#[test]
fn schema_show_matches_embedded_bytes() {
    let env = setup(false);
    let expected_sdd = vitrify_core::content::get("schemas/sdd-schema.md")
        .unwrap()
        .body;
    let out = vitrify(&env).args(["schema", "show"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}\n", expected_sdd.trim_end_matches('\n'))
    );

    let expected_tdd = vitrify_core::content::get("schemas/tdd-schema.md")
        .unwrap()
        .body;
    let out = vitrify(&env)
        .args(["schema", "show", "--tdd"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}\n", expected_tdd.trim_end_matches('\n'))
    );
}

#[test]
fn briefs_name_the_real_worktree_root_of_the_repository() {
    // working copies live outside the working tree, under the vitrify home;
    // a brief printed inside a repository says exactly where
    let env = setup(true);
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&env.cwd)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@test"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "seed"]);

    let out = vitrify(&env).args(["brief", "integrate"]).output().unwrap();
    assert!(out.status.success());
    let brief = String::from_utf8(out.stdout).unwrap();
    let root = env.home.join("worktrees");
    assert!(!brief.contains("{worktrees}"), "{brief}");
    assert!(
        brief.contains(&format!("cd {}", root.display())) && brief.contains("/mainline\n"),
        "{brief}"
    );
    assert!(!brief.contains(".vitrify/mainline"));

    let out = vitrify(&env)
        .args(["brief", "build-unit", "--unit", "U-003", "--holder", "w1"])
        .output()
        .unwrap();
    let prompt = String::from_utf8(out.stdout).unwrap();
    let line = prompt
        .lines()
        .find(|l| l.starts_with("git worktree add "))
        .unwrap();
    assert!(line.contains(&root.display().to_string()), "{line}");
    assert!(
        line.contains("/calc-U-003-c1 -b sdd/calc/U-003/c1 "),
        "{line}"
    );

    // run.worktree_dir moves them — beside the repository, say
    let out = vitrify(&env)
        .args(["brief", "kickoff"])
        .env("VITRIFY_RUN_WORKTREE_DIR", "../calc-worktrees")
        .output()
        .unwrap();
    let brief = String::from_utf8(out.stdout).unwrap();
    let beside = env.cwd.parent().unwrap().join("calc-worktrees");
    assert!(brief.contains(&beside.display().to_string()), "{brief}");
}

#[test]
fn outside_a_repository_a_brief_still_prints() {
    let env = setup(false);
    let out = vitrify(&env).args(["brief", "integrate"]).output().unwrap();
    assert!(out.status.success());
    let brief = String::from_utf8(out.stdout).unwrap();
    assert!(brief.contains("<worktrees>/mainline"), "{brief}");
}

#[test]
fn every_command_reads_the_configuration_alike() {
    let env = setup(true);
    let repo_config = env.cwd.join(".vitrify/config.toml");
    std::fs::create_dir_all(repo_config.parent().unwrap()).unwrap();
    for (text, says) in [
        ("schema = 2\n", "`schema`"),
        (
            "[probe.claude]\npresent = true\n",
            "`probe` describes the machine",
        ),
        ("[run]\nworker_cap = \"many\"\n", "run.worker_cap"),
    ] {
        std::fs::write(&repo_config, text).unwrap();
        for args in [&["locate"][..], &["lint"], &["brief"], &["questions"]] {
            let out = vitrify(&env).args(args).output().unwrap();
            assert_eq!(out.status.code(), Some(2), "{args:?} with {text:?}");
            let err = String::from_utf8(out.stderr).unwrap();
            assert!(err.contains(says), "{args:?} with {text:?}: {err}");
        }
    }
    // overrides that are a repository's to make are taken
    std::fs::write(
        &repo_config,
        "schema = 1\n[run]\nworker_cap = 2\n[harness.claude.worker]\nmodel = \"sonnet\"\n",
    )
    .unwrap();
    vitrify(&env).arg("locate").assert().success();
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
