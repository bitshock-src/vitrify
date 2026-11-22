//! `vitrify check` — every read-only check over the seeded drift fixtures:
//! each finding surfaces under its source with its remediation, open
//! questions are notes, nothing is written, and the exit code is the worst
//! of the components. In a git repository it also lists the dirty units:
//! Verified units a committed change reached, with the paths that did it.

mod common;

use std::collections::BTreeMap;
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
    copy_dir(&repo_root().join(fixture), tmp.path());
    let canon = tmp.path().canonicalize().unwrap();
    (tmp, canon)
}

/// vitrify in `dir`, with a temp root of its own that goes when the command
/// does (audit check 4 runs the fixture's commands, each in a temp directory).
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

/// `check --json` on `root`: the exit code and the parsed report.
fn check_json(root: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    let out = vitrify(root)
        .args(["check", "--json"])
        .args(args)
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

/// `(source, check, subject)` of every finding of one SDD's report.
fn found(report: &serde_json::Value) -> Vec<(String, Option<String>, String)> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["source"].as_str().unwrap().to_string(),
                x["check"].as_str().map(str::to_string),
                x["subject"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Every file under `root` with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

#[test]
fn clean_calc_is_one_clean_sdd() {
    let (_tmp, root) = scratch("fixtures/example");
    vitrify(&root)
        .args(["check"])
        .assert()
        .success()
        .stdout("check clean: 1 SDD\n");
    let (code, v) = check_json(&root, &[]);
    assert_eq!(code, 0);
    assert_eq!(
        v,
        serde_json::json!({
            "sdd": "calc",
            "root": "specs/calc",
            "clean": true,
            "findings": [],
            "notes": [],
            "dirty": []
        })
    );
}

#[test]
fn every_seeded_audit_finding_surfaces_with_its_source_and_remediation() {
    for fixture in [
        "audit-orphan",
        "audit-ghost",
        "audit-escape",
        "audit-rotten",
        "audit-trace-drift",
        "audit-projection",
        "audit-lease",
    ] {
        let (_tmp, root) = scratch(&format!("fixtures/seeded/{fixture}"));
        let before = snapshot(&root);
        let (code, v) = check_json(&root, &[]);
        assert_eq!(snapshot(&root), before, "{fixture}: check wrote something");
        let got = found(&v);
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("expected.json")).unwrap())
                .unwrap();
        for e in expected["findings"].as_array().unwrap() {
            let n = e["check"].as_i64().unwrap().to_string();
            let subject = e["subject"].as_str().unwrap().to_string();
            // the journal's mismatch is reported once, as the journal's
            let want = if fixture == "audit-projection" {
                ("journal".to_string(), None, subject)
            } else {
                ("audit".to_string(), Some(n), subject)
            };
            assert!(got.contains(&want), "{fixture}: {want:?} not in {got:?}");
        }
        for x in v["findings"].as_array().unwrap() {
            assert!(
                !x["remediation"].as_str().unwrap().is_empty(),
                "{fixture}: {x}"
            );
        }
        let mismatch = got.iter().any(|(s, _, _)| s == "journal" || s == "ledger");
        assert_eq!(code, if mismatch { 3 } else { 2 }, "{fixture}: {got:?}");
    }
}

#[test]
fn every_seeded_lint_finding_surfaces_under_lint() {
    for n in 1..=8 {
        let fixture = format!("lint-L{n}");
        let (_tmp, root) = scratch(&format!("fixtures/seeded/{fixture}"));
        let (code, v) = check_json(&root, &[]);
        assert_ne!(code, 0, "{fixture}");
        let got = found(&v);
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("expected.json")).unwrap())
                .unwrap();
        for e in expected["findings"].as_array().unwrap() {
            let want = (
                "lint".to_string(),
                Some(e["check"].as_str().unwrap().to_string()),
                e["subject"].as_str().unwrap().to_string(),
            );
            if n == 7 {
                // the SDD has build state: it is linted as a built SDD,
                // and L7 is not among that set
                assert!(!got.contains(&want), "{fixture}: {got:?}");
            } else {
                assert!(got.contains(&want), "{fixture}: {want:?} not in {got:?}");
            }
        }
    }
}

#[test]
fn text_lines_name_the_sdd_the_source_and_the_remediation() {
    let (_tmp, root) = scratch("fixtures/seeded/lint-L4");
    vitrify(&root)
        .args(["check"])
        .assert()
        .code(2)
        .stdout(
            "x: [lint L4] U-002: consumes unresolvable contract C-009 → write contract C-009, or remove it from U-002's consumes\n\
             x: [lint L4] C-001: its Consumers line names U-002, which does not list it in `consumes` → make C-001's Consumers line name exactly the units that list it in consumes\n\
             x: [audit 3] U-002: consumes nonexistent contract C-009 → add the contract (§7), or fix the unit's `consumes`, under a change entry (§12)\n\
             check: 3 findings in 1 of 1 SDD\n",
        );
}

#[test]
fn a_journal_mismatch_is_exit_3_and_reported_once() {
    let (_tmp, root) = scratch("fixtures/seeded/audit-projection");
    let out = vitrify(&root).args(["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains(
            "x: [journal] U-001: status: journal projects 'Claimed', front matter says 'Built' → "
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("[audit 6]"), "{stdout}");
}

#[test]
fn a_ledger_mismatch_is_exit_3_until_regenerated() {
    let (_tmp, root) = scratch("fixtures/seeded/audit-trace-drift");
    let out = vitrify(&root).args(["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains(
            "x: [ledger] ledger.md: holds no ledger → vitrify ledger regen --sdd specs/x\n"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("x: [audit 5] U-001: "), "{stdout}");
    // with the ledger written, the trace finding alone is left: exit 2
    vitrify(&root)
        .args(["ledger", "regen", "--sdd", "specs/x"])
        .assert()
        .success();
    let out = vitrify(&root).args(["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains("[ledger]"), "{stdout}");
    // the edited ledger is a mismatch again
    let ledger = root.join("specs/x/ledger.md");
    let text = std::fs::read_to_string(&ledger).unwrap();
    std::fs::write(&ledger, format!("{text}hand edit\n")).unwrap();
    let (code, v) = check_json(&root, &[]);
    assert_eq!(code, 3);
    let ledger_finding = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["source"] == "ledger")
        .unwrap();
    assert_eq!(ledger_finding["check"], serde_json::Value::Null);
    assert_eq!(ledger_finding["subject"], "ledger.md");
    assert_eq!(
        ledger_finding["remediation"],
        "vitrify ledger regen --sdd specs/x"
    );
}

#[test]
fn skip_rotten_leaves_out_audit_check_4() {
    let (_tmp, root) = scratch("fixtures/seeded/audit-rotten");
    let (_, v) = check_json(&root, &[]);
    assert!(found(&v).contains(&("audit".into(), Some("4".into()), "U-001".into())));
    let (_, v) = check_json(&root, &["--skip-rotten"]);
    assert!(
        !found(&v)
            .iter()
            .any(|(s, c, _)| s == "audit" && c.as_deref() == Some("4")),
        "{v}"
    );
}

#[test]
fn all_checks_every_sdd_and_reports_a_stale_pin_once() {
    let (_tmp, root) = scratch("fixtures/seeded/pin-multi/project");
    vitrify(&root)
        .args(["check", "--all"])
        .assert()
        .code(2)
        .stdout(
            "beta: [pin] C-002: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97) → vitrify pin sync --sdd specs/beta\n\
             check: 1 finding in 1 of 3 SDDs\n",
        );
    let (code, v) = check_json(&root, &["--all"]);
    assert_eq!(code, 2);
    let reports = v.as_array().unwrap();
    assert_eq!(reports.len(), 3);
    let names: Vec<&str> = reports.iter().map(|r| r["sdd"].as_str().unwrap()).collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
    assert_eq!(reports[1]["root"], "specs/beta");
    assert_eq!(reports[1]["clean"], false);
    assert_eq!(
        reports[1]["findings"],
        serde_json::json!([{
            "source": "pin",
            "check": null,
            "subject": "C-002",
            "cause": "its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97)",
            "remediation": "vitrify pin sync --sdd specs/beta"
        }])
    );
    assert_eq!(reports[2]["root"], "specs/gamma.sdd.md");
    assert_eq!(reports[2]["clean"], true);
    // the JSON objects lead with sdd and root, then clean, findings, notes
    let text = String::from_utf8(
        vitrify(&root)
            .args(["check", "--all", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let keys: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("    \""))
        .take(5)
        .map(|l| l.trim().split('"').nth(1).unwrap())
        .collect();
    assert_eq!(keys, ["sdd", "root", "clean", "findings", "notes"]);

    // once synced, the repository is clean
    vitrify(&root)
        .args(["pin", "sync", "--all"])
        .assert()
        .success();
    vitrify(&root)
        .args(["check", "--all"])
        .assert()
        .success()
        .stdout("check clean: 3 SDDs\n");
    // several SDDs and neither flag is a refusal
    vitrify(&root)
        .args(["check"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("multiple SDDs found"));
}

#[test]
fn open_questions_are_notes_not_findings() {
    let (_tmp, root) = scratch("fixtures/example");
    let decisions = root.join("specs/calc/decisions.md");
    let text = std::fs::read_to_string(&decisions).unwrap();
    std::fs::write(
        &decisions,
        format!(
            "{text}\n- **Q-007** (2026-11-01, open) — reported by w1 on U-002: which rounding?\n- **Q-008** (2026-11-01, resolved 2026-11-02) — settled.\n"
        ),
    )
    .unwrap();
    vitrify(&root)
        .args(["check"])
        .assert()
        .success()
        .stdout("calc: note: Q-007 is open (blocks U-002)\ncheck clean: 1 SDD\n");
    let (code, v) = check_json(&root, &[]);
    assert_eq!(code, 0);
    assert_eq!(v["clean"], true);
    assert_eq!(
        v["notes"],
        serde_json::json!(["Q-007 is open (blocks U-002)"])
    );
}

#[test]
fn dirty_units_are_listed_with_the_paths_that_made_them_dirty() {
    let p = common::Project::new();
    let dirty_lines = |out: &str| -> Vec<String> {
        out.lines()
            .filter(|l| l.contains(": dirty "))
            .map(str::to_string)
            .collect()
    };
    // freshly verified: nothing dirty, and no count
    let (_, out, _) = p.vitrify(&["check", "--all", "--skip-rotten"]);
    assert!(dirty_lines(&out).is_empty(), "{out}");
    assert!(!out.contains("dirty"), "{out}");

    // a foundation change reaches its SDD and, through a pin, beta U-001;
    // an anchor git cannot resolve makes beta U-002 dirty
    p.write("a/one.txt", "one, again\n");
    p.event(
        "specs/beta",
        "verified",
        Some("U-002"),
        "integrator",
        "deadbee",
    );
    p.commit("foundation");
    let before = snapshot(&p.root);
    let (_, out, _) = p.vitrify(&["check", "--all", "--skip-rotten"]);
    assert_eq!(snapshot(&p.root), before, "check wrote something");
    assert_eq!(
        dirty_lines(&out),
        [
            "alpha: dirty U-001: a/one.txt (unit alpha U-001)",
            "alpha: dirty U-002: a/one.txt (unit alpha U-001)",
            "beta: dirty U-001: a/one.txt (unit alpha U-001)",
            "beta: dirty U-002: anchor deadbee cannot be resolved (rewritten history, or a shallow clone)",
        ],
        "{out}"
    );
    assert!(out.ends_with("; 4 units dirty\n"), "{out}");

    let (_, v) = check_json(&p.root, &["--all", "--skip-rotten"]);
    let anchor = p.anchor.as_str();
    assert_eq!(
        v[0]["dirty"],
        serde_json::json!([
            {"unit": "U-001", "anchor": anchor, "why": "changed",
             "paths": [{"path": "a/one.txt", "owner": "unit alpha U-001"}]},
            {"unit": "U-002", "anchor": anchor, "why": "changed",
             "paths": [{"path": "a/one.txt", "owner": "unit alpha U-001"}]}
        ])
    );
    assert_eq!(
        v[1]["dirty"][1],
        serde_json::json!({"unit": "U-002", "anchor": "deadbee", "why": "unresolved-anchor", "paths": []})
    );
    // one SDD: its own dirty units only
    let (_, v) = check_json(&p.root, &["--sdd", "specs/beta", "--skip-rotten"]);
    let units: Vec<&str> = v["dirty"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["unit"].as_str().unwrap())
        .collect();
    assert_eq!(units, ["U-001", "U-002"]);
}

#[test]
fn no_sdd_is_exit_4() {
    let tmp = tempfile::tempdir().unwrap();
    vitrify(tmp.path()).args(["check"]).assert().code(4);
    vitrify(tmp.path())
        .args(["check", "--all"])
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
