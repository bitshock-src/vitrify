//! Golden ledger byte equality (date-normalized),
//! progress line, status-mix rendering, single-shape section rewrite.

use std::path::PathBuf;

use vitrify_core::ledger;
use vitrify_core::model::{Instance, Meta, Unit};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn example_dir() -> PathBuf {
    repo_root().join("fixtures/example")
}

fn load_calc() -> Instance {
    Instance::discover(&example_dir(), None, &example_dir()).unwrap()
}

#[test]
fn golden_ledger_bytes() {
    let inst = load_calc();
    let golden =
        std::fs::read_to_string(repo_root().join("fixtures/golden/calc/ledger.md")).unwrap();
    // The golden is date-normalized to {DATE}; render with that literal token.
    let rendered = ledger::render(&inst, "{DATE}");
    assert_eq!(golden, rendered);
}

#[test]
fn matches_committed_example_ledger_modulo_date() {
    let inst = load_calc();
    let committed = std::fs::read_to_string(example_dir().join("specs/calc/ledger.md")).unwrap();
    // Extract the committed derived date and render with it.
    let marker = "(derived ";
    let i = committed.find(marker).unwrap() + marker.len();
    let date = &committed[i..i + 10];
    assert_eq!(committed, ledger::render(&inst, date));
}

#[test]
fn golden_progress_line() {
    let inst = load_calc();
    let golden =
        std::fs::read_to_string(repo_root().join("fixtures/golden/calc/progress.txt")).unwrap();
    assert_eq!(golden.trim_end(), ledger::progress_line(&inst.units));
}

fn mk_unit(block: &str) -> Unit {
    let meta = Meta(vitrify_core::yamlish::parse(block));
    Unit {
        unit: meta.get_str("unit").unwrap().to_string(),
        file: PathBuf::from("mem"),
        meta,
    }
}

#[test]
fn status_mix_table_and_notes() {
    let units = vec![
        mk_unit(
            "unit: U-001\nstatus: Verified\nbuilt: 2026-01-01 @ SDD 1.2.0, commit abc\nverified: 2026-01-02 @ SDD 1.2.0, commit def\n",
        ),
        mk_unit(
            "unit: U-002\nstatus: Built\nparallel_group: a\nbuilt: 2026-01-01 @ SDD 1.2.0, commit abc\n",
        ),
        mk_unit(
            "unit: U-003\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: 2026-01-03T00:00:00Z\n",
        ),
        mk_unit("unit: U-004\nstatus: Claimed\nclaimed_by: w2\nclaim_expires: —\n"),
        mk_unit("unit: U-005\nstatus: Ready\n"),
        mk_unit("unit: U-006\nstatus: Stale\n"),
        mk_unit("unit: U-007\nstatus: Draft\n"),
        mk_unit("unit: U-008\nstatus: Retired\n"),
    ];
    let progress = ledger::progress_line(&units);
    assert_eq!(
        progress,
        "Progress: 1/7 Verified · 1 Built · 2 Claimed · 1 Ready · 1 Stale · 1 Draft · 1 Retired"
    );

    let inst = Instance {
        root: PathBuf::from("mem"),
        shape: vitrify_core::model::Shape::Bundle,
        file: PathBuf::from("mem/sdd.md"),
        meta: Meta(vitrify_core::yamlish::parse("sdd: mix\nversion: 2.0.0\n")),
        units,
    };
    let body = ledger::render(&inst, "2026-10-30");
    insta::assert_snapshot!(body);
    assert!(body.contains("lease to 2026-01-03T00:00:00Z"));
    assert!(body.contains("| lease")); // empty-marker expiry → bare "lease"
    assert!(body.contains("awaiting gate"));
    assert!(body.contains("# Ledger — mix @ SDD 2.0.0 (derived 2026-10-30)"));
}

#[test]
fn zero_counts_omitted_after_first_term() {
    let units = vec![mk_unit("unit: U-001\nstatus: Ready\n")];
    assert_eq!(
        ledger::progress_line(&units),
        "Progress: 0/1 Verified · 1 Ready"
    );
}

#[test]
fn single_shape_rewrites_only_the_state_ledger_section() {
    let tmp = tempfile::tempdir().unwrap();
    let specs = tmp.path().join("specs");
    std::fs::create_dir_all(&specs).unwrap();
    let doc = "---\nsdd: mini\nschema_version: 2.0\nversion: 0.3.0\nstatus: Active\n---\n\n# mini\n\n\
## State Ledger\n\nstale content to be replaced\n\n\
## Work Units\n\n### U-001 — x\n\n```yaml\nunit: U-001\nstatus: Ready\n```\n\nBody.\n";
    let path = specs.join("mini.sdd.md");
    std::fs::write(&path, doc).unwrap();
    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();

    let (written, msg) = ledger::regen(&inst, "2026-10-30").unwrap();
    assert_eq!(written, path);
    assert!(msg.contains("regenerated ledger section"));

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("stale content to be replaced"));
    assert!(text.contains("## State Ledger\n\n# Ledger — mini @ SDD 0.3.0 (derived 2026-10-30)"));
    // surrounding bytes preserved
    assert!(text.starts_with(
        "---\nsdd: mini\nschema_version: 2.0\nversion: 0.3.0\nstatus: Active\n---\n\n# mini\n\n"
    ));
    assert!(text.contains("## Work Units\n\n### U-001 — x"));
    assert!(text.ends_with("Body.\n"));

    // a doc without the section gets it appended
    let doc2 = "---\nsdd: mini\nschema_version: 2.0\nversion: 0.3.0\nstatus: Active\n---\n\n## Work Units\n\n### U-001 — x\n\n```yaml\nunit: U-001\nstatus: Ready\n```\n";
    std::fs::write(&path, doc2).unwrap();
    let inst2 = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    ledger::regen(&inst2, "2026-10-30").unwrap();
    let text2 = std::fs::read_to_string(&path).unwrap();
    assert!(text2.contains("\n\n## State Ledger\n\n# Ledger — mini"));
}
