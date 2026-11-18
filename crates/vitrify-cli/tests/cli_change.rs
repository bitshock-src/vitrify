//! `vitrify change apply` on copies of the calc example and the pin-multi
//! fixture: a PATCH entry re-stamps, a MINOR entry marks Stale, a recorded
//! entry is skipped, a dry run writes nothing, and the refusals.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::prelude::*;

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

/// A scratch copy of `fixtures/<rel>`.
fn scratch(rel: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures").join(rel), tmp.path());
    let canon = tmp.path().canonicalize().unwrap();
    (tmp, canon)
}

fn vitrify(dir: &Path, args: &[&str]) -> Output {
    Command::cargo_bin("vitrify")
        .unwrap()
        .current_dir(dir)
        .env("VITRIFY_HOME", home())
        .args(args)
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Every file under `dir` and its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn edit(path: &Path, from: &str, to: &str) {
    let t = std::fs::read_to_string(path).unwrap();
    assert!(t.contains(from), "{} lacks {from:?}", path.display());
    std::fs::write(path, t.replacen(from, to, 1)).unwrap();
}

fn append(path: &Path, more: &str) {
    let mut t = std::fs::read_to_string(path).unwrap_or_default();
    t.push_str(more);
    std::fs::write(path, t).unwrap();
}

const U001: &str = "specs/calc/units/U-001-foundation.md";

/// The calc example with U-001's text edited and a change entry `id`
/// (`from → to`, dated `date`) appended, `version` and `updated` set to it
/// the way schema §12 says.
fn calc_with_entry(id: &str, from: &str, to: &str, date: &str) -> (tempfile::TempDir, PathBuf) {
    let (tmp, dir) = scratch("example");
    edit(
        &dir.join(U001),
        "THE SYSTEM SHALL provide the package `calc`",
        "THE SYSTEM SHALL provide the Python package `calc`",
    );
    append(
        &dir.join("specs/calc/changelog.md"),
        &format!(
            "\n## {id} — U-001 package wording ({date}) — {from} → {to}\n\n- Modified: U-001\n"
        ),
    );
    let sdd = dir.join("specs/calc/sdd.md");
    edit(&sdd, &format!("version: {from}"), &format!("version: {to}"));
    edit(&sdd, "updated: 2026-10-28", &format!("updated: {date}"));
    (tmp, dir)
}

fn journal_lines(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("specs/calc/journal.ndjson"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn assert_consistent(dir: &Path) {
    for args in [
        &["audit", "--skip-rotten"][..],
        &["journal", "replay"],
        &["ledger", "regen", "--check"],
        &["lint", "--built"],
    ] {
        let out = vitrify(dir, args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            text(&out.stdout),
            text(&out.stderr)
        );
    }
}

#[test]
fn a_patch_entry_restamps_the_moved_traces() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.0.1", "2026-11-17");
    let unit_before = std::fs::read_to_string(dir.join(U001)).unwrap();
    assert_eq!(
        vitrify(&dir, &["trace", "verify", "--unit", "U-001"])
            .status
            .code(),
        Some(3)
    );

    let out = vitrify(&dir, &["change", "apply", "--commit", "abc1234"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "calc: CH-002 1.0.0 → 1.0.1 (patch): traces of U-001 re-recorded\n\
         calc: - Staleness applied: —\n"
    );
    assert_eq!(
        text(&out.stderr),
        "next: copy the Staleness applied line into CH-002 (specs/calc/changelog.md), then run vitrify check\n"
    );

    // the trace matches; status and stamps are kept
    assert!(
        vitrify(&dir, &["trace", "verify", "--unit", "U-001"])
            .status
            .success()
    );
    let unit_after = std::fs::read_to_string(dir.join(U001)).unwrap();
    for key in ["status:", "built:", "verified:", "claim_seq:"] {
        let line = |t: &str| t.lines().find(|l| l.starts_with(key)).unwrap().to_string();
        assert_eq!(line(&unit_before), line(&unit_after), "{key}");
    }

    let events = journal_lines(&dir);
    let last = events.last().unwrap();
    assert_eq!(last["event"], "change");
    assert_eq!(last["entry"], "CH-002");
    assert_eq!(last["sdd"], "1.0.1");
    assert_eq!(last["commit"], "abc1234");
    assert!(last.get("stale").is_none());
    assert_eq!(events.len(), 5);

    let ledger = std::fs::read_to_string(dir.join("specs/calc/ledger.md")).unwrap();
    assert!(
        ledger.starts_with("# Ledger — calc @ SDD 1.0.1"),
        "{ledger}"
    );
    assert_consistent(&dir);
}

#[test]
fn a_minor_entry_marks_the_moved_units_stale() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.1.0", "2026-11-17");
    let out = vitrify(&dir, &["change", "apply", "--sdd", "specs/calc"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "calc: CH-002 1.0.0 → 1.1.0 (minor): U-001 marked Stale\n\
         calc: - Staleness applied: U-001 → Stale\n"
    );
    assert!(text(&out.stderr).starts_with("next: copy the Staleness applied line into CH-002"));

    let unit = std::fs::read_to_string(dir.join(U001)).unwrap();
    assert!(unit.contains("\nstatus: Stale\n"), "{unit}");
    let events = journal_lines(&dir);
    let n = events.len();
    assert_eq!(events[n - 2]["event"], "change");
    assert_eq!(events[n - 2]["entry"], "CH-002");
    assert_eq!(events[n - 2]["stale"], serde_json::json!(["U-001"]));
    assert_eq!(events[n - 1]["event"], "stale");
    assert_eq!(events[n - 1]["unit"], "U-001");
    assert_eq!(
        events[n - 1]["cause"],
        "recorded trace differs from current content: unit"
    );
    assert!(events[n - 1]["seq"].as_i64() > events[n - 2]["seq"].as_i64());
    assert_consistent(&dir);

    // a second run finds the entry recorded and changes nothing
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stdout),
        "calc: CH-002 already recorded — nothing to apply\n"
    );
    assert_eq!(text(&out.stderr), "");
    assert!(tree(&dir) == before, "a second run wrote");
}

#[test]
fn a_dry_run_prints_the_same_and_writes_nothing() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "2.0.0", "2026-11-17");
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "calc: CH-002 1.0.0 → 2.0.0 (major): U-001 marked Stale\n\
         calc: - Staleness applied: U-001 → Stale\n"
    );
    assert_eq!(text(&out.stderr), "dry run: nothing written\n");
    assert!(tree(&dir) == before, "a dry run wrote");

    // the patch path too
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.0.1", "2026-11-17");
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["restamped"], serde_json::json!(["U-001"]));
    assert_eq!(v["dry_run"], true);
    assert!(tree(&dir) == before, "a dry run wrote");
}

#[test]
fn json_reports_one_object() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.1.0", "2026-11-17");
    let out = vitrify(&dir, &["change", "apply", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    let mut want = vec![
        "sdd",
        "root",
        "entry",
        "from",
        "to",
        "bump",
        "action",
        "restamped",
        "stale",
        "staleness_line",
        "dry_run",
    ];
    want.sort();
    let mut got = keys.clone();
    got.sort();
    assert_eq!(got, want);
    // fields in the documented order
    let raw = text(&out.stdout);
    let pos: Vec<usize> = [
        "\"sdd\"",
        "\"root\"",
        "\"entry\"",
        "\"from\"",
        "\"to\"",
        "\"bump\"",
        "\"action\"",
        "\"restamped\"",
        "\"stale\"",
        "\"staleness_line\"",
        "\"dry_run\"",
    ]
    .iter()
    .map(|k| raw.find(k).unwrap())
    .collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{raw}");
    assert_eq!(v["sdd"], "calc");
    assert_eq!(v["root"], "specs/calc");
    assert_eq!(v["entry"], "CH-002");
    assert_eq!(v["from"], "1.0.0");
    assert_eq!(v["to"], "1.1.0");
    assert_eq!(v["bump"], "minor");
    assert_eq!(v["action"], "applied");
    assert_eq!(v["restamped"], serde_json::json!([]));
    assert_eq!(v["stale"], serde_json::json!(["U-001"]));
    assert_eq!(v["staleness_line"], "- Staleness applied: U-001 → Stale");
    assert_eq!(v["dry_run"], false);

    let out = vitrify(&dir, &["change", "apply", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "already-recorded");
    assert_eq!(v["staleness_line"], serde_json::Value::Null);
}

#[test]
fn a_version_or_updated_mismatch_refuses() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.1.0", "2026-11-17");
    // the entry is written, the front matter left behind
    edit(
        &dir.join("specs/calc/sdd.md"),
        "version: 1.1.0",
        "version: 1.0.0",
    );
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(text(&out.stdout), "");
    let err = text(&out.stderr);
    assert!(err.starts_with("error: calc: version: "), "{err}");
    assert!(err.contains("CH-002"), "{err}");
    assert!(tree(&dir) == before, "a refusal wrote");

    // the date too
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.1.0", "2026-11-17");
    edit(
        &dir.join("specs/calc/sdd.md"),
        "updated: 2026-11-17",
        "updated: 2026-11-16",
    );
    let out = vitrify(&dir, &["change", "apply"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).starts_with("error: calc: updated: "));
}

#[test]
fn a_stale_pin_refuses() {
    let (_tmp, dir) = scratch("seeded/pin-multi/project");
    std::fs::write(
        dir.join("specs/beta/changelog.md"),
        "# Changelog\n\n## CH-001 — Activation — 0.1.0 → 1.0.0\n",
    )
    .unwrap();
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply", "--sdd", "specs/beta"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        text(&out.stderr),
        "error: beta: C-002's pin is stale → vitrify pin sync --sdd specs/beta first\n"
    );
    assert!(tree(&dir) == before, "a refusal wrote");
}

#[test]
fn a_draft_sdd_or_one_without_an_entry_gets_nothing() {
    let (_tmp, dir) = calc_with_entry("CH-002", "1.0.0", "1.1.0", "2026-11-17");
    edit(
        &dir.join("specs/calc/sdd.md"),
        "status: Active",
        "status: Draft",
    );
    let before = tree(&dir);
    let out = vitrify(&dir, &["change", "apply"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout), "calc: Draft — nothing to apply\n");
    assert!(tree(&dir) == before, "a Draft SDD was written");
    let out = vitrify(&dir, &["change", "apply", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "draft");
    assert_eq!(v["bump"], serde_json::Value::Null);

    let (_tmp, dir) = scratch("example");
    std::fs::remove_file(dir.join("specs/calc/changelog.md")).unwrap();
    let out = vitrify(&dir, &["change", "apply"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stdout),
        "calc: no change entry — nothing to apply\n"
    );
}

#[test]
fn a_single_files_entry_outside_its_changelog_is_refused_and_named() {
    let (_tmp, dir) = scratch("seeded/pin-multi/project");
    let gamma = dir.join("specs/gamma.sdd.md");
    // a `##` entry ends the section it was meant for: no entry is read
    append(
        &gamma,
        "\n## Changelog\n\n## CH-001 — Activation — 0.1.0 → 1.0.0\n",
    );
    let before = tree(&dir);
    let refusal = "error: gamma: CH-001: a change entry outside `## Changelog` is not read: \
                   `## CH-001` ends that section → write it as `### CH-001 — …` under \
                   `## Changelog`, then run vitrify change apply --sdd specs/gamma.sdd.md\n";
    for args in [
        &["change", "apply", "--sdd", "specs/gamma.sdd.md"][..],
        &[
            "change",
            "apply",
            "--sdd",
            "specs/gamma.sdd.md",
            "--dry-run",
        ],
    ] {
        let out = vitrify(&dir, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(text(&out.stdout), "");
        assert!(
            text(&out.stderr).starts_with(refusal),
            "{}",
            text(&out.stderr)
        );
    }
    assert!(tree(&dir) == before, "a refusal wrote");
    // lint names it too, and so `check` and a run's preflight refuse
    let out = vitrify(&dir, &["lint", "--sdd", "specs/gamma.sdd.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stdout).contains(
            "[L10] CH-001: a change entry outside `## Changelog` is not read: `## CH-001` ends that section → write it as `### CH-001 — …` under `## Changelog`"
        ),
        "{}",
        text(&out.stdout)
    );

    // written where it is read, it applies
    edit(&gamma, "## CH-001 — Activation", "### CH-001 — Activation");
    let out = vitrify(&dir, &["change", "apply", "--sdd", "specs/gamma.sdd.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "gamma: CH-001 0.1.0 → 1.0.0 (major): no unit marked Stale\n\
         gamma: - Staleness applied: —\n"
    );
    // a later entry in the wrong place is refused, not passed over as
    // "already recorded"
    edit(&gamma, "version: 1.0.0", "version: 1.1.0");
    append(&gamma, "\n## CH-002 — More — 1.0.0 → 1.1.0\n");
    let out = vitrify(&dir, &["change", "apply", "--sdd", "specs/gamma.sdd.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr)
            .starts_with("error: gamma: CH-002: a change entry outside `## Changelog` is not read"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn all_applies_each_sdd_and_exits_with_the_worst() {
    let (_tmp, dir) = scratch("seeded/pin-multi/project");
    // beta's entry is refused (stale pin); gamma's activation entry, in a
    // single file with no journal yet, creates its journal
    std::fs::write(
        dir.join("specs/beta/changelog.md"),
        "# Changelog\n\n## CH-001 — Activation — 0.1.0 → 1.0.0\n",
    )
    .unwrap();
    let gamma = dir.join("specs/gamma.sdd.md");
    append(
        &gamma,
        "\n## Changelog\n\n### CH-001 — Activation — 0.1.0 → 1.0.0\n",
    );

    let out = vitrify(&dir, &["change", "apply", "--all", "--dry-run"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        text(&out.stdout),
        "alpha: no change entry — nothing to apply\n\
         gamma: CH-001 0.1.0 → 1.0.0 (major): no unit marked Stale\n\
         gamma: - Staleness applied: —\n"
    );
    assert!(
        !std::fs::read_to_string(&gamma)
            .unwrap()
            .contains("## Journal")
    );

    let out = vitrify(&dir, &["change", "apply", "--all", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["sdd"].as_str().unwrap()).collect();
    assert_eq!(names, ["alpha", "gamma"]);
    assert_eq!(rows[1]["action"], "applied");
    assert_eq!(rows[1]["root"], "specs/gamma.sdd.md");
    let err = text(&out.stderr);
    assert!(
        err.contains("error: beta: C-002's pin is stale → vitrify pin sync --sdd specs/beta first"),
        "{err}"
    );
    assert!(
        err.contains("next: copy the Staleness applied line into CH-001 (specs/gamma.sdd.md)"),
        "{err}"
    );
    let g = std::fs::read_to_string(&gamma).unwrap();
    assert!(
        g.contains("## Journal") && g.contains("\"event\":\"change\",\"entry\":\"CH-001\""),
        "{g}"
    );

    // once beta's pin is synced, every SDD is applied or has nothing to do
    assert!(
        vitrify(&dir, &["pin", "sync", "--sdd", "specs/beta"])
            .status
            .success()
    );
    let out = vitrify(&dir, &["change", "apply", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "alpha: no change entry — nothing to apply\n\
         beta: CH-001 0.1.0 → 1.0.0 (major): no unit marked Stale\n\
         beta: - Staleness applied: —\n\
         gamma: CH-001 already recorded — nothing to apply\n"
    );
    assert!(dir.join("specs/beta/journal.ndjson").is_file());
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
