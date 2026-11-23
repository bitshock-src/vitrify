//! Each seeded lint fixture reports exactly its own
//! finding(s); pristine calc reports only L7 findings.

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn vitrify(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(dir).env("VITRIFY_HOME", home());
    c
}

fn finding_set(json: &str) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let mut set: Vec<(String, String)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["check"].as_str().unwrap().to_string(),
                x["subject"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    set.sort();
    set.dedup();
    set
}

#[test]
fn seeded_fixtures_report_exactly_their_own_finding() {
    for n in 1..=8 {
        let fixture = format!("lint-L{n}");
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(
            &repo_root().join("fixtures/seeded").join(&fixture),
            tmp.path(),
        );
        let root = tmp.path().canonicalize().unwrap();
        let out = vitrify(&root).args(["lint", "--json"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{fixture}");
        let got = finding_set(&String::from_utf8(out.stdout).unwrap());
        let expected = finding_set(&std::fs::read_to_string(root.join("expected.json")).unwrap());
        assert_eq!(got, expected, "{fixture}");
        // every finding belongs to the fixture's own check
        for (check, _) in &got {
            assert_eq!(
                check,
                &format!("L{n}"),
                "{fixture} leaked a foreign finding"
            );
        }
    }
}

/// `(sdd, check, subject)` of every finding: from `lint --all --json`'s
/// reports, or from an `expected.json` that names each finding's SDD.
fn sdd_finding_set(v: &serde_json::Value) -> Vec<(String, String, String)> {
    let s = |x: &serde_json::Value| x.as_str().unwrap().to_string();
    let mut set: Vec<(String, String, String)> = match v {
        serde_json::Value::Array(reports) => reports
            .iter()
            .flat_map(|r| {
                r["findings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| (s(&r["sdd"]), s(&x["check"]), s(&x["subject"])))
                    .collect::<Vec<_>>()
            })
            .collect(),
        _ => v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (s(&x["sdd"]), s(&x["check"]), s(&x["subject"])))
            .collect(),
    };
    set.sort();
    set
}

#[test]
fn seeded_fixtures_across_sdds_report_exactly_their_own_findings() {
    // pins in depends_on misused (L1), and what the graph across SDDs
    // refuses: an authority without producers, a cycle through pins and a
    // unit behind it (L12); and a cycle inside one SDD through a pin of its
    // own contract (L12)
    for (fixture, check) in [
        ("lint-L1-pins", "L1"),
        ("lint-L12", "L12"),
        ("lint-L12-self-pin", "L12"),
        ("lint-tests", "L2"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(
            &repo_root().join("fixtures/seeded").join(fixture),
            tmp.path(),
        );
        let root = tmp.path().canonicalize().unwrap();
        let out = vitrify(&root)
            .args(["lint", "--all", "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{fixture}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let got = sdd_finding_set(&v);
        let expected = sdd_finding_set(
            &serde_json::from_str(&std::fs::read_to_string(root.join("expected.json")).unwrap())
                .unwrap(),
        );
        assert_eq!(got, expected, "{fixture}");
        for report in v.as_array().unwrap() {
            for x in report["findings"].as_array().unwrap() {
                assert_eq!(x["check"], check, "{fixture} leaked a foreign finding");
                assert!(!x["remediation"].as_str().unwrap().is_empty(), "{x}");
            }
        }
    }
}

#[test]
fn l1_and_l12_name_cause_and_remediation() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/lint-L1-pins"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/app"])
        .assert()
        .code(2)
        .stdout(
            "[L1] U-003: depends_on names pin C-002, which U-003 does not consume → add C-002 to U-003's consumes, or remove it from U-003's depends_on\n\
             [L1] U-004: depends_on names C-001, an internal contract of this SDD — name the unit instead → replace C-001 in U-004's depends_on with U-001\n\
             [L1] U-005: depends_on names C-003, a transcription — it has no authority, so there is nothing to wait for → remove C-003 from U-005's depends_on\n",
        );
    vitrify(&root)
        .args(["lint", "--sdd", "specs/old"])
        .assert()
        .code(2)
        .stdout(
            "[L1] U-001: depends_on names pin C-001, and this SDD declares schema_version 2.0 — a pin in depends_on is schema 2.1 → set schema_version: 2.1 in the SDD's core front matter (a PATCH change entry), or remove C-001 from U-001's depends_on\n",
        );

    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/seeded/lint-L12"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/api"])
        .assert()
        .code(2)
        .stdout(
            "[L12] U-002: depends_on names pin C-002 (core C-001), and the Producer line of core C-001 names no unit → name the units of core that produce C-001 on its Producer line\n\
             [L12] api U-001 → core U-002 → api U-001: depends_on graph across SDDs contains a cycle through pins → remove one of the cycle's depends_on edges — a pin a unit only reads stays in consumes alone\n\
             [L12] U-003: waits behind the cycle api U-001 → core U-002 → api U-001 → break the cycle its own L12 finding names\n",
        );
    // breaking the cycle clears the cycle and the unit behind it
    let unit = root.join("specs/core/units/U-002.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("depends_on: [C-003]", "depends_on: []")).unwrap();
    vitrify(&root)
        .args(["lint", "--all"])
        .assert()
        .code(2)
        .stdout(
            "api: [L12] U-002: depends_on names pin C-002 (core C-001), and the Producer line of core C-001 names no unit → name the units of core that produce C-001 on its Producer line\n\
             core: lint clean (L1–L12)\n",
        );
}

#[test]
fn l12_sees_a_cycle_through_a_pin_of_the_sdds_own_contract() {
    // core C-004 pins core C-002, which U-002 produces; U-002 depends on
    // C-004: U-002 waits on itself, through the pin
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/lint-L12-self-pin"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/core"])
        .assert()
        .code(2)
        .stdout(
            "[L12] core U-002 → core U-002: depends_on graph contains a cycle through a pin of this SDD's own contract → remove one of the cycle's depends_on edges — a pin a unit only reads stays in consumes alone\n",
        );
    // reading the pin without waiting on it clears the cycle
    let unit = root.join("specs/core/units/U-002.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("depends_on: [C-004]", "depends_on: []")).unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/core"])
        .assert()
        .code(0);
}

#[test]
fn l2_follows_gitignore_not_ecosystem_names() {
    // L2's exhaustiveness covers what git would commit: ignored files never
    // need an owner, and no directory name is special
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/seeded/lint-L2"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
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

    let out = vitrify(&root).args(["lint", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        finding_set(&String::from_utf8(out.stdout).unwrap()),
        vec![
            ("L2".to_string(), "U-001/U-002".to_string()),
            (
                "L2".to_string(),
                "src/node_modules/pkg/index.js".to_string()
            ),
        ]
    );
}

#[test]
fn lint_fixture_base_is_clean() {
    // the shared base (fixture minus its mutation) must be lint-clean;
    // lint-L1 restored to a valid dep demonstrates it
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/seeded/lint-L1"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    let upath = root.join("specs/x/units/U-002-b.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    std::fs::write(
        &upath,
        text.replace("depends_on: [U-003]", "depends_on: [U-001]"),
    )
    .unwrap();
    vitrify(&root)
        .args(["lint"])
        .assert()
        .success()
        .stdout("lint clean (L1–L12)\n");
}

#[test]
fn pristine_calc_reports_only_l7() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    let out = vitrify(&root).args(["lint", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let got = finding_set(&String::from_utf8(out.stdout).unwrap());
    assert!(!got.is_empty(), "the built example must trip L7");
    for (check, subject) in &got {
        assert_eq!(check, "L7", "unexpected {check} finding on {subject}");
    }
    // the built unit's state is what trips it
    assert!(got.iter().any(|(_, s)| s == "U-001"));
}

/// A git repository holding the calc example, with one lease-free claim's
/// worth of state committed.
fn calc_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "t@test"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "seed"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
    }
    (tmp, root)
}

fn readset_total(root: &Path, unit: &str) -> i64 {
    // `--budget 0` fails naming the total: the number L5 judges
    let out = vitrify(root)
        .args(["readset", "--unit", unit, "--budget", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    let total = stderr
        .split("totals ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_else(|| panic!("no total in: {stderr}"));
    total.parse().unwrap()
}

#[test]
fn l5_measures_what_the_author_wrote_not_what_vitrify_stamps() {
    // Claiming, stamping Built and stamping Verified all write into the
    // unit's front matter — holder, lease, stamps, a trace line per input.
    // None of it may move the reading-set total, or a unit authored at the
    // budget would fail L5 the moment vitrify had built it, and the next
    // `vitrify run` would refuse to start.
    let (_tmp, root) = calc_repo();
    let authored = readset_total(&root, "U-003");
    let file_size = |root: &Path| {
        std::fs::metadata(root.join("specs/calc/units/U-003-tokenizer.md"))
            .unwrap()
            .len()
    };
    let before = file_size(&root);

    vitrify(&root)
        .args(["claim", "--unit", "U-002", "--holder", "w0"])
        .assert()
        .success();
    for stamp in ["built", "verified"] {
        vitrify(&root)
            .args(["trace", "record", "--unit", "U-003", "--stamp", stamp])
            .assert()
            .success();
    }
    assert!(file_size(&root) > before, "the stamps did grow the file");
    assert_eq!(readset_total(&root, "U-003"), authored);

    // a budget of exactly the authored size still passes L5 after the build
    let out = vitrify(&root)
        .args(["lint", "--json"])
        .env("VITRIFY_LINT_READSET_BUDGET_BYTES", authored.to_string())
        .output()
        .unwrap();
    let got = finding_set(&String::from_utf8(out.stdout).unwrap());
    assert!(
        !got.iter()
            .any(|(check, subject)| check == "L5" && subject == "U-003"),
        "{got:?}"
    );
    // and one byte less still fails: the check is as strict as it was
    let out = vitrify(&root)
        .args(["lint", "--json"])
        .env(
            "VITRIFY_LINT_READSET_BUDGET_BYTES",
            (authored - 1).to_string(),
        )
        .output()
        .unwrap();
    let got = finding_set(&String::from_utf8(out.stdout).unwrap());
    assert!(
        got.iter()
            .any(|(check, subject)| check == "L5" && subject == "U-003"),
        "{got:?}"
    );
}

// ---------------------------------------------- the checks beyond the SDD

/// A scratch copy of the calc example.
fn calc() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{rel} has no `{from}`");
    std::fs::write(path, text.replace(from, to)).unwrap();
}

/// `lint --built --json`: the findings of every check but L7, with causes.
fn built_findings(root: &Path) -> Vec<(String, String, String)> {
    let out = vitrify(root)
        .args(["lint", "--built", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        out.status.code(),
        Some(if v["clean"] == true { 0 } else { 2 })
    );
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["check"].as_str().unwrap().to_string(),
                x["subject"].as_str().unwrap().to_string(),
                x["cause"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn built_leaves_out_l7_and_nothing_else() {
    // the calc example ships with its baseline unit Verified: every check
    // but L7 passes on it, which is what `run` preflights with
    let (_tmp, root) = calc();
    vitrify(&root)
        .args(["lint", "--built"])
        .assert()
        .success()
        .stdout("lint clean (L1–L12)\n");
    // a real finding still shows
    edit(
        &root,
        "specs/calc/units/U-003-tokenizer.md",
        "depends_on: [U-001, U-002]",
        "depends_on: [U-001, U-009]",
    );
    let got = built_findings(&root);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("L1", "U-003"));
}

#[test]
fn l4_holds_a_consumers_line_to_the_units_that_consume() {
    let (_tmp, root) = calc();
    // calc's producers consume their own contracts (U-002 lists C-001) and
    // are not named as consumers: that is in order
    assert!(built_findings(&root).is_empty());

    // a unit named that does not consume the contract; one that does, left out
    edit(
        &root,
        "specs/calc/contracts/C-003-evaluation.md",
        "- Consumers: U-001, U-005",
        "- Consumers: U-001, U-003",
    );
    let got = built_findings(&root);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(
        got[0],
        (
            "L4".to_string(),
            "C-003".to_string(),
            "its Consumers line names U-003, which does not list it in `consumes` and omits U-005, which consumes it"
                .to_string()
        )
    );
    // no consumers named at all
    edit(
        &root,
        "specs/calc/contracts/C-003-evaluation.md",
        "- Consumers: U-001, U-003",
        "- Consumers: —",
    );
    let got = built_findings(&root);
    assert_eq!(
        got[0].2,
        "its Consumers line omits U-001 and U-005, which consume it"
    );
}

#[test]
fn l8_holds_a_marker_to_the_decision_log() {
    let (_tmp, root) = calc();
    // a Draft unit may carry a marker — for a question that has an entry
    edit(
        &root,
        "specs/calc/units/U-005-cli.md",
        "status: Ready",
        "status: Draft",
    );
    edit(
        &root,
        "specs/calc/units/U-005-cli.md",
        "## Implementation\n",
        "## Implementation\n\nOPEN(Q-001): the usage line's wording. OPEN(Q-002) too.\n",
    );
    let got = built_findings(&root);
    let causes: Vec<&str> = got.iter().map(|f| f.2.as_str()).collect();
    assert_eq!(
        causes,
        [
            "OPEN(Q-001) names a question with no entry in the Decision Log",
            "OPEN(Q-002) names a question with no entry in the Decision Log",
        ]
    );
    assert!(got.iter().all(|f| f.0 == "L8" && f.1 == "U-005"));

    edit(
        &root,
        "specs/calc/decisions.md",
        "None.\n",
        "- **Q-001** (2026-10-29, open) — the usage line's wording. Blocked: U-005.\n- **Q-002** (2026-10-29, resolved 2026-10-30) — settled.\n",
    );
    assert!(built_findings(&root).is_empty());
}

#[test]
fn l10_holds_version_and_updated_to_the_last_change_entry() {
    let (_tmp, root) = calc();
    // a version bump without its change entry
    edit(
        &root,
        "specs/calc/sdd.md",
        "version: 1.0.0",
        "version: 1.1.0",
    );
    let got = built_findings(&root);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("L10", "version"));
    assert!(
        got[0]
            .2
            .starts_with("front matter says 1.1.0, the last change entry CH-001 goes to 1.0.0"),
        "{}",
        got[0].2
    );

    // the entry arrives — dated later than `updated` says
    let log = root.join("specs/calc/changelog.md");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(
        &log,
        format!(
            "{text}\n## CH-002 — A rule added (2026-11-02) — 1.0.0 → 1.1.0\n\n- Modified: U-003\n"
        ),
    )
    .unwrap();
    let got = built_findings(&root);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("L10", "updated"));
    assert!(got[0].2.contains("is dated 2026-11-02"), "{}", got[0].2);
    edit(
        &root,
        "specs/calc/sdd.md",
        "updated: 2026-10-28",
        "updated: 2026-11-02",
    );
    assert!(built_findings(&root).is_empty());

    // entries out of id order
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text.replace("## CH-002", "## CH-000")).unwrap();
    let got = built_findings(&root);
    assert!(
        got.iter()
            .any(|f| f.0 == "L10" && f.1 == "CH-000" && f.2.starts_with("follows CH-001")),
        "{got:?}"
    );
}

#[test]
fn l11_holds_the_tdd_pin() {
    let (_tmp, root) = calc();
    // the TDD moves on to a new revision, still in review: an Active SDD
    // may pin neither an unapproved TDD nor a revision that is not current
    edit(
        &root,
        "tdd/calc.md",
        "| **Revision** | 1.0 |",
        "| **Revision** | 1.1 |",
    );
    edit(
        &root,
        "tdd/calc.md",
        "| **Status** | Approved |",
        "| **Status** | In Review |",
    );
    let got = built_findings(&root);
    let view: Vec<(&str, &str)> = got.iter().map(|f| (f.0.as_str(), f.1.as_str())).collect();
    assert_eq!(view, [("L11", "tdd"), ("L11", "tdd_version")]);
    assert!(got[0].2.contains("Status is `In Review`"), "{}", got[0].2);
    assert!(
        got[1]
            .2
            .starts_with("`1.0 (Approved, 2026-10-28)` does not name the TDD's Revision, 1.1"),
        "{}",
        got[1].2
    );
    // a Draft SDD is still being derived: only the path is held
    edit(
        &root,
        "specs/calc/sdd.md",
        "status: Active",
        "status: Draft",
    );
    assert!(built_findings(&root).is_empty());

    // the path names no file
    edit(
        &root,
        "specs/calc/sdd.md",
        "tdd: ../../tdd/calc.md",
        "tdd: ../../tdd/gone.md",
    );
    let got = built_findings(&root);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("L11", "tdd"));
    assert!(
        got[0].2.contains("`../../tdd/gone.md` names no file"),
        "{}",
        got[0].2
    );
    // a path from the repository root is as good as one from the SDD
    edit(
        &root,
        "specs/calc/sdd.md",
        "tdd: ../../tdd/gone.md",
        "tdd: tdd/calc.md",
    );
    assert!(built_findings(&root).is_empty());
}

#[test]
fn all_lints_every_sdd_of_the_repository() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/run-multi/project"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--all"])
        .assert()
        .success()
        .stdout("alpha: lint clean (L1–L12)\nbeta: lint clean (L1–L12)\n");

    // one SDD's finding does not hide the other's result, and names its SDD
    edit(
        &root,
        "specs/beta/units/U-002-two.md",
        "depends_on: [U-001]",
        "depends_on: [U-007]",
    );
    let out = vitrify(&root).args(["lint", "--all"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "alpha: lint clean (L1–L12)\nbeta: [L1] U-002: depends_on references nonexistent unit U-007 → remove U-007 from U-002's depends_on, or add unit U-007\n"
    );
    let out = vitrify(&root)
        .args(["lint", "--all", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["sdd"], "alpha");
    assert_eq!(v[0]["root"], "specs/alpha");
    assert_eq!(v[0]["clean"], true);
    assert_eq!(v[1]["sdd"], "beta");
    assert_eq!(v[1]["findings"][0]["check"], "L1");
    assert_eq!(
        v[1]["findings"][0]["remediation"],
        "remove U-007 from U-002's depends_on, or add unit U-007"
    );

    // --all and --sdd do not combine; one SDD is still named by --sdd
    vitrify(&root)
        .args(["lint", "--all", "--sdd", "specs/alpha"])
        .assert()
        .code(2);
    vitrify(&root)
        .args(["lint", "--sdd", "specs/alpha"])
        .assert()
        .success()
        .stdout("lint clean (L1–L12)\n");
}

#[test]
fn every_finding_carries_its_remediation() {
    for n in 1..=8 {
        let fixture = format!("lint-L{n}");
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(
            &repo_root().join("fixtures/seeded").join(&fixture),
            tmp.path(),
        );
        let root = tmp.path().canonicalize().unwrap();
        let out = vitrify(&root).args(["lint"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{fixture}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        for line in stdout.lines() {
            let (_, remediation) = line
                .split_once(" → ")
                .unwrap_or_else(|| panic!("{fixture}: no remediation in `{line}`"));
            assert!(!remediation.trim().is_empty(), "{fixture}: `{line}`");
        }
        let out = vitrify(&root).args(["lint", "--json"]).output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        for x in v["findings"].as_array().unwrap() {
            assert!(
                !x["remediation"].as_str().unwrap().is_empty(),
                "{fixture}: {x}"
            );
        }
    }
}

#[test]
fn remediations_name_the_command_or_the_edit() {
    // a built SDD linted without --built: L7's remediation is --built
    let (_tmp, root) = calc();
    let out = vitrify(&root)
        .args(["lint", "--sdd", "specs/calc"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "[L7] U-001: status Verified is beyond Ready before the build → the SDD is being built: vitrify lint --built --sdd specs/calc\n\
         [L7] U-001: trace recorded before the build → the SDD is being built: vitrify lint --built --sdd specs/calc\n"
    );

    // L10: the version and date to write are the last entry's
    edit(
        &root,
        "specs/calc/sdd.md",
        "version: 1.0.0",
        "version: 1.1.0",
    );
    let out = vitrify(&root)
        .args(["lint", "--built", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["findings"][0]["check"], "L10");
    assert_eq!(
        v["findings"][0]["remediation"],
        "set version and updated in the front matter to CH-001's (1.0.0 and 2026-10-28)"
    );

    // L9: a stale pin is synced
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(
        &repo_root().join("fixtures/seeded/pin-multi/project"),
        tmp.path(),
    );
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/beta"])
        .assert()
        .code(2)
        .stdout(
            "[L9] C-002: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97) → vitrify pin sync --sdd specs/beta\n",
        );
}

#[test]
fn new_sdd_and_tdd_name_the_next_step() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("specs");
    std::fs::create_dir_all(&cwd).unwrap();
    set_up_home(&tmp.path().join("home"));
    let new = |args: &[&str]| {
        let mut c = Command::cargo_bin("vitrify").unwrap();
        c.current_dir(&cwd)
            .env("VITRIFY_HOME", tmp.path().join("home"))
            .arg("new")
            .args(args);
        c.output().unwrap()
    };
    let out = new(&["sdd", "calc"]);
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.ends_with(
            "next: fill calc per vitrify schema show, then run vitrify lint --sdd calc\n"
        ),
        "{stderr}"
    );
    let out = new(&["sdd", "--single", "pool"]);
    assert!(String::from_utf8(out.stderr).unwrap().ends_with(
        "next: fill pool.sdd.md per vitrify schema show, then run vitrify lint --sdd pool.sdd.md\n"
    ));
    let out = new(&["tdd", "calc"]);
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .ends_with("next: fill calc.md per vitrify schema show --tdd\n")
    );
    // stdout is the created path alone
    let out = new(&["tdd", "other"]);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        cwd.canonicalize()
            .unwrap()
            .join("other.md")
            .display()
            .to_string()
    );
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

#[test]
fn l2_tests_entries_name_cause_and_remediation() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&repo_root().join("fixtures/seeded/lint-tests"), tmp.path());
    let root = tmp.path().canonicalize().unwrap();
    vitrify(&root)
        .args(["lint", "--sdd", "specs/new"])
        .assert()
        .code(2)
        .stdout(
            "[L2] docs/new/: tests: entry `docs/new/` lies outside scope_roots → narrow `docs/new/` to a path under scope_roots, or remove it from tests:\n\
             [L2] tests/new/: tests: entry `tests/new/` and unmanaged: entry `tests/new/gen/` can match a common path → narrow one of them so no path is both a test and unmanaged\n",
        );
    vitrify(&root)
        .args(["lint", "--sdd", "specs/old"])
        .assert()
        .code(2)
        .stdout(
            "[L2] tests:: §6 lists tests:, and this SDD declares schema_version 2.1 — a tests: list is schema 2.2 → set schema_version: 2.2 in the SDD's core front matter (a PATCH change entry), or remove the tests: list\n",
        );
}
