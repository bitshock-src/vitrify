//! Trace record/verify, stamps, ledger regen/progress,
//! journal append/replay — on scratch copies of the calc example fixture.

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
fn record_writes_golden_hash_map_into_front_matter() {
    let (_tmp, root) = scratch_example();
    let out = vitrify(&root)
        .args(["trace", "record", "--unit", "U-003", "--stamp", "built"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let grec: serde_json::Value =
        serde_json::from_str(&golden("trace-U-003.json").replace("{ROOT}", "")).unwrap();
    assert_eq!(parsed["recorded"], grec["recorded"]);
    assert_eq!(parsed["stamp"], "built");

    // front matter carries the map, aligned exactly as the trace writer aligns it
    let unit_text =
        std::fs::read_to_string(root.join("specs/calc/units/U-003-tokenizer.md")).unwrap();
    assert!(unit_text.contains("trace:\n  unit:   de819ef7af80"));
    assert!(unit_text.contains("\n  global: b546f01e00cb\n"));

    // and verify is clean
    vitrify(&root)
        .args(["trace", "verify", "--unit", "U-003"])
        .assert()
        .success()
        .stdout("U-003: trace matches\n");
}

#[test]
fn tampered_contract_fails_verify_naming_the_key() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["trace", "record", "--unit", "U-003", "--stamp", "built"])
        .assert()
        .success();
    let cpath = root.join("specs/calc/contracts/C-002-token-stream.md");
    let mut c = std::fs::read_to_string(&cpath).unwrap();
    c.push_str("\ntampered\n");
    std::fs::write(&cpath, c).unwrap();

    vitrify(&root)
        .args(["trace", "verify", "--unit", "U-003"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains(
            "U-003: STALE — mismatched keys: C-002",
        ));

    let out = vitrify(&root)
        .args(["trace", "verify", "--unit", "U-003", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["match"], false);
    assert_eq!(v["mismatches"][0]["key"], "C-002");
}

#[test]
fn stamp_semantics_and_claim_seq_preservation() {
    let (_tmp, root) = scratch_example();
    // seed a claim state on U-002 by hand
    let upath = root.join("specs/calc/units/U-002-errors.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    let text = text
        .replace("status: Ready", "status: Claimed")
        .replace("claimed_by: —", "claimed_by: w1")
        .replace("claim_seq: 0", "claim_seq: 3");
    std::fs::write(&upath, text).unwrap();

    vitrify(&root)
        .args([
            "trace", "record", "--unit", "U-002", "--stamp", "built", "--commit", "abc1234",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"stamp\": \"built\""));
    let text = std::fs::read_to_string(&upath).unwrap();
    assert!(text.contains("status: Built"));
    assert!(text.contains("@ SDD 1.0.0, commit abc1234"));
    assert!(text.contains("claim_seq: 3"), "claim_seq must never reset");
    assert!(text.contains("claimed_by: w1"), "built keeps the holder");

    vitrify(&root)
        .args([
            "trace", "record", "--unit", "U-002", "--stamp", "verified", "--commit", "def5678",
        ])
        .assert()
        .success();
    let text = std::fs::read_to_string(&upath).unwrap();
    assert!(text.contains("status: Verified"));
    assert!(text.contains("verified: "));
    assert!(text.contains("claimed_by: —"), "verified clears the holder");
    assert!(text.contains("claim_expires: —"));
    assert!(text.contains("claim_seq: 3"), "claim_seq must never reset");

    // stamp-invariance at the CLI level: re-record and verify stays clean
    vitrify(&root)
        .args(["trace", "record", "--unit", "U-002", "--stamp", "verified"])
        .assert()
        .success();
    vitrify(&root)
        .args(["trace", "verify", "--unit", "U-002"])
        .assert()
        .success();
}

#[test]
fn record_missing_contract_exits_4() {
    let (_tmp, root) = scratch_example();
    std::fs::remove_file(root.join("specs/calc/contracts/C-002-token-stream.md")).unwrap();
    vitrify(&root)
        .args(["trace", "record", "--unit", "U-003", "--stamp", "built"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("consumed contract missing: C-002"));
}

#[test]
fn verify_without_recorded_trace_exits_4() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["trace", "verify", "--unit", "U-002"]) // trace: — (unset)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("no trace recorded for U-002"));
}

#[test]
fn ledger_regen_matches_golden_and_progress_prints() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["ledger", "regen"])
        .assert()
        .success()
        .stdout("Progress: 1/5 Verified · 4 Ready\n")
        .stderr(predicate::str::contains("regenerated"));
    let written = std::fs::read_to_string(root.join("specs/calc/ledger.md")).unwrap();
    let today = chrono_free_today();
    assert_eq!(golden("ledger.md").replace("{DATE}", &today), written);

    vitrify(&root)
        .args(["ledger", "progress"])
        .assert()
        .success()
        .stdout(golden("progress.txt"));
}

/// Today's UTC date without a chrono dependency: derive it from `date -u`.
fn chrono_free_today() -> String {
    let out = Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn journal_append_then_replay_verify_clean() {
    let (_tmp, root) = scratch_example();
    // the pristine example replays clean
    vitrify(&root)
        .args(["journal", "replay"])
        .assert()
        .success()
        .stdout("journal consistent (4 events)\n");

    // append a claim for U-002 and record it in front matter
    let out = vitrify(&root)
        .args([
            "journal",
            "append",
            "--event",
            "claim",
            "--unit",
            "U-002",
            "--actor",
            "w1",
            "--claim-seq",
            "1",
            "--commit",
            "abc1234",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let line = String::from_utf8(out.stdout).unwrap();
    assert!(line.starts_with(r#"{"seq":5,"at":""#), "line: {line}");
    assert!(line.contains(r#""event":"claim","unit":"U-002","actor":"w1","claim_seq":1,"sdd":"1.0.0","commit":"abc1234"}"#));

    // the projection disagrees with front matter (still Ready) → exit 3
    vitrify(&root)
        .args(["journal", "replay"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains(
            "mismatch U-002.status: journal projects 'Claimed', front matter says 'Ready'",
        ));

    // fix the front matter → clean again
    let upath = root.join("specs/calc/units/U-002-errors.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    let text = text
        .replace("status: Ready", "status: Claimed")
        .replace("claim_seq: 0", "claim_seq: 1");
    std::fs::write(&upath, text).unwrap();
    vitrify(&root)
        .args(["journal", "replay"])
        .assert()
        .success();
}

#[test]
fn replay_json_matches_golden_on_pristine_example() {
    let (_tmp, root) = scratch_example();
    vitrify(&root)
        .args(["journal", "replay", "--json"])
        .assert()
        .success()
        .stdout(golden("replay.json"));
}

#[test]
fn journal_missing_exits_4_and_unit_required_exits_2() {
    let (_tmp, root) = scratch_example();
    std::fs::remove_file(root.join("specs/calc/journal.ndjson")).unwrap();
    vitrify(&root)
        .args(["journal", "replay"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("no journal found"));

    vitrify(&root)
        .args(["journal", "append", "--event", "claim"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--event claim requires --unit"));
}

#[test]
fn regen_check_compares_and_writes_nothing() {
    let (_tmp, root) = scratch_example();
    let ledger = root.join("specs/calc/ledger.md");
    let committed = std::fs::read_to_string(&ledger).unwrap();
    // the committed ledger was derived on another day: the date is the one
    // thing a regeneration always changes, and the one thing not compared
    assert!(!committed.contains(&chrono_free_today()));
    vitrify(&root)
        .args(["ledger", "regen", "--check"])
        .assert()
        .success()
        .stdout("ledger current (specs/calc/ledger.md)\n");
    assert_eq!(std::fs::read_to_string(&ledger).unwrap(), committed);

    // front matter moves on and the ledger is not regenerated
    let upath = root.join("specs/calc/units/U-002-errors.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    std::fs::write(&upath, text.replace("status: Ready", "status: Stale")).unwrap();
    vitrify(&root)
        .args(["ledger", "regen", "--check"])
        .assert()
        .code(3)
        .stdout(predicate::str::starts_with(
            "ledger differs from its derivation at specs/calc/ledger.md:3 — run `vitrify ledger regen`\n  derived: Progress: 1/5 Verified · 3 Ready · 1 Stale\n  on disk: Progress: 1/5 Verified · 4 Ready\n",
        ));
    assert_eq!(
        std::fs::read_to_string(&ledger).unwrap(),
        committed,
        "--check never writes"
    );
    // regenerated, it is current again
    vitrify(&root).args(["ledger", "regen"]).assert().success();
    vitrify(&root)
        .args(["ledger", "regen", "--check"])
        .assert()
        .success();

    // a version bump the ledger has not followed
    let spath = root.join("specs/calc/sdd.md");
    let text = std::fs::read_to_string(&spath).unwrap();
    std::fs::write(&spath, text.replace("version: 1.0.0", "version: 1.0.1")).unwrap();
    vitrify(&root)
        .args(["ledger", "regen", "--check"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("specs/calc/ledger.md:1"))
        .stdout(predicate::str::contains(
            "derived: # Ledger — calc @ SDD 1.0.1 (derived)",
        ));

    // no ledger at all is not a ledger that matches
    std::fs::remove_file(&ledger).unwrap();
    vitrify(&root)
        .args(["ledger", "regen", "--check"])
        .assert()
        .code(3)
        .stdout("ledger missing: specs/calc/ledger.md holds none — run `vitrify ledger regen`\n");
    assert!(!ledger.exists());
}

#[test]
fn ledger_and_audit_take_every_sdd_with_all() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/run-multi/project"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["ledger", "progress", "--all"])
        .assert()
        .success()
        .stdout(
            "alpha: Progress: 0/2 Verified · 2 Ready\nbeta: Progress: 0/2 Verified · 2 Ready\n",
        );
    // one SDD is still the default, and several without a flag are refused
    vitrify(&root)
        .args(["ledger", "progress"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("multiple SDDs found"));
    vitrify(&root)
        .args(["ledger", "progress", "--all", "--sdd", "specs/alpha"])
        .assert()
        .code(2);

    // neither SDD has a ledger yet
    vitrify(&root)
        .args(["ledger", "regen", "--check", "--all"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains(
            "alpha: ledger missing: specs/alpha/ledger.md holds none",
        ))
        .stdout(predicate::str::contains(
            "beta: ledger missing: specs/beta/ledger.md holds none",
        ));
    vitrify(&root)
        .args(["ledger", "regen", "--all"])
        .assert()
        .success()
        .stdout(
            "alpha: Progress: 0/2 Verified · 2 Ready\nbeta: Progress: 0/2 Verified · 2 Ready\n",
        );
    vitrify(&root)
        .args(["ledger", "regen", "--check", "--all"])
        .assert()
        .success()
        .stdout("alpha: ledger current (specs/alpha/ledger.md)\nbeta: ledger current (specs/beta/ledger.md)\n");

    vitrify(&root)
        .args(["audit", "--all"])
        .assert()
        .success()
        .stdout("alpha: audit clean (7/7 checks)\nbeta: audit clean (7/7 checks)\n");
    // a finding in one is reported under its name; the other is still audited
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(root.join("b/stray.txt"), "nobody owns this\n").unwrap();
    let out = vitrify(&root).args(["audit", "--all"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with("alpha: audit clean (7/7 checks)\nbeta: [check 1] b/stray.txt: "),
        "{stdout}"
    );
    let out = vitrify(&root)
        .args(["audit", "--all", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["sdd"], "alpha");
    assert_eq!(v[0]["clean"], true);
    assert_eq!(v[1]["root"], "specs/beta");
    assert_eq!(v[1]["findings"][0]["check"], 1);
}

#[test]
fn regen_check_reads_a_single_file_sdds_ledger_section() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/pin-multi/project"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    let sdd = root.join("specs/gamma.sdd.md");
    let before = std::fs::read_to_string(&sdd).unwrap();
    // no State Ledger section yet
    vitrify(&root)
        .args(["ledger", "regen", "--check", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .code(3)
        .stdout(predicate::str::starts_with(
            "ledger missing: specs/gamma.sdd.md",
        ));
    assert_eq!(std::fs::read_to_string(&sdd).unwrap(), before);
    vitrify(&root)
        .args(["ledger", "regen", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .success();
    vitrify(&root)
        .args(["ledger", "regen", "--check", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .success()
        .stdout("ledger current (specs/gamma.sdd.md)\n");
    // the section goes stale with the unit it was derived from
    let text = std::fs::read_to_string(&sdd).unwrap();
    std::fs::write(&sdd, text.replace("status: Ready", "status: Draft")).unwrap();
    vitrify(&root)
        .args(["ledger", "regen", "--check", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains(
            "derived: Progress: 0/1 Verified · 1 Draft",
        ));
}

#[test]
fn replay_all_reports_every_sdd_under_its_name() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/pin-multi/project"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    // no SDD has a journal yet: reported, and nothing fails
    vitrify(&root)
        .args(["journal", "replay", "--all"])
        .assert()
        .success()
        .stdout("alpha: no journal yet\nbeta: no journal yet\ngamma: no journal yet\n");

    vitrify(&root)
        .args([
            "journal",
            "append",
            "--event",
            "audit",
            "--sdd",
            "specs/alpha",
        ])
        .assert()
        .success();
    vitrify(&root)
        .args(["journal", "replay", "--all"])
        .assert()
        .success()
        .stdout(
            "alpha: journal consistent (1 events)\nbeta: no journal yet\ngamma: no journal yet\n",
        );

    // a claim the front matter does not show: a mismatch, exit 3
    vitrify(&root)
        .args([
            "journal",
            "append",
            "--event",
            "claim",
            "--unit",
            "U-001",
            "--claim-seq",
            "1",
            "--sdd",
            "specs/beta",
        ])
        .assert()
        .success();
    let out = vitrify(&root)
        .args(["journal", "replay", "--all"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("alpha: journal consistent (1 events)\n"));
    assert!(
        stdout.contains(
            "beta: mismatch U-001.status: journal projects 'Claimed', front matter says 'Ready'\n"
        ),
        "{stdout}"
    );
    assert!(stdout.ends_with("gamma: no journal yet\n"), "{stdout}");

    let out = vitrify(&root)
        .args(["journal", "replay", "--all", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let text = String::from_utf8(out.stdout).unwrap();
    let keys: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("    \""))
        .take(7)
        .map(|l| l.trim().split('"').nth(1).unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "sdd",
            "root",
            "events",
            "consistent",
            "order_findings",
            "mismatches",
            "projection"
        ]
    );
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v[0]["sdd"], "alpha");
    assert_eq!(v[0]["root"], "specs/alpha");
    assert_eq!(v[0]["consistent"], true);
    assert_eq!(v[1]["consistent"], false);
    assert_eq!(v[1]["projection"]["U-001"], "Claimed");
    assert_eq!(v[1]["mismatches"][0]["unit"], "U-001");
    // an SDD with no journal: nothing replayed, nothing judged
    assert_eq!(v[2]["sdd"], "gamma");
    assert_eq!(v[2]["events"], 0);
    assert_eq!(v[2]["consistent"], serde_json::Value::Null);

    // one SDD: the JSON carries the projection; no journal is exit 4
    vitrify(&root)
        .args(["journal", "replay", "--sdd", "specs/alpha", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"projection\""));
    vitrify(&root)
        .args(["journal", "replay", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .code(4);
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
