//! Journal line round-trips, golden replay equality,
//! append semantics, seeded order findings, single-shape fence creation.

use std::path::PathBuf;

use vitrify_core::journal::{self, Event};
use vitrify_core::model::Instance;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn example_dir() -> PathBuf {
    repo_root().join("fixtures/example")
}

#[test]
fn render_line_round_trips_every_vendored_line() {
    let raw = std::fs::read_to_string(example_dir().join("specs/calc/journal.ndjson")).unwrap();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let event = journal::parse_line(line).unwrap();
        assert_eq!(journal::render_line(&event), line);
    }
}

#[test]
fn replay_matches_golden() {
    let inst = Instance::discover(&example_dir(), None, &example_dir()).unwrap();
    let (events, exists) = journal::read(&inst.handle()).unwrap();
    assert!(exists);
    let g: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("fixtures/golden/calc/replay.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(g["events"].as_i64().unwrap() as usize, events.len());
    let proj = journal::replay(&events);
    assert!(proj.order_findings.is_empty());
    let mismatches = journal::verify(&proj, &inst.units);
    assert!(mismatches.is_empty(), "{mismatches:?}");
    assert_eq!(g["consistent"], true);
    let gproj = g["projection"].as_object().unwrap();
    assert_eq!(gproj.len(), proj.status.len());
    for (unit, status) in &proj.status {
        assert_eq!(gproj[unit.as_str()].as_str().unwrap(), status);
    }
}

#[test]
fn append_assigns_seq_and_reads_back() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("specs/x");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sdd.md"),
        "---\nsdd: x\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\n---\n",
    )
    .unwrap();
    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let handle = inst.handle();

    let (events, exists) = journal::read(&handle).unwrap();
    assert!(!exists);
    assert!(events.is_empty());

    let e1 = Event {
        seq: 1,
        at: "2026-10-30T00:00:00Z".into(),
        event: "claim".into(),
        unit: Some("U-001".into()),
        actor: Some("w1".into()),
        claim_seq: Some(1),
        sdd: "0.1.0".into(),
        commit: Some("abc1234".into()),
        ..Default::default()
    };
    journal::append_line(&handle, &journal::render_line(&e1)).unwrap();
    let (events, exists) = journal::read(&handle).unwrap();
    assert!(exists);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 1);

    // next seq = last + 1
    let e2 = Event {
        seq: events.last().map(|e| e.seq).unwrap_or(0) + 1,
        at: "2026-10-30T00:01:00Z".into(),
        event: "built".into(),
        unit: Some("U-001".into()),
        actor: Some("w1".into()),
        claim_seq: Some(1),
        sdd: "0.1.0".into(),
        ..Default::default()
    };
    journal::append_line(&handle, &journal::render_line(&e2)).unwrap();
    let (events, _) = journal::read(&handle).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].seq, 2);

    let proj = journal::replay(&events);
    assert_eq!(proj.status_of("U-001"), Some("Built"));
    assert_eq!(proj.seq_of("U-001"), Some(1));
}

#[test]
fn seeded_order_and_unknown_event_findings() {
    let lines = [
        r#"{"seq":1,"at":"2026-10-30T00:00:00Z","event":"claim","unit":"U-001","claim_seq":1,"sdd":"0.1.0"}"#,
        r#"{"seq":1,"at":"2026-10-30T00:00:01Z","event":"built","unit":"U-001","sdd":"0.1.0"}"#,
        r#"{"seq":3,"at":"2026-10-30T00:00:02Z","event":"frobnicate","unit":"U-001","sdd":"0.1.0"}"#,
    ];
    let events: Vec<Event> = lines
        .iter()
        .map(|l| journal::parse_line(l).unwrap())
        .collect();
    let proj = journal::replay(&events);
    assert_eq!(
        proj.order_findings,
        vec![
            "seq not strictly increasing at seq=1".to_string(),
            "unknown event 'frobnicate' at seq=3".to_string(),
        ]
    );
}

#[test]
fn projection_rules_cover_vocabulary() {
    let mk = |seq: i64, event: &str, unit: Option<&str>| Event {
        seq,
        at: "2026-10-30T00:00:00Z".into(),
        event: event.into(),
        unit: unit.map(str::to_string),
        claim_seq: if event == "claim" { Some(2) } else { None },
        stale: if event == "change" {
            Some(vec!["U-003".into(), "U-004".into()])
        } else {
            None
        },
        sdd: "0.1.0".into(),
        ..Default::default()
    };
    let events = vec![
        mk(1, "claim", Some("U-001")),
        mk(2, "renew", Some("U-001")),
        mk(3, "release", Some("U-001")),
        mk(4, "claim", Some("U-002")),
        mk(5, "built", Some("U-002")),
        mk(6, "verified", Some("U-002")),
        mk(7, "change", None),
        mk(8, "stale", Some("U-005")),
        mk(9, "retired", Some("U-006")),
        mk(10, "audit", None),
    ];
    let proj = journal::replay(&events);
    assert!(proj.order_findings.is_empty());
    assert_eq!(proj.status_of("U-001"), Some("Ready"));
    assert_eq!(proj.status_of("U-002"), Some("Verified"));
    assert_eq!(proj.seq_of("U-002"), Some(2));
    assert_eq!(proj.status_of("U-003"), Some("Stale"));
    assert_eq!(proj.status_of("U-004"), Some("Stale"));
    assert_eq!(proj.status_of("U-005"), Some("Stale"));
    assert_eq!(proj.status_of("U-006"), Some("Retired"));
}

#[test]
fn no_event_units_must_be_draft_or_ready() {
    let inst_units = |status: &str| {
        vec![vitrify_core::model::Unit {
            unit: "U-001".into(),
            file: PathBuf::from("mem"),
            meta: vitrify_core::model::Meta(vitrify_core::yamlish::parse(&format!(
                "unit: U-001\nstatus: {status}\n"
            ))),
        }]
    };
    let empty = journal::replay(&[]);
    assert!(journal::verify(&empty, &inst_units("Ready")).is_empty());
    assert!(journal::verify(&empty, &inst_units("Draft")).is_empty());
    let mm = journal::verify(&empty, &inst_units("Built"));
    assert_eq!(mm.len(), 1);
    assert_eq!(mm[0].projected.repr(), "'Draft|Ready (no events)'");
    assert_eq!(mm[0].current.repr(), "'Built'");
}

#[test]
fn single_shape_fence_created_then_appended() {
    let tmp = tempfile::tempdir().unwrap();
    let specs = tmp.path().join("specs");
    std::fs::create_dir_all(&specs).unwrap();
    let doc = "---\nsdd: mini\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\n---\n\n## Work Units\n\n### U-001 — x\n\n```yaml\nunit: U-001\nstatus: Ready\n```\n";
    let path = specs.join("mini.sdd.md");
    std::fs::write(&path, doc).unwrap();
    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let handle = inst.handle();

    let (_, exists) = journal::read(&handle).unwrap();
    assert!(!exists);

    let line1 = r#"{"seq":1,"at":"2026-10-30T00:00:00Z","event":"claim","unit":"U-001","claim_seq":1,"sdd":"0.1.0"}"#;
    journal::append_line(&handle, line1).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("## Journal"));
    assert!(text.contains("```\n{\"seq\":1"));
    assert!(text.ends_with("```\n"));

    let line2 = r#"{"seq":2,"at":"2026-10-30T00:00:01Z","event":"release","unit":"U-001","claim_seq":1,"sdd":"0.1.0"}"#;
    journal::append_line(&handle, line2).unwrap();
    let (events, exists) = journal::read(&handle).unwrap();
    assert!(exists);
    assert_eq!(events.len(), 2);
    // the original document bytes before the journal section are preserved
    let text2 = std::fs::read_to_string(&path).unwrap();
    assert!(text2.starts_with(doc.trim_end_matches('\n')));
}
