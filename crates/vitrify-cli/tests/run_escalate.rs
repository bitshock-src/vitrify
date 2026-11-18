//! Fake harness returning each verdict; malformed-once
//! (re-ask with the parse error appended); malformed-twice → abort; block
//! requires q; evidence recorded.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use vitrify_cli::run::escalate::{Evidence, VerdictKind, build_prompt, escalate};

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    logs: PathBuf,
}

fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("vhome");
    let bin = tmp.path().join("bin");
    let logs = tmp.path().join("logs");
    std::fs::create_dir_all(&bin).unwrap();
    // a home `vitrify init` set up: the escalation template resolves from it
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), vitrify_core::content::CONFIG.body).unwrap();
    Env {
        home,
        bin,
        logs,
        _tmp: tmp,
    }
}

fn fake(env: &Env, name: &str, script: &str) -> Vec<String> {
    let path = env.bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![path.to_string_lossy().to_string()]
}

fn evidence() -> Evidence {
    Evidence {
        unit: "U-003".to_string(),
        branch: "sdd/U-003/c2".to_string(),
        fencing: "branch token c2; current claim_seq 2; lease to 2026-10-30T13:00:00Z".to_string(),
        command: "cargo test --locked".to_string(),
        output_tail: (0..150)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
        journal_tail: (0..30)
            .map(|i| format!("{{\"seq\":{i},\"event\":\"claim\"}}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn prompt_fills_slots_and_truncates_tails() {
    let env = setup();
    let prompt = build_prompt(&env.home, VERSION, &evidence()).unwrap();
    assert!(prompt.contains("Unit: U-003"));
    assert!(prompt.contains("Branch: sdd/U-003/c2"));
    assert!(prompt.contains("branch token c2"));
    assert!(prompt.contains("cargo test --locked"));
    // output tail truncated to the last 100 lines
    assert!(!prompt.contains("line 49\n"));
    assert!(prompt.contains("line 50"));
    assert!(prompt.contains("line 149"));
    // journal tail truncated to the last 20 events
    assert!(!prompt.contains("\"seq\":9,"));
    assert!(prompt.contains("\"seq\":10,"));
    // the verbatim required-reply schema
    assert!(prompt.contains(r#"{"verdict":"retry|release|block|abort""#));
}

#[test]
fn each_verdict_parses_and_is_recorded() {
    let env = setup();
    let cases = [
        ("retry", VerdictKind::Retry),
        ("release", VerdictKind::Release),
        ("abort", VerdictKind::Abort),
    ];
    for (i, (name, kind)) in cases.iter().enumerate() {
        let argv = fake(
            &env,
            &format!("model-{name}"),
            &format!(
                r#"echo "Reasoning about it..."
echo '{{"verdict":"{name}","unit":"U-003","q":null,"reason":"because"}}'"#
            ),
        );
        let (verdict, path) =
            escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, i as u32).unwrap();
        assert_eq!(verdict.verdict, *kind);
        assert_eq!(verdict.reason, "because");
        let record = std::fs::read_to_string(&path).unwrap();
        assert!(record.contains(&format!("\"verdict\": \"{name}\"")));
        assert!(record.contains("cargo test --locked"));
    }
    // block carries its q
    let argv = fake(
        &env,
        "model-block",
        r#"echo '{"verdict":"block","unit":"U-003","q":"Q-007","reason":"spec gap"}'"#,
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 9).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Block);
    assert_eq!(verdict.q.as_deref(), Some("Q-007"));
}

#[test]
fn envelope_wrapped_verdict_parses() {
    let env = setup();
    // a stream of one `result` event wraps the whole reply in a one-line
    // JSON envelope whose `result` field carries the assistant text —
    // reasoning prose with the verdict object as its last line.
    let argv = fake(
        &env,
        "model-envelope",
        r#"printf '%s\n' '{"type":"result","is_error":false,"result":"Reasoning first.\n{\"verdict\":\"release\",\"unit\":\"U-003\",\"q\":null,\"reason\":\"lease lapsed\"}"}'"#,
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 42).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Release);
    assert_eq!(verdict.reason, "lease lapsed");

    // result that IS exactly the verdict object, no prose
    let argv = fake(
        &env,
        "model-envelope-bare",
        r#"printf '%s\n' '{"type":"result","is_error":false,"result":"{\"verdict\":\"abort\",\"unit\":null,\"q\":null,\"reason\":\"operator review\"}"}'"#,
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 43).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Abort);
    assert_eq!(verdict.reason, "operator review");
}

#[test]
fn fenced_verdict_parses() {
    let env = setup();
    // models habitually wrap the verdict in a markdown code fence
    let argv = fake(
        &env,
        "model-fenced",
        r#"printf '%s\n' 'Reasoning.' '' '```json' '{"verdict":"retry","unit":"U-003","q":null,"reason":"transient"}' '```'"#,
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 44).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Retry);
    assert_eq!(verdict.reason, "transient");
}

#[test]
fn malformed_once_gets_one_reask_with_the_parse_error() {
    let env = setup();
    let marker = env.bin.join("asked-once");
    let argv = fake(
        &env,
        "model-flaky",
        &format!(
            r#"if [ -f "{m}" ]; then
  case "$1" in *"could not be parsed"*) ;; *) echo "re-ask prompt missing the parse error" >&2; exit 1;; esac
  echo '{{"verdict":"retry","unit":"U-003","q":null,"reason":"second try"}}'
else
  touch "{m}"
  echo 'not json at all'
fi"#,
            m = marker.display()
        ),
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 0).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Retry);
    assert_eq!(verdict.reason, "second try");
}

#[test]
fn malformed_twice_aborts() {
    let env = setup();
    let argv = fake(&env, "model-garbage", "echo 'never json'");
    let (verdict, path) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 0).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Abort);
    assert!(verdict.reason.contains("malformed escalation reply twice"));
    // both replies recorded
    let record = std::fs::read_to_string(&path).unwrap();
    assert_eq!(record.matches("never json").count(), 2);
}

#[test]
fn block_without_q_is_malformed() {
    let env = setup();
    // both replies are block-without-q → abort after the re-ask
    let argv = fake(
        &env,
        "model-blocknoq",
        r#"echo '{"verdict":"block","unit":"U-003","q":null,"reason":"no q"}'"#,
    );
    let (verdict, _) = escalate(&env.home, VERSION, &argv, &evidence(), &env.logs, 0).unwrap();
    assert_eq!(verdict.verdict, VerdictKind::Abort);
}

// keep Path used in signatures
#[allow(dead_code)]
fn _unused(_: &Path) {}
