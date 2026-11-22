//! `vitrify brief`: the procedure index, every task printed with the
//! binary's version filled in and no slot left open, `brief run`'s
//! status-check interval from configuration, and who approves a TDD in
//! `brief authoring` and `brief change` (`authoring.approval`). Every test forces
//! VITRIFY_HOME to a temp dir — the suite never touches the real ~/.vitrify.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::prelude::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const TASKS: [&str; 7] = [
    "onboard",
    "authoring",
    "change",
    "run",
    "kickoff",
    "build-unit",
    "integrate",
];

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
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("work");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&cwd).unwrap();
    if with_example {
        copy_dir(&repo_root().join("fixtures/example"), &cwd);
    }
    set_up_home(&home);
    Env {
        cwd: cwd.canonicalize().unwrap(),
        home,
        _tmp: tmp,
    }
}

fn vitrify(env: &Env, args: &[&str]) -> Output {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(&env.cwd)
        .env("VITRIFY_HOME", &env.home)
        .env_remove("VITRIFY_RUN_STATUS_CHECK_SECONDS")
        .env_remove("VITRIFY_AUTHORING_APPROVAL")
        .args(args);
    c.output().unwrap()
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

/// A `{name}` slot left in printed text: braces around a bare identifier.
/// JSON (`{"unit":…}`) and prose braces do not match.
fn open_slots(text: &str) -> Vec<String> {
    let mut slots = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('}') {
            let name = &rest[..j];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                slots.push(format!("{{{name}}}"));
            }
        }
    }
    slots
}

#[test]
fn brief_without_a_task_prints_the_index() {
    let env = setup(false);
    let index = stdout(&vitrify(&env, &["brief"]));
    let lines: Vec<&str> = index.lines().collect();
    assert_eq!(
        lines[0],
        format!("vitrify {VERSION} procedures — print one with: vitrify brief <task>")
    );
    assert_eq!(lines[1], "");
    let tasks: Vec<&str> = lines[2..]
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(tasks, TASKS);
    assert!(index.contains(
        "  change      change TDDs and SDDs, one or many, and record what each change stales"
    ));
    // the audience column lines up, and says who follows each procedure
    let audience: Vec<&str> = lines[2..]
        .iter()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(
        audience,
        [
            "session",
            "session",
            "session",
            "session",
            "session",
            "workers",
            "integrator"
        ]
    );
    let column: Vec<usize> = lines[2..]
        .iter()
        .map(|l| l.chars().count() - l.rsplit(' ').next().unwrap().chars().count())
        .collect();
    assert!(column.iter().all(|c| *c == column[0]), "{index}");
}

#[test]
fn every_task_prints_with_its_version_and_no_open_slot() {
    let env = setup(true);
    for task in TASKS {
        let args: Vec<&str> = if task == "build-unit" {
            vec!["brief", task, "--unit", "U-003", "--holder", "job1-w1"]
        } else {
            vec!["brief", task]
        };
        let body = stdout(&vitrify(&env, &args));
        assert!(
            body.contains(&format!("Version: vitrify {VERSION}")),
            "{task}: {body}"
        );
        assert!(body.contains("Use when: "), "{task}");
        assert!(body.contains("Not for: "), "{task}");
        assert_eq!(open_slots(&body), Vec::<String>::new(), "{task}");
    }
    // the bare build-unit template keeps its job slots for the worker, but
    // names the version
    let template = stdout(&vitrify(&env, &["brief", "build-unit"]));
    assert!(template.contains(&format!("Version: vitrify {VERSION}")));
    assert!(template.contains("unit: {unit}"));
}

#[test]
fn brief_run_names_the_status_check_interval() {
    let env = setup(false);
    let body = stdout(&vitrify(&env, &["brief", "run"]));
    assert!(body.contains("every **270 seconds**"), "{body}");

    let mut c = Command::cargo_bin("vitrify").unwrap();
    let out = c
        .current_dir(&env.cwd)
        .env("VITRIFY_HOME", &env.home)
        .env("VITRIFY_RUN_STATUS_CHECK_SECONDS", "600")
        .args(["brief", "run"])
        .output()
        .unwrap();
    assert!(stdout(&out).contains("every **600 seconds**"));

    // from the home config file
    std::fs::create_dir_all(&env.home).unwrap();
    std::fs::write(
        env.home.join("config.toml"),
        "[run]\nstatus_check_seconds = 900\n",
    )
    .unwrap();
    let body = stdout(&vitrify(&env, &["brief", "run"]));
    assert!(body.contains("every **900 seconds**"), "{body}");
}

#[test]
fn an_invalid_status_check_interval_exits_2_naming_the_key() {
    let env = setup(false);
    let mut c = Command::cargo_bin("vitrify").unwrap();
    let out = c
        .current_dir(&env.cwd)
        .env("VITRIFY_HOME", &env.home)
        .env("VITRIFY_RUN_STATUS_CHECK_SECONDS", "0")
        .args(["brief", "run"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("run.status_check_seconds"), "{err}");

    std::fs::create_dir_all(&env.home).unwrap();
    std::fs::write(
        env.home.join("config.toml"),
        "[run]\nstatus_check_seconds = \"soon\"\n",
    )
    .unwrap();
    let out = vitrify(&env, &["brief", "run"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("run.status_check_seconds"), "{err}");
    assert!(err.contains("config.toml"), "{err}");
}

/// The sign-off sentences as the briefs have always read them: what
/// `authoring.approval = "human"` prints, byte for byte.
const HUMAN_SIGN_OFF: [(&str, &str); 5] = [
    (
        "authoring",
        "Repeat until §1–§6 can be written. The human approves the TDD, or its\nrevision; you do not.",
    ),
    (
        "authoring",
        "named rejected alternatives. The human drives it to **Approved** with a\npinnable revision identifier.",
    ),
    (
        "change",
        "before editing anything. Show it to the human when the\nchange is theirs to approve.",
    ),
    (
        "change",
        "until the revision can be written. The human approves the revision before\nany SDD text changes; you do not.",
    ),
    (
        "change",
        "* Mark a TDD revision Approved: the human approves it.",
    ),
];

/// What `authoring.approval = "agent"` prints in their place.
const AGENT_SIGN_OFF: [(&str, &str); 5] = [
    (
        "authoring",
        "Repeat until §1–§6 can be written. Approval is delegated to you\n(`authoring.approval = \"agent\"`): you approve the TDD, or its revision,\nyourself, at the gate in §2.",
    ),
    (
        "authoring",
        "named rejected alternatives. You drive it to **Approved** with a\npinnable revision identifier: when every other line of the gate below\npasses, set the TDD's Status to Approved and bump its Revision yourself.",
    ),
    (
        "change",
        "before editing anything. Approval is delegated to you\n(`authoring.approval = \"agent\"`): go on once the list checks out.",
    ),
    (
        "change",
        "until the revision can be written. You approve the revision before\nany SDD text changes: run it through the SDD-readiness checklist\n(`vitrify schema show --tdd`) line by line; when every other line passes,\nset its Status to Approved and bump its Revision yourself. If any line\nfails, stop and sharpen that line rather than approve.",
    ),
    (
        "change",
        "* Mark a TDD revision Approved before every other line of the\n  SDD-readiness checklist passes.",
    ),
];

fn brief_with(env: &Env, task: &str, approval: Option<&str>) -> Output {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(&env.cwd)
        .env("VITRIFY_HOME", &env.home)
        .env_remove("VITRIFY_AUTHORING_APPROVAL")
        .args(["brief", task]);
    if let Some(v) = approval {
        c.env("VITRIFY_AUTHORING_APPROVAL", v);
    }
    c.output().unwrap()
}

/// Which sign-off the authoring and change briefs print: every sentence of
/// one set, none of the other.
fn assert_sign_off(env: &Env, approval: Option<&str>, agent: bool) {
    let (want, not) = if agent {
        (AGENT_SIGN_OFF, HUMAN_SIGN_OFF)
    } else {
        (HUMAN_SIGN_OFF, AGENT_SIGN_OFF)
    };
    for task in ["authoring", "change"] {
        let body = stdout(&brief_with(env, task, approval));
        for (_, text) in want.iter().filter(|(t, _)| *t == task) {
            assert!(
                body.contains(text),
                "{task}, agent={agent}: missing {text:?}\n{body}"
            );
        }
        for (_, text) in not.iter().filter(|(t, _)| *t == task) {
            assert!(!body.contains(text), "{task}, agent={agent}: has {text:?}");
        }
        assert_eq!(open_slots(&body), Vec::<String>::new(), "{task}");
    }
}

#[test]
fn approval_human_prints_the_sign_off_as_it_always_read() {
    let env = setup(false);
    // the default
    assert_sign_off(&env, None, false);
    // and said out loud
    assert_sign_off(&env, Some("human"), false);
}

#[test]
fn approval_agent_delegates_the_sign_off_to_the_agent() {
    let env = setup(false);
    assert_sign_off(&env, Some("agent"), true);
}

#[test]
fn approval_resolves_env_over_repo_config_over_home_config() {
    let env = setup(false);
    // the home config file
    let home_cfg = env.home.join("config.toml");
    let shipped = std::fs::read_to_string(&home_cfg).unwrap();
    assert!(shipped.contains("approval = \"human\""), "{shipped}");
    std::fs::write(
        &home_cfg,
        shipped.replacen("approval = \"human\"", "approval = \"agent\"", 1),
    )
    .unwrap();
    assert_sign_off(&env, None, true);
    // the repository's config file over it
    std::fs::create_dir_all(env.cwd.join(".vitrify")).unwrap();
    std::fs::write(
        env.cwd.join(".vitrify/config.toml"),
        "[authoring]\napproval = \"human\"\n",
    )
    .unwrap();
    assert_sign_off(&env, None, false);
    // the environment over both
    assert_sign_off(&env, Some("agent"), true);
}

#[test]
fn an_invalid_approval_exits_2_naming_the_key_and_its_file() {
    let env = setup(false);
    for task in ["authoring", "change"] {
        let out = brief_with(&env, task, Some("auto"));
        assert_eq!(out.status.code(), Some(2), "{task}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("VITRIFY_AUTHORING_APPROVAL"), "{err}");
        assert!(err.contains("authoring.approval"), "{err}");
    }
    std::fs::create_dir_all(env.cwd.join(".vitrify")).unwrap();
    std::fs::write(
        env.cwd.join(".vitrify/config.toml"),
        "[authoring]\napproval = \"robot\"\n",
    )
    .unwrap();
    for task in ["authoring", "change"] {
        let out = brief_with(&env, task, None);
        assert_eq!(out.status.code(), Some(2), "{task}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("authoring.approval"), "{err}");
        assert!(err.contains(".vitrify/config.toml"), "{err}");
    }
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
