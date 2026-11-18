//! Golden hash equality with the regression corpus, masking
//! edge cases, stamp invariance, verify-mismatch naming.

use std::path::PathBuf;

use vitrify_core::model::{self, Instance};
use vitrify_core::trace;
use vitrify_core::yamlish;

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
fn golden_hash_maps_byte_identical() {
    let inst = load_calc();
    for uid in ["U-001", "U-002", "U-003", "U-004", "U-005"] {
        let g: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                repo_root().join(format!("fixtures/golden/calc/trace-{uid}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let grecorded = g["recorded"].as_object().unwrap();
        let unit = inst.unit(uid).unwrap();
        let (computed, problems) = trace::compute(&inst, unit).unwrap();
        assert!(problems.is_empty(), "{uid}: {problems:?}");
        assert_eq!(grecorded.len(), computed.len(), "{uid}: key count");
        for (key, hash) in &computed {
            assert_eq!(
                grecorded[key].as_str().unwrap(),
                hash,
                "{uid}: key {key} differs from golden"
            );
        }
        // fixed key order: unit, consumes in listed order, global
        let mut expected_order = vec!["unit".to_string()];
        expected_order.extend(unit.meta.get_list("consumes"));
        expected_order.push("global".to_string());
        let actual_order: Vec<String> = computed.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(expected_order, actual_order, "{uid}: key order");
    }
}

#[test]
fn recorded_traces_in_example_verify_clean() {
    // The committed calc units carry the traces recorded at authoring time;
    // they must verify clean under the vitrify recipe.
    let inst = load_calc();
    let mut checked = 0;
    for u in &inst.units {
        // Ready units carry `trace: —` (no recorded trace); only stamped
        // units verify.
        let Some(recorded) = u.meta.get_map("trace").cloned() else {
            continue;
        };
        let (current, _) = trace::compute(&inst, u).unwrap();
        let mismatches = trace::verify(&recorded, &current);
        assert!(mismatches.is_empty(), "{}: {mismatches:?}", u.unit);
        checked += 1;
    }
    assert!(checked >= 1, "no recorded traces found in the example");
}

#[test]
fn masking_walk_edges() {
    // volatile key at start, middle, end; keys that merely prefix volatile
    // names are kept; trace continuation lines dropped until the next key.
    let block = "\
status: Ready
unit: U-001
claimed_by: w1
title: t
statusx: kept
trace:
  unit:   aaa

  global: bbb
owns:
  - a.rs
verified: stamp
";
    let masked = trace::mask_meta_block(block);
    assert_eq!(
        masked,
        "unit: U-001\ntitle: t\nstatusx: kept\nowns:\n  - a.rs\n"
    );
    // empty result has no trailing newline
    assert_eq!(trace::mask_meta_block("status: Ready\n"), "");
    // non-key lines reset trace skipping
    let block2 = "trace:\n  unit: aaa\nplain text line\n  indented after plain\n";
    assert_eq!(
        trace::mask_meta_block(block2),
        "plain text line\n  indented after plain\n"
    );
}

#[test]
fn hash12_is_sha256_prefix() {
    // sha256("") = e3b0c44298fc...
    assert_eq!(trace::hash12(""), "e3b0c44298fc");
    assert_eq!(trace::hash12("abc").len(), 12);
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
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

#[test]
fn stamp_invariance() {
    // Recomputing after a stamp edit yields identical hashes: stamps touch
    // only masked fields.
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&example_dir(), tmp.path());
    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let unit = inst.unit("U-002").unwrap().clone();
    let (before, _) = trace::compute(&inst, &unit).unwrap();

    let stamp = model::render_stamp("2026-10-30", "1.0.0", "abc1234");
    model::rewrite_unit_meta(&inst.handle(), &unit, |inner| {
        let b = yamlish::set_scalar_in_place(inner, "status", "Built");
        let b = yamlish::set_scalar_in_place(&b, "built", &stamp);
        yamlish::set_map_in_place(&b, "trace", &before)
    })
    .unwrap();

    let inst2 = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let unit2 = inst2.unit("U-002").unwrap();
    assert_eq!(unit2.meta.get_str("status"), Some("Built"));
    let (after, _) = trace::compute(&inst2, unit2).unwrap();
    assert_eq!(before, after, "stamp edits changed the trace");
}

#[test]
fn verify_names_every_mismatch_and_compares_prefixes() {
    let recorded = vec![
        ("unit".to_string(), "aaaaaaaaaaaa".to_string()),
        ("C-001".to_string(), "bbbbbbbbbbbb".to_string()),
        ("gone".to_string(), "dddddddddddd".to_string()),
    ];
    let current = vec![
        ("unit".to_string(), "aaaaaaaaaaaaffff".to_string()), // longer, same prefix
        ("C-001".to_string(), "XXXXbbbbbbbb".to_string()),    // differs
        ("new".to_string(), "eeeeeeeeeeee".to_string()),      // missing in recorded
    ];
    let mismatches = trace::verify(&recorded, &current);
    let keys: Vec<&str> = mismatches.iter().map(|m| m.key.as_str()).collect();
    // sorted union; `unit` matches on the shorter prefix
    assert_eq!(keys, ["C-001", "gone", "new"]);

    // truncated-prefix comparison: a shorter recorded hash still matches
    let recorded_short = vec![("unit".to_string(), "aaaaaa".to_string())];
    let current_full = vec![("unit".to_string(), "aaaaaabbbbbb".to_string())];
    assert!(trace::verify(&recorded_short, &current_full).is_empty());
}

#[test]
fn single_shape_masked_region() {
    let tmp = tempfile::tempdir().unwrap();
    let specs = tmp.path().join("specs");
    std::fs::create_dir_all(&specs).unwrap();
    let doc = "---\nsdd: mini\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\n---\n\n\
## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n\
## Work Units\n\n### U-001 — only\n\n```yaml\nunit: U-001\nstatus: Ready\nclaim_seq: 0\nconsumes: []\n```\n\nBody.\n";
    std::fs::write(specs.join("mini.sdd.md"), doc).unwrap();
    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let unit = inst.unit("U-001").unwrap();
    let masked = trace::mask_unit_text(&inst, unit).unwrap();
    assert!(masked.contains("unit: U-001\n"));
    assert!(!masked.contains("status: Ready"));
    assert!(!masked.contains("claim_seq"));
    assert!(masked.contains("Body."));
    assert!(masked.starts_with("### U-001"));
}
