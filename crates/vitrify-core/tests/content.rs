//! Registry completeness, digest correctness, the
//! vitrify-job block, law presence in the briefs.

use std::collections::BTreeSet;
use std::path::PathBuf;

use vitrify_core::content::{self, sha256_hex};

/// The crate's own directory: the embedded sources live in its `assets/`.
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Disk path → asset-relative path, mirroring the registry's mapping.
fn rel_key(disk: &str) -> String {
    if let Some(rest) = disk.strip_prefix("assets/briefs/") {
        return format!("briefs/{rest}");
    }
    if disk == "assets/escalation.md" {
        return "briefs/escalation.md".to_string();
    }
    if disk == "assets/config.toml" {
        return "config.toml".to_string();
    }
    if let Some(rest) = disk.strip_prefix("assets/templates/") {
        return format!("templates/{rest}");
    }
    if let Some(rest) = disk.strip_prefix("assets/schemas/") {
        return format!("schemas/{rest}");
    }
    panic!("unmapped asset path: {disk}");
}

fn walk_assets() -> Vec<String> {
    let root = crate_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join("assets")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(&root).unwrap().to_str().unwrap().to_string();
                out.push(rel);
            }
        }
    }
    out
}

#[test]
fn registry_is_complete_and_digests_correct() {
    let disk: BTreeSet<String> = walk_assets().iter().map(|d| rel_key(d)).collect();
    // the default configuration is embedded apart from the registry: init
    // writes it once, and nothing materializes or refreshes it
    let registry: BTreeSet<String> = content::assets()
        .iter()
        .chain(std::iter::once(&content::CONFIG))
        .map(|a| a.rel_path.to_string())
        .collect();
    assert_eq!(
        disk, registry,
        "registry does not enumerate assets/ exactly"
    );
    assert!(content::get("config.toml").is_none());

    for asset in content::assets()
        .iter()
        .chain(std::iter::once(&content::CONFIG))
    {
        assert_eq!(
            sha256_hex(asset.body),
            asset.sha256,
            "digest literal wrong for {}",
            asset.rel_path
        );
    }
    // lookup works
    assert!(content::get("briefs/build-unit.md").is_some());
    assert!(content::get("nonexistent").is_none());
}

#[test]
fn build_unit_brief_opens_with_the_vitrify_job_block() {
    let brief = content::get("briefs/build-unit.md").unwrap().body;
    assert!(
        brief.starts_with("```vitrify-job\n"),
        "job block must open the brief"
    );
    let block: Vec<&str> = brief.lines().skip(1).take_while(|l| *l != "```").collect();
    let keys: Vec<&str> = block.iter().map(|l| l.split(':').next().unwrap()).collect();
    assert_eq!(keys, ["unit", "holder", "branch", "sdd", "lease_minutes"]);
    // placeholder form ships in the asset
    assert!(brief.contains("unit: {unit}"));
    assert!(brief.contains("holder: {holder}"));
}

#[test]
fn briefs_preserve_the_laws() {
    let build_unit = content::get("briefs/build-unit.md").unwrap().body;
    // the four law phrases, verbatim in force
    assert!(build_unit.contains("One unit per session"));
    assert!(build_unit.contains("write only the unit's `owns` paths"));
    assert!(
        build_unit.contains("stop at `Q-###` rather\nthan guess")
            || build_unit.contains("stop at `Q-###` rather than guess")
    );
    assert!(build_unit.contains("Read only the unit's reading set"));
    // the handoff law: workers never commit; a green build is never reverted
    assert!(build_unit.contains("You never run `git commit`"));
    assert!(build_unit.contains("Never revert a green build"));
    let integrate_brief = content::get("briefs/integrate.md").unwrap().body;
    assert!(integrate_brief.contains("implementers never run `git commit`"));
    // the report schema, verbatim
    assert!(build_unit.contains(
        r#"{"unit":"U-###","outcome":"built|blocked|released","branch":"sdd/<sdd>/U-###/c<seq>","claim_seq":N,"q":"Q-### or null","notes":"<one line>"}"#
    ));
    // every verb the implementer runs names the SDD: a repository may hold
    // several, each with the same unit ids
    for line in build_unit.lines().filter(|l| l.starts_with("vitrify ")) {
        assert!(line.contains("--sdd {sdd}"), "no --sdd on: {line}");
    }
    // the worker is told where the session note goes, and where to work
    assert!(build_unit.contains("{session}"));
    assert!(build_unit.contains("git worktree add {worktree} -b {branch}"));
    // working copies are never placed under the repository's scratch dir
    for name in [
        "briefs/build-unit.md",
        "briefs/integrate.md",
        "briefs/kickoff.md",
    ] {
        let body = content::get(name).unwrap().body;
        assert!(!body.contains(".vitrify/"), "{name} points into .vitrify/");
    }
    // the gate the integrator runs includes the SDD's Lint and Test commands
    let integrate_brief = content::get("briefs/integrate.md").unwrap().body;
    assert!(integrate_brief.contains("The §5 Lint command."));
    assert!(integrate_brief.contains("The §5 Test command"));
    assert!(integrate_brief.contains("vitrify trace verify --unit <unit> --sdd <root>"));
    // every brief keeps the decision discipline
    for name in BRIEFS {
        let body = content::get(name).unwrap().body;
        assert!(
            body.contains("Q-###"),
            "{name} lost the decision discipline"
        );
        // no interpreter invocations, no symlinks
        assert!(!body.contains("python3"), "{name} mentions python3");
        assert!(
            !body.to_lowercase().contains("symlink"),
            "{name} mentions symlinks"
        );
    }
    // integrator and kickoff keep their structural laws
    let integrate = content::get("briefs/integrate.md").unwrap().body;
    assert!(integrate.contains("One unit at a time, always"));
    assert!(integrate.contains("never leave mainline failing"));
    let kickoff = content::get("briefs/kickoff.md").unwrap().body;
    assert!(kickoff.contains("resume = re-derive"));
}

/// Every task `vitrify brief` serves, in index order.
const BRIEFS: [&str; 7] = [
    "briefs/onboard.md",
    "briefs/authoring.md",
    "briefs/change.md",
    "briefs/run.md",
    "briefs/kickoff.md",
    "briefs/build-unit.md",
    "briefs/integrate.md",
];

#[test]
fn every_brief_opens_with_its_use_and_version_and_ends_with_its_report() {
    for name in BRIEFS {
        let body = content::get(name).unwrap().body;
        // build-unit's job block comes first: the worker's prompt opens with it
        let head: Vec<&str> = body.split("\n## ").next().unwrap().lines().collect();
        let title = head.iter().position(|l| l.starts_with("# ")).unwrap();
        // the purpose: a paragraph right under the title
        assert!(
            head[title + 1].is_empty() && !head[title + 2].is_empty(),
            "{name}"
        );
        for label in ["Use when: ", "Not for: ", "Version: vitrify {version}"] {
            assert!(
                head.iter().any(|l| l.starts_with(label)),
                "{name} lacks `{label}` before its first section"
            );
        }
        let last = body.rsplit("\n## ").next().unwrap();
        assert!(
            last.to_lowercase().contains("report"),
            "{name} does not end with its report"
        );
    }
    let run = content::get("briefs/run.md").unwrap().body;
    assert!(run.contains("{status_check_seconds}"));
}

#[test]
fn escalation_template_carries_slots_and_schema() {
    let esc = content::get("briefs/escalation.md").unwrap().body;
    for slot in [
        "{unit}",
        "{branch}",
        "{fencing}",
        "{command}",
        "{output_tail}",
        "{journal_tail}",
    ] {
        assert!(esc.contains(slot), "missing slot {slot}");
    }
    assert!(esc.contains(
        r#"{"verdict":"retry|release|block|abort","unit":"U-### or null","q":"Q-### or null","reason":"<one line>"}"#
    ));
    assert!(
        esc.contains("one re-ask") || esc.contains("One malformed reply gets exactly one re-ask")
    );
}

#[test]
fn instance_templates_present() {
    assert!(content::get("templates/tdd-template.md").is_some());
    assert!(content::get("templates/sdd-template-single.md").is_some());
    assert!(content::get("templates/sdd-template-bundle/sdd.md").is_some());
    assert!(content::get("templates/sdd-template-bundle/units/U-001-foundation.md").is_some());
    assert!(content::get("templates/sdd-template-bundle/contracts/C-001-example.md").is_some());
    assert!(content::get("templates/sdd-template-bundle/changelog.md").is_some());
    assert!(content::get("templates/sdd-template-bundle/decisions.md").is_some());
    // the normative schemas are embedded byte-identically
    let schema = content::get("schemas/sdd-schema.md").unwrap().body;
    let disk = std::fs::read_to_string(crate_root().join("assets/schemas/sdd-schema.md")).unwrap();
    assert_eq!(schema, disk);
}
