//! The yamlish grammar over the example corpus,
//! front-matter splitting, in-place writes, newline torture.

use std::path::PathBuf;

use vitrify_core::newline::split_universal;
use vitrify_core::yamlish::{
    Value, parse, set_map_in_place, set_scalar_in_place, split_front_matter,
};

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/example")
}

#[test]
fn parses_every_front_matter_block_in_calc() {
    let specs = example_dir().join("specs/calc");
    let mut files = vec![specs.join("sdd.md")];
    let mut units: Vec<_> = std::fs::read_dir(specs.join("units"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    units.sort();
    files.extend(units);
    let mut rendered = String::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let (inner, _) = split_front_matter(&text).expect("front matter");
        let parsed = parse(inner);
        rendered.push_str(&format!(
            "== {}\n{:#?}\n",
            f.file_name().unwrap().to_str().unwrap(),
            parsed
        ));
    }
    insta::assert_snapshot!(rendered);
}

#[test]
fn scalar_inline_list_block_list_nested_map_comments() {
    let block = "\
unit: U-042   # trailing comment
title: 'quoted title'
status: Ready
depends_on: [U-001, U-002]
owns:
  - src/a.rs    # per-item comment
  - src/b.rs
consumes: []
# full-line comment
trace:
  unit:   abc123
  global: def456
empty:
";
    let parsed = parse(block);
    let get = |k: &str| parsed.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    assert_eq!(get("unit"), Some(&Value::Str("U-042".into())));
    assert_eq!(get("title"), Some(&Value::Str("quoted title".into())));
    assert_eq!(
        get("depends_on"),
        Some(&Value::List(vec!["U-001".into(), "U-002".into()]))
    );
    assert_eq!(
        get("owns"),
        Some(&Value::List(vec!["src/a.rs".into(), "src/b.rs".into()]))
    );
    assert_eq!(get("consumes"), Some(&Value::List(vec![])));
    assert_eq!(
        get("trace"),
        Some(&Value::Map(vec![
            ("unit".into(), "abc123".into()),
            ("global".into(), "def456".into())
        ]))
    );
    assert_eq!(get("empty"), Some(&Value::Str("".into())));
    // key order preserved
    let keys: Vec<&str> = parsed.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "unit",
            "title",
            "status",
            "depends_on",
            "owns",
            "consumes",
            "trace",
            "empty"
        ]
    );
}

#[test]
fn front_matter_split_fencing() {
    assert_eq!(
        split_front_matter("---\na: b\n---\nbody\n"),
        Some(("a: b\n", "body\n"))
    );
    assert_eq!(
        split_front_matter("---\r\na: b\r\n---\r\nbody"),
        Some(("a: b\r\n", "body"))
    );
    // no front matter when the doc does not begin with a fence
    assert_eq!(split_front_matter("a: b\n---\n"), None);
    // closing fence must be a line of its own followed by a newline
    assert_eq!(split_front_matter("---\na: b\n---"), None);
    // degenerate: "---\n---\n" has no inner block ending with a newline
    assert_eq!(split_front_matter("---\n---\n"), None);
}

#[test]
fn set_scalar_preserves_other_lines_and_comments() {
    let block = "unit: U-001\nstatus: Ready  # keep me\nclaim_seq: 0\n";
    let out = set_scalar_in_place(block, "status", "Claimed");
    assert_eq!(
        out,
        "unit: U-001\nstatus: Claimed  # keep me\nclaim_seq: 0\n"
    );
    // absent key appends
    let out2 = set_scalar_in_place(block, "claimed_by", "w1");
    assert_eq!(
        out2,
        "unit: U-001\nstatus: Ready  # keep me\nclaim_seq: 0\nclaimed_by: w1\n"
    );
    // untouched lines byte-equal after a round of edits
    let lines_before = split_universal(block);
    let out3 = set_scalar_in_place(block, "claim_seq", "1");
    let lines_after = split_universal(&out3);
    assert_eq!(lines_before[0], lines_after[0]);
    assert_eq!(lines_before[1], lines_after[1]);
}

#[test]
fn set_map_replaces_trace_block_in_place() {
    let block = "unit: U-001\ntrace:\n  unit:   old1\n  global: old2\nowns: []\n";
    let pairs = vec![
        ("unit".to_string(), "aaaaaaaaaaaa".to_string()),
        ("C-001".to_string(), "bbbbbbbbbbbb".to_string()),
        ("global".to_string(), "cccccccccccc".to_string()),
    ];
    let out = set_map_in_place(block, "trace", &pairs);
    assert_eq!(
        out,
        "unit: U-001\ntrace:\n  unit:   aaaaaaaaaaaa\n  C-001:  bbbbbbbbbbbb\n  global: cccccccccccc\nowns: []\n"
    );
    // absent block appends at end
    let out2 = set_map_in_place("unit: U-001\n", "trace", &pairs);
    assert!(out2.starts_with("unit: U-001\ntrace:\n"));
}

#[test]
fn cr_and_crlf_terminate_lines() {
    let block = "unit: U-009\r\nstatus: Ready\rtitle: x\n";
    let parsed = parse(block);
    let keys: Vec<&str> = parsed.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["unit", "status", "title"]);
    assert_eq!(split_universal("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
}
