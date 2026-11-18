//! `vitrify questions`: every `Q-###` of the Decision Log, open unless
//! marked resolved, with the units it blocks — for one SDD or all, in both
//! shapes.

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
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

fn vitrify(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("vitrify").unwrap();
    c.current_dir(dir).env("VITRIFY_HOME", home());
    c
}

fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{rel} has no `{from}`");
    std::fs::write(path, text.replace(from, to)).unwrap();
}

const LOG: &str = "- **Q-001** (2026-10-29, open) — TDD §3.2 is silent on a leading `+`;\n  conflicts with C-002. Blocked: U-003.\n- **Q-002** (2026-10-29, resolved 2026-10-30) — settled by TDD 1.1.\n- **Q-003** (2026-10-31) — reported by run-20261031T101500Z-p4242-w2 on U-005: the usage line's wording\n";

#[test]
fn lists_each_question_its_status_and_the_units_it_blocks() {
    let (_tmp, root) = scratch("fixtures/example");
    // calc ships with none
    vitrify(&root)
        .args(["questions"])
        .assert()
        .success()
        .stdout("0 open, 0 resolved\n");

    edit(&root, "specs/calc/decisions.md", "None.\n", LOG);
    // a unit carries the marker of the question it waits on; a resolved
    // question's leftover marker still names its unit
    edit(
        &root,
        "specs/calc/units/U-003-tokenizer.md",
        "## Implementation\n",
        "## Implementation\n\nOPEN(Q-001): whether `+7` is one token.\n",
    );
    edit(
        &root,
        "specs/calc/units/U-004-evaluator.md",
        "## Implementation\n",
        "## Implementation\n\nOPEN(Q-001) and, once, OPEN(Q-002).\n",
    );
    vitrify(&root)
        .args(["questions"])
        .assert()
        .success()
        .stdout(
            "Q-001  open      blocks U-003, U-004  TDD §3.2 is silent on a leading `+`; conflicts with C-002. Blocked: U-003.\n\
             Q-002  resolved  blocks U-004         settled by TDD 1.1.\n\
             Q-003  open      blocks U-005         reported by run-20261031T101500Z-p4242-w2 on U-005: the usage line's wording\n\
             2 open, 1 resolved\n",
        );
    // --open leaves the resolved out of the list, not out of the count
    vitrify(&root)
        .args(["questions", "--open"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Q-002").not())
        .stdout(predicate::str::ends_with("2 open, 1 resolved\n"));

    let out = vitrify(&root)
        .args(["questions", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["open"], 2);
    assert_eq!(v["resolved"], 1);
    assert_eq!(
        v["questions"][0],
        serde_json::json!({
            "sdd": "calc",
            "q": "Q-001",
            "status": "open",
            "blocks": ["U-003", "U-004"],
            "noted": "2026-10-29, open",
            "text": "TDD §3.2 is silent on a leading `+`; conflicts with C-002. Blocked: U-003."
        })
    );
    // an entry that says nothing about its status is open — what a run
    // landed before it said `open` is not read as resolved
    assert_eq!(v["questions"][2]["status"], "open");
    assert_eq!(v["questions"][2]["noted"], "2026-10-31");
    assert_eq!(v["questions"][2]["blocks"], serde_json::json!(["U-005"]));
    // resolved, it blocks nothing by having been reported
    edit(
        &root,
        "specs/calc/decisions.md",
        "(2026-10-31)",
        "(2026-10-31, resolved 2026-11-01)",
    );
    let out = vitrify(&root)
        .args(["questions", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["questions"][2]["status"], "resolved");
    assert_eq!(v["questions"][2]["blocks"], serde_json::json!([]));
}

#[test]
fn questions_of_every_sdd_and_of_a_single_file() {
    let (_tmp, root) = scratch("fixtures/seeded/pin-multi/project");
    std::fs::write(
        root.join("specs/beta/decisions.md"),
        "# Decisions & Open Questions\n\n- **D-001** (2026-11-01, SDD 1.0.0) — a decision.\n- **Q-001** (2026-11-01, open) — which clock? Blocked: U-002.\n",
    )
    .unwrap();
    // a single file keeps its log in a section, and its units in regions
    let gamma = root.join("specs/gamma.sdd.md");
    let text = std::fs::read_to_string(&gamma).unwrap();
    std::fs::write(
        &gamma,
        format!(
            "{}\n## Decisions & Open Questions\n\n- **Q-004** (2026-11-02, resolved 2026-11-03) — closed.\n- **Q-005** (2026-11-02, open) — the greeting's case.\n",
            text.replace(
                "#### Requirements\n",
                "#### Requirements\n\nOPEN(Q-005): capitalized or not.\n"
            )
        ),
    )
    .unwrap();

    vitrify(&root)
        .args(["questions", "--all"])
        .assert()
        .success()
        .stdout(
            "beta Q-001   open      blocks —      which clock? Blocked: U-002.\n\
             gamma Q-004  resolved  blocks —      closed.\n\
             gamma Q-005  open      blocks U-001  the greeting's case.\n\
             2 open, 1 resolved\n",
        );
    vitrify(&root)
        .args(["questions", "--sdd", "specs/gamma.sdd.md", "--open"])
        .assert()
        .success()
        .stdout("Q-005  open      blocks U-001  the greeting's case.\n1 open, 1 resolved\n");
    // several SDDs and no flag
    vitrify(&root)
        .args(["questions"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("multiple SDDs found"));
    // the marker in gamma's Ready unit is L8's; its question has its entry
    vitrify(&root)
        .args(["lint", "--sdd", "specs/gamma.sdd.md"])
        .assert()
        .code(2)
        .stdout(
            "[L8] U-001: OPEN(Q-###) marker inside a Ready unit's region → resolve Q-005 in the Decision Log and remove the marker, or set U-001 back to Draft\n",
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
