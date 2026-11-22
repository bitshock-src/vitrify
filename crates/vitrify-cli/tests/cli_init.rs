//! `vitrify init` and `vitrify probe`, with fake harness CLIs on a
//! test-controlled PATH. init sets the home up — config.toml with every
//! default, the content, a first probe recorded without asking — and refuses
//! to set it up twice. probe brings the record up to date: it says what it
//! would change and asks first (`-y` answers yes), and changes nothing else.
//! The one streaming spelling per harness is recorded when it works, a CLI
//! that does not accept it fails naming it, absent → untested, hang →
//! timeout → named failure.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use assert_cmd::prelude::*;
use predicates::prelude::*;

struct Env {
    _tmp: tempfile::TempDir,
    cwd: PathBuf,
    home: PathBuf,
    bin: PathBuf,
}

fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("repo");
    let home = tmp.path().join("vhome");
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    Env {
        cwd: cwd.canonicalize().unwrap(),
        home,
        bin,
        _tmp: tmp,
    }
}

fn fake_cli(env: &Env, name: &str, script: &str) {
    let path = env.bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A claude that answers every probe; its `--version` is `version`.
fn working_claude(env: &Env, version: &str) {
    fake_cli(
        env,
        "claude",
        &format!(
            r#"case "$1" in --version) echo "{version}"; exit 0;; esac
echo OK"#
        ),
    );
}

fn vitrify(env: &Env) -> Command {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    // controlled PATH: fakes + system dirs for git/sh
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", env.bin.display());
    c.current_dir(&env.cwd)
        .env("VITRIFY_HOME", &env.home)
        .env("PATH", path)
        .env("VITRIFY_PROBE_TIMEOUT_SECS", "2");
    c
}

fn config_toml(env: &Env) -> String {
    std::fs::read_to_string(env.home.join("config.toml")).unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

/// `vitrify probe` with `answer` on stdin (`None`: stdin closed at once).
fn probe(env: &Env, args: &[&str], answer: Option<&str>) -> Output {
    use std::io::Write;
    let mut c = vitrify(env);
    c.arg("probe")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match answer {
        Some(a) => {
            let mut child = c.stdin(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(a.as_bytes()).unwrap();
            child.wait_with_output().unwrap()
        }
        None => c.stdin(Stdio::null()).output().unwrap(),
    }
}

// ------------------------------------------------------------------ init

#[test]
fn init_writes_every_default_then_the_first_probe() {
    let env = setup();
    working_claude(&env, "claude 9.9.9");
    let out = vitrify(&env).arg("init").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let t = text(&out);
    assert!(
        t.starts_with(&format!(
            "ok: vitrify version {}\nok: created ~/.vitrify/config.toml with every setting at its default\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{t}"
    );
    assert!(
        t.ends_with("ok: recorded the probe in ~/.vitrify/config.toml\n"),
        "{t}"
    );

    // the shipped defaults, byte for byte, then the record
    let toml_text = config_toml(&env);
    assert!(
        toml_text.starts_with(vitrify_core::content::CONFIG.body),
        "{toml_text}"
    );
    let table: toml::Table = toml_text.parse().unwrap();
    assert_eq!(table["schema"].as_integer(), Some(1));
    assert_eq!(table["run"]["worker_cap"].as_integer(), Some(20));
    assert_eq!(
        table["lint"]["readset_budget_bytes"].as_integer(),
        Some(65536)
    );
    assert_eq!(table["harness"]["preferred"].as_str(), Some(""));
    assert_eq!(
        table["harness"]["claude"]["worker"]["model"].as_str(),
        Some("opus")
    );
    assert_eq!(
        table["probe"]["vitrify_version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    // and the content, materialized
    assert!(env.home.join("briefs/kickoff.md").exists());
    assert!(env.home.join(".manifest.toml").exists());
    // nothing written into the directory it ran in
    assert_eq!(std::fs::read_dir(&env.cwd).unwrap().count(), 0);
}

#[test]
fn init_refuses_a_home_it_already_set_up() {
    let env = setup();
    vitrify(&env).arg("init").assert().success();
    let mut edited = config_toml(&env);
    edited = edited.replacen("worker_cap = 20", "worker_cap = 7", 1);
    std::fs::write(env.home.join("config.toml"), &edited).unwrap();

    vitrify(&env)
        .arg("init")
        .assert()
        .code(2)
        .stderr(
            "error: refusing to overwrite existing ~/.vitrify/config.toml → vitrify probe brings its probe record up to date\n",
        );
    assert_eq!(config_toml(&env), edited);
}

#[test]
fn records_the_spelling_that_worked() {
    let env = setup();
    // claude succeeds on its one spelling — the one that streams its
    // session, so a worker's log fills as it works
    working_claude(&env, "claude 9.9.9");
    vitrify(&env).arg("init").assert().success();
    let toml_text = config_toml(&env);
    assert!(toml_text.contains("[probe.claude]"));
    assert!(
        toml_text.contains(
            r#"headless = ["claude", "-p", "--output-format", "stream-json", "--verbose"]"#
        ),
        "{toml_text}"
    );
    assert!(toml_text.contains(r#"version = "claude 9.9.9""#));
    // absent harnesses recorded as not present
    assert!(toml_text.contains("[probe.codex]\npresent = false"));
}

#[test]
fn a_cli_without_its_streaming_spelling_fails_naming_it() {
    let env = setup();
    // a claude without stream-json, a codex without --json: each answers
    // only a buffered spelling vitrify does not try
    fake_cli(
        &env,
        "claude",
        r#"case "$1" in --version) echo "claude 1.0"; exit 0;; esac
for a in "$@"; do [ "$a" = "stream-json" ] && exit 1; done
echo OK"#,
    );
    fake_cli(
        &env,
        "codex",
        r#"case "$1" in --version) echo "codex 2.0"; exit 0;; esac
for a in "$@"; do [ "$a" = "--json" ] && exit 1; done
echo OK"#,
    );
    let out = vitrify(&env).arg("init").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let t = text(&out);
    assert!(
        t.contains(
            "FAIL: harness claude: headless probe failed — vitrify needs `claude -p --output-format stream-json --verbose`; update claude until it accepts it"
        ),
        "{t}"
    );
    assert!(
        t.contains(
            "FAIL: harness codex: headless probe failed — vitrify needs `codex exec --json`; update codex until it accepts it"
        ),
        "{t}"
    );
    // nothing recorded for either: `run` has no spelling to use
    let toml_text = config_toml(&env);
    assert!(!toml_text.contains("headless ="), "{toml_text}");
    assert!(
        toml_text.contains("[probe.claude]\npresent = true"),
        "{toml_text}"
    );
}

#[test]
fn each_harness_is_probed_with_its_one_spelling() {
    let env = setup();
    // each fake logs its argv; the probe prompt is the last word
    for name in ["claude", "codex", "opencode"] {
        fake_cli(
            &env,
            name,
            &format!(
                r#"case "$1" in --version) echo "{name} 1"; exit 0;; esac
echo "$@" >> "$0.calls"
echo OK"#
            ),
        );
    }
    vitrify(&env).arg("init").assert().success();
    let calls =
        |name: &str| std::fs::read_to_string(env.bin.join(format!("{name}.calls"))).unwrap();
    // the probe runs the spelling once; role probes add flags to it
    let first = |name: &str| calls(name).lines().next().unwrap().to_string();
    assert_eq!(
        first("claude"),
        "-p --output-format stream-json --verbose Reply with exactly: OK"
    );
    assert_eq!(first("codex"), "exec --json Reply with exactly: OK");
    assert_eq!(
        first("opencode"),
        "run --format json Reply with exactly: OK"
    );
    let toml_text = config_toml(&env);
    assert!(
        toml_text.contains(r#"headless = ["codex", "exec", "--json"]"#),
        "{toml_text}"
    );
    assert!(
        toml_text.contains(r#"headless = ["opencode", "run", "--format", "json"]"#),
        "{toml_text}"
    );
}

#[test]
fn absent_cli_is_untested_never_an_error() {
    let env = setup();
    vitrify(&env)
        .arg("init")
        .assert()
        .success() // no harness present is still exit 0
        .stdout(predicate::str::contains(
            "untested: harness claude: CLI not on PATH",
        ))
        .stdout(predicate::str::contains("untested: harness codex"))
        .stdout(predicate::str::contains("untested: harness opencode"));
}

#[test]
fn hang_times_out_as_a_named_failure() {
    let env = setup();
    fake_cli(
        &env,
        "opencode",
        r#"case "$1" in --version) echo "opencode 1.0"; exit 0;; esac
sleep 60"#,
    );
    vitrify(&env)
        .arg("init")
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "FAIL: harness opencode: probe timed out after 2s",
        ));
}

#[test]
fn a_probe_failure_is_a_named_failure() {
    let env = setup();
    fake_cli(
        &env,
        "claude",
        r#"case "$1" in --version) echo "claude 9"; exit 0;; esac
exit 3"#,
    );
    vitrify(&env)
        .arg("init")
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "FAIL: harness claude: headless probe failed",
        ));
}

#[test]
fn init_json_emits_the_probe_record() {
    let env = setup();
    working_claude(&env, "claude 9.9.9");
    let out = vitrify(&env).args(["init", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&text(&out)).unwrap();
    assert_eq!(v["green"], true);
    let claude = v["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == "claude")
        .unwrap();
    assert_eq!(claude["present"], true);
    assert_eq!(
        claude["headless"],
        serde_json::json!([
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "--verbose"
        ])
    );
}

#[test]
fn a_role_model_the_harness_rejects_fails_by_role() {
    let env = setup();
    // accepts the bare headless spelling; rejects any --model it is given
    fake_cli(
        &env,
        "claude",
        r#"case "$1" in --version) echo "claude 9.9.9"; exit 0;; esac
for a in "$@"; do [ "$a" = "--model" ] && exit 1; done
echo OK"#,
    );
    let out = vitrify(&env).arg("init").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let t = text(&out);
    assert!(
        t.contains(
            "ok: harness claude: headless = claude -p --output-format stream-json --verbose"
        ),
        "{t}"
    );
    assert!(
        t.contains(
            "FAIL: harness claude worker: probe failed (claude -p --output-format stream-json --verbose --model "
        ),
        "{t}"
    );
    assert!(
        t.contains("— fix [harness.claude.worker] in ~/.vitrify/config.toml"),
        "{t}"
    );
    assert!(
        t.contains("FAIL: harness claude retry: probe failed"),
        "{t}"
    );
    assert!(
        t.contains("FAIL: harness claude escalation: probe failed"),
        "{t}"
    );
}

// ------------------------------------------------------------------ probe

#[test]
fn probe_refuses_a_home_init_has_not_set_up() {
    let env = setup();
    let out = probe(&env, &["-y"], None);
    assert_eq!(out.status.code(), Some(4));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init\n"
    );
    assert!(!env.home.exists());
}

#[test]
fn probe_with_nothing_new_changes_nothing() {
    let env = setup();
    working_claude(&env, "claude 9.9.9");
    vitrify(&env).arg("init").assert().success();
    let before = config_toml(&env);
    let out = probe(&env, &[], None);
    assert_eq!(out.status.code(), Some(0));
    let t = text(&out);
    assert!(!t.contains("change: "), "{t}");
    assert!(
        t.ends_with("ok: ~/.vitrify/config.toml: no changes\n"),
        "{t}"
    );
    // nothing to ask about, so nothing asked
    assert!(out.stderr.is_empty());
    assert_eq!(config_toml(&env), before);
}

#[test]
fn probe_asks_and_applies_on_yes_keeping_every_setting() {
    let env = setup();
    working_claude(&env, "claude 1.0");
    vitrify(&env).arg("init").assert().success();
    // a setting edited by hand, comment and all
    let shipped = config_toml(&env);
    let edited = shipped.replacen(
        "worker_cap = 20                # max concurrent workers",
        "worker_cap = 7 # mine",
        1,
    );
    assert_ne!(edited, shipped);
    std::fs::write(env.home.join("config.toml"), &edited).unwrap();
    working_claude(&env, "claude 1.1");

    let out = probe(&env, &[], Some("y\n"));
    assert_eq!(out.status.code(), Some(0));
    let t = text(&out);
    assert!(
        t.contains("change: probe.claude.version: claude 1.0 → claude 1.1\n"),
        "{t}"
    );
    assert!(
        t.ends_with("ok: updated ~/.vitrify/config.toml (1 change)\n"),
        "{t}"
    );
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "apply 1 change to ~/.vitrify/config.toml? [y/N] "
    );
    let after = config_toml(&env);
    assert_eq!(
        after,
        edited.replacen("version = \"claude 1.0\"", "version = \"claude 1.1\"", 1)
    );
}

#[test]
fn probe_declined_changes_nothing() {
    let env = setup();
    working_claude(&env, "claude 1.0");
    vitrify(&env).arg("init").assert().success();
    let before = config_toml(&env);
    working_claude(&env, "claude 2.0");

    let out = probe(&env, &[], Some("n\n"));
    assert_eq!(out.status.code(), Some(2));
    let t = text(&out);
    assert!(
        t.contains("change: probe.claude.version: claude 1.0 → claude 2.0\n"),
        "{t}"
    );
    assert!(
        t.ends_with("note: ~/.vitrify/config.toml: nothing changed\n"),
        "{t}"
    );
    assert_eq!(config_toml(&env), before);

    // nobody there to answer: the same, and how to apply anyway
    let out = probe(&env, &[], None);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out).ends_with(
            "note: ~/.vitrify/config.toml: nothing changed — no answer on stdin; vitrify probe -y applies without asking\n"
        ),
        "{}",
        text(&out)
    );
    assert_eq!(config_toml(&env), before);
}

#[test]
fn probe_assume_yes_applies_without_asking() {
    let env = setup();
    vitrify(&env).arg("init").assert().success(); // no harness yet
    working_claude(&env, "claude 3.0");
    for flag in ["-y", "--assume-yes"] {
        let out = probe(&env, &[flag], None);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        assert!(out.stderr.is_empty(), "{flag} asked");
        if flag == "-y" {
            let t = text(&out);
            assert!(
                t.contains(
                    "change: probe.claude.headless: (none) → claude -p --output-format stream-json --verbose\n"
                ),
                "{t}"
            );
            assert!(
                t.contains("change: probe.claude.present: false → true\n"),
                "{t}"
            );
        } else {
            // the record is up to date: nothing left to change
            assert!(text(&out).ends_with("ok: ~/.vitrify/config.toml: no changes\n"));
        }
    }
    let table: toml::Table = config_toml(&env).parse().unwrap();
    assert_eq!(
        table["probe"]["claude"]["version"].as_str(),
        Some("claude 3.0")
    );
}

#[test]
fn probe_json_lists_the_changes() {
    let env = setup();
    working_claude(&env, "claude 1.0");
    vitrify(&env).arg("init").assert().success();
    working_claude(&env, "claude 1.5");
    let out = probe(&env, &["--json", "-y"], None);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&text(&out)).unwrap();
    assert_eq!(v["green"], true);
    assert_eq!(v["applied"], true);
    assert_eq!(
        v["changes"],
        serde_json::json!([
            {"key": "probe.claude.version", "from": "claude 1.0", "to": "claude 1.5"}
        ])
    );
}

#[test]
fn probe_reports_home_state_and_overrides() {
    let env = setup();
    vitrify(&env).arg("init").assert().success();
    std::fs::write(env.home.join("briefs/kickoff.md"), "override\n").unwrap();

    let out = probe(&env, &[], None);
    assert_eq!(out.status.code(), Some(0));
    let t = text(&out);
    assert!(t.contains("note: override: briefs/kickoff.md"), "{t}");
    assert!(t.contains("~/.vitrify materialized"), "{t}");
    // nothing written into the directory it ran in
    assert_eq!(std::fs::read_dir(&env.cwd).unwrap().count(), 0);
}

#[test]
fn probe_reports_per_role_profiles_and_refuses_a_broken_one() {
    let env = setup();
    working_claude(&env, "claude 9.9.9");
    // shipped defaults: one line per role for the working harness only
    let out = vitrify(&env).arg("init").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let t = text(&out);
    assert!(t.contains("ok: harness claude worker: model="), "{t}");
    assert!(t.contains("ok: harness claude retry: model="), "{t}");
    assert!(t.contains("ok: harness claude escalation: model="), "{t}");
    assert!(!t.contains("harness codex worker"), "{t}");

    // an edited profile is what the probe reports
    let path = env.home.join("config.toml");
    let shipped = config_toml(&env);
    let edited = shipped
        .replacen(
            "[harness.claude.worker]\nmodel = \"opus\"\neffort = \"medium\"",
            "[harness.claude.worker]\nmodel = \"haiku\"",
            1,
        )
        .replacen(
            "[harness.claude.retry]\nmodel = \"opus\"\neffort = \"high\"\n",
            "",
            1,
        );
    assert_ne!(edited, shipped);
    std::fs::write(&path, &edited).unwrap();
    let t = text(&probe(&env, &[], None));
    assert!(
        t.contains("ok: harness claude worker: model=haiku effort=(harness default)"),
        "{t}"
    );
    // no [harness.claude.retry]: retries run as the worker does, and there
    // is nothing separate to report or probe
    assert!(!t.contains("harness claude retry"), "{t}");

    // a typo'd key fails the probe by name
    std::fs::write(
        &path,
        edited.replacen(
            "model = \"haiku\"",
            "model = \"haiku\"\nefort = \"high\"",
            1,
        ),
    )
    .unwrap();
    let out = probe(&env, &[], None);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out).contains(
            "FAIL: ~/.vitrify/config.toml: `harness.claude.worker.efort` is not a known key"
        ),
        "{}",
        text(&out)
    );
}
