//! The write-set law as pure classification: only owned paths pass, and
//! never the SDD's own files, whatever `owns` matches; a unit's own spec
//! entry may change only in its handoff fields (status, built, trace) — not
//! its lease fields, not its body, and (single shape) not another unit's
//! region or any text outside the unit's region.

use vitrify_core::model::Shape;
use vitrify_core::trace;
use vitrify_core::writeset::{HANDOFF_FIELDS, Violation, WriteSet};

const UNIT: &str = "---\nunit: U-001\ntitle: one\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: 2026-10-31T12:00:00Z\nclaim_seq: 1\ndepends_on: []\nowns: [src/one.txt]\nbuilt: —\nverified: —\ntrace: —\n---\n\n# U-001\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f src/one.txt` | exit 0 |\n";

fn bundle_ws(owns: &[&str]) -> WriteSet {
    WriteSet {
        unit: "U-001".into(),
        owns: owns.iter().map(|g| g.to_string()).collect(),
        spec_file: "specs/x/units/U-001-one.md".into(),
        spec_paths: vec!["specs/x".into()],
        tests: Vec::new(),
        shape: Shape::Bundle,
    }
}

/// What `trace record --stamp built` does to the unit file.
fn stamped(text: &str) -> String {
    text.replace("status: Claimed", "status: Built")
        .replace("built: —", "built: 2026-10-31 @ SDD 1.0.0, commit abc1234")
        .replace(
            "trace: —",
            "trace:\n  unit:   0123456789ab\n  global: ba9876543210",
        )
}

#[test]
fn only_owned_paths_are_covered() {
    let ws = bundle_ws(&["src/one.txt", "tests/one/"]);
    assert!(ws.covers("src/one.txt"));
    assert!(ws.covers("tests/one/test_parse.sh")); // directory-prefix ownership
    assert!(!ws.covers("src/two.txt"));
    assert!(!ws.covers("tests/two/test_x.sh"));
    assert!(!ws.covers("build/out.log")); // unmanaged or not, nobody's to write
    assert!(!ws.covers("README.md"));
}

#[test]
fn no_glob_covers_the_sdds_own_files() {
    // `*` crosses `/`: a careless glob matches spec files, which stay
    // read-only anyway
    let ws = bundle_ws(&["src/one.txt", "*.md", "specs/"]);
    assert!(ws.covers("docs/guide.md"));
    for p in [
        "specs/x/units/U-002-two.md",
        "specs/x/contracts/C-001-api.md",
        "specs/x/sdd.md",
        "specs/x/ledger.md",
        "specs/x/journal.ndjson",
    ] {
        assert!(!ws.covers(p), "{p}");
    }
    let changed = vec!["specs/x/units/U-002-two.md".to_string()];
    assert_eq!(
        ws.violations(&changed, || (None, None)),
        vec![Violation::SpecFile("specs/x/units/U-002-two.md".into())]
    );
}

#[test]
fn a_bundle_at_the_repo_root_protects_its_own_files_only() {
    let ws = WriteSet {
        unit: "U-001".into(),
        owns: vec!["*.md".into(), "src/".into()],
        spec_file: "units/U-001-one.md".into(),
        spec_paths: ["sdd.md", "ledger.md", "units", "contracts"]
            .iter()
            .map(|p| p.to_string())
            .collect(),
        tests: Vec::new(),
        shape: Shape::Bundle,
    };
    assert!(ws.covers("src/a.md"));
    assert!(ws.covers("NOTES.md"));
    assert!(!ws.covers("sdd.md"));
    assert!(!ws.covers("units/U-002-two.md"));
    assert!(!ws.covers("contracts/C-001-api.md"));
}

#[test]
fn handoff_fields_may_change_nothing_else() {
    let ws = bundle_ws(&["src/one.txt"]);
    assert!(ws.spec_edit_allowed(UNIT, &stamped(UNIT)));
    // lease fields are not handoff fields
    assert!(!ws.spec_edit_allowed(UNIT, &UNIT.replace("claim_seq: 1", "claim_seq: 9")));
    assert!(!ws.spec_edit_allowed(UNIT, &UNIT.replace("verified: —", "verified: x")));
    // nor is any front-matter content or body text
    assert!(!ws.spec_edit_allowed(
        UNIT,
        &UNIT.replace("owns: [src/one.txt]", "owns: [src/one.txt, tests/]")
    ));
    assert!(!ws.spec_edit_allowed(
        UNIT,
        &stamped(UNIT).replace("`test -f src/one.txt`", "`true`")
    ));
}

#[test]
fn violations_name_each_rule() {
    let ws = bundle_ws(&["src/one.txt"]);
    let changed: Vec<String> = [
        "src/one.txt",
        "src/two.txt",
        "build/out.log",
        "specs/x/units/U-001-one.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    // a clean handoff: only the files the unit does not own are violations
    let v = ws.violations(&changed, || (Some(UNIT.into()), Some(stamped(UNIT))));
    assert_eq!(
        v,
        vec![
            Violation::NotOwned("src/two.txt".into()),
            Violation::NotOwned("build/out.log".into()),
        ]
    );
    assert_eq!(v[0].describe("U-001"), "src/two.txt: not owned by U-001");

    // the spec file edited beyond the handoff fields, or deleted
    let edited = stamped(UNIT).replace("`test -f src/one.txt`", "`true`");
    let v = ws.violations(&changed, || (Some(UNIT.into()), Some(edited)));
    assert!(v.contains(&Violation::SpecEdit("specs/x/units/U-001-one.md".into())));
    let v = ws.violations(&changed, || (Some(UNIT.into()), None));
    let spec = v
        .iter()
        .find(|x| matches!(x, Violation::SpecEdit(_)))
        .unwrap();
    assert!(
        spec.describe("U-001")
            .contains("only status, built, trace may change")
    );
}

const SINGLE: &str = "---\nsdd: solo\nschema_version: 2.0\n---\n\n## Ground Rules\n\nG.\n\n## Work Units\n\n### U-001 — one\n\n```yaml\nunit: U-001\nstatus: Claimed\nclaim_seq: 1\nowns: [src/one.txt]\nbuilt: —\ntrace: —\n```\n\n#### Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f src/one.txt` | exit 0 |\n\n### U-002 — two\n\n```yaml\nunit: U-002\nstatus: Ready\nclaim_seq: 0\nowns: [src/two.txt]\nbuilt: —\ntrace: —\n```\n\n#### Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f src/two.txt` | exit 0 |\n";

#[test]
fn single_file_allows_only_the_units_own_handoff_fields() {
    let ws = WriteSet {
        unit: "U-001".into(),
        owns: vec!["src/one.txt".into()],
        spec_file: "specs/solo.sdd.md".into(),
        spec_paths: vec!["specs/solo.sdd.md".into()],
        tests: Vec::new(),
        shape: Shape::Single,
    };
    let own = SINGLE.replacen("status: Claimed", "status: Built", 1);
    assert!(ws.spec_edit_allowed(SINGLE, &own));
    // another unit's region
    let other = own.replace("`test -f src/two.txt`", "`true`");
    assert!(!ws.spec_edit_allowed(SINGLE, &other));
    // another unit's handoff field is still another unit's
    let other_status = SINGLE.replacen("status: Ready", "status: Built", 1);
    assert!(!ws.spec_edit_allowed(SINGLE, &other_status));
    // text outside every unit region
    let outside = own.replace("G.\n", "G, edited.\n");
    assert!(!ws.spec_edit_allowed(SINGLE, &outside));
    // the unit's region removed
    let gone = SINGLE.replace("### U-001 — one", "### U-009 — one");
    assert!(!ws.spec_edit_allowed(SINGLE, &gone));
}

#[test]
fn handoff_mask_is_the_masking_walk_over_fewer_keys() {
    // with every volatile key, mask_unit_fields reproduces the `unit` hash
    // input; HANDOFF_FIELDS is a strict subset of VOLATILE
    for k in HANDOFF_FIELDS {
        assert!(trace::VOLATILE.contains(&k), "{k}");
    }
    let full = trace::mask_unit_fields(Shape::Bundle, UNIT, "U-001", &trace::VOLATILE).unwrap();
    let inner = UNIT.strip_prefix("---\n").unwrap();
    let (block, body) = inner.split_once("---\n").unwrap();
    assert_eq!(
        full,
        format!("---\n{}---\n{}", trace::mask_meta_block(block), body)
    );
}

#[test]
fn a_pre_existing_test_is_named_as_such() {
    let ws = WriteSet {
        unit: "U-001".into(),
        owns: vec!["src/a.rs".into(), "tests/own.rs".into()],
        spec_file: "specs/x/units/U-001-one.md".into(),
        spec_paths: vec!["specs/x".into()],
        tests: vec!["tests/".into()],
        shape: Shape::Bundle,
    };
    let none = || (None, None);
    let v = ws.violations(&["tests/pre.rs".to_string()], none);
    assert_eq!(v, vec![Violation::PreExistingTest("tests/pre.rs".into())]);
    assert_eq!(
        v[0].describe("U-001"),
        "tests/pre.rs: a pre-existing test (§6 tests:) — no unit's work may change it"
    );
    assert!(
        ws.violations(&["tests/own.rs".to_string()], none)
            .is_empty()
    );
    assert_eq!(
        ws.violations(&["src/b.rs".to_string()], none),
        vec![Violation::NotOwned("src/b.rs".into())]
    );
}
