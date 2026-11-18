//! Discovery, instance model, claimable predicate, and
//! reading sets — semantic equality against the golden corpus.

use std::path::{Path, PathBuf};

use vitrify_core::contract::Peers;
use vitrify_core::model::{self, Instance, Shape, UtcTime};
use vitrify_core::readset;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn example_dir() -> PathBuf {
    repo_root().join("fixtures/example")
}

fn golden(name: &str) -> serde_json::Value {
    let p = repo_root().join("fixtures/golden/calc").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn load_calc() -> Instance {
    Instance::discover(&example_dir(), None, &example_dir()).unwrap()
}

fn subst_root(golden_path: &str) -> String {
    // golden {ROOT} is the example root; compare canonicalized
    let root = example_dir().canonicalize().unwrap();
    golden_path.replace("{ROOT}", root.to_str().unwrap())
}

fn canon(p: &Path) -> String {
    p.canonicalize().unwrap().to_str().unwrap().to_string()
}

#[test]
fn locate_units_matches_golden() {
    let inst = load_calc();
    assert_eq!(inst.shape, Shape::Bundle);
    let g = golden("locate.json");
    assert_eq!(subst_root(g["sdd"].as_str().unwrap()), canon(&inst.root));
    assert_eq!(g["shape"], "bundle");
    assert_eq!(
        g["version"].as_str().unwrap(),
        inst.meta.get_str("version").unwrap()
    );
    assert_eq!(
        g["status"].as_str().unwrap(),
        inst.meta.get_str("status").unwrap()
    );

    let gunits = g["units"].as_array().unwrap();
    assert_eq!(gunits.len(), inst.units.len());
    let now = UtcTime::now();
    for (gu, u) in gunits.iter().zip(&inst.units) {
        let v = model::unit_view(u, &inst.units, now);
        assert_eq!(gu["unit"].as_str().unwrap(), v.unit);
        assert_eq!(gu["status"].as_str().unwrap(), v.status);
        match gu["parallel_group"].as_str() {
            Some(s) => assert_eq!(Some(s.to_string()), v.parallel_group),
            None => assert_eq!(None, v.parallel_group),
        }
        let gdeps = gu["depends_on"].as_object().unwrap();
        assert_eq!(gdeps.len(), v.depends_on.len());
        for (dep, status) in &v.depends_on {
            assert_eq!(
                gdeps[dep].as_str().unwrap(),
                status,
                "dep {dep} of {}",
                v.unit
            );
        }
        match gu["claimed_by"].as_str() {
            Some(s) => assert_eq!(Some(s.to_string()), v.claimed_by),
            None => assert_eq!(None, v.claimed_by),
        }
        match gu["claim_expires"].as_str() {
            Some(s) => assert_eq!(Some(s.to_string()), v.claim_expires),
            None => assert_eq!(None, v.claim_expires),
        }
        assert_eq!(gu["claim_seq"].as_i64().unwrap(), v.claim_seq);
        assert_eq!(gu["lease_live"].as_bool().unwrap(), v.lease_live);
        assert_eq!(gu["claimable"].as_bool().unwrap(), v.claimable);
        assert_eq!(subst_root(gu["file"].as_str().unwrap()), canon(&v.file));
    }
}

#[test]
fn readset_matches_golden_per_unit() {
    let inst = load_calc();
    for uid in ["U-001", "U-002", "U-003", "U-004", "U-005"] {
        let items = readset::resolve(&inst, uid, &Peers::of(Vec::new())).unwrap();
        let g = golden(&format!("readset-{uid}.json"));
        assert_eq!(g["unit"].as_str().unwrap(), uid);
        assert_eq!(subst_root(g["sdd"].as_str().unwrap()), canon(&inst.root));
        let gconsumes: Vec<&str> = g["consumes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(gconsumes, inst.unit(uid).unwrap().meta.get_list("consumes"));
        let greadings = g["readings"].as_array().unwrap();
        assert_eq!(greadings.len(), items.len(), "{uid}");
        for (gr, item) in greadings.iter().zip(&items) {
            assert_eq!(subst_root(gr["path"].as_str().unwrap()), canon(&item.path));
            match gr["region"].as_str() {
                Some(s) => assert_eq!(Some(s.to_string()), item.region),
                None => assert_eq!(None, item.region),
            }
            assert!(item.bytes > 0, "{uid}: zero-byte reading");
        }
    }
}

fn mk_unit(block: &str) -> model::Unit {
    let meta = model::Meta(vitrify_core::yamlish::parse(block));
    model::Unit {
        unit: meta.get_str("unit").unwrap().to_string(),
        file: PathBuf::from("mem"),
        meta,
    }
}

#[test]
fn claimable_predicate_truth_table() {
    let now = UtcTime::parse("2026-10-30T12:00:00Z").unwrap();
    let dep_built = mk_unit("unit: U-001\nstatus: Built\n");
    let dep_ready = mk_unit("unit: U-002\nstatus: Ready\n");

    let cases: Vec<(&str, bool)> = vec![
        // deps ok + Ready → claimable
        ("unit: U-010\nstatus: Ready\ndepends_on: [U-001]\n", true),
        // unmet dep → not claimable
        ("unit: U-011\nstatus: Ready\ndepends_on: [U-002]\n", false),
        // missing dep → not claimable
        ("unit: U-012\nstatus: Ready\ndepends_on: [U-099]\n", false),
        // Stale rejoins
        ("unit: U-013\nstatus: Stale\n", true),
        // Draft never claimable
        ("unit: U-014\nstatus: Draft\n", false),
        // Built / Verified never claimable
        ("unit: U-015\nstatus: Built\n", false),
        ("unit: U-016\nstatus: Verified\n", false),
        // Claimed with live lease → not claimable
        (
            "unit: U-017\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: 2026-10-30T13:00:00Z\n",
            false,
        ),
        // Claimed with lapsed lease → claimable again
        (
            "unit: U-018\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: 2026-10-30T11:00:00Z\n",
            true,
        ),
        // Claimed with expiry exactly now (expires ≤ now) → lapsed
        (
            "unit: U-019\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: 2026-10-30T12:00:00Z\n",
            true,
        ),
        // Claimed with unparseable expiry → treated lapsed
        (
            "unit: U-020\nstatus: Claimed\nclaimed_by: w1\nclaim_expires: —\n",
            true,
        ),
    ];
    for (block, expected) in cases {
        let u = mk_unit(block);
        let all = vec![dep_built.clone(), dep_ready.clone(), u.clone()];
        assert_eq!(
            model::is_claimable(&u, &all, now),
            expected,
            "case: {block}"
        );
    }
}

#[test]
fn two_sdds_without_arg_is_refusal_4() {
    let tmp = tempfile::tempdir().unwrap();
    for slug in ["alpha", "beta"] {
        let dir = tmp.path().join("specs").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sdd.md"),
            format!("---\nsdd: {slug}\nschema_version: 2.0\nversion: 0.1.0\nstatus: Draft\n---\n# {slug}\n"),
        )
        .unwrap();
    }
    let err = Instance::discover(tmp.path(), None, tmp.path()).unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert!(err.to_string().contains("multiple SDDs found"));

    // naming one resolves it
    let arg = tmp.path().join("specs/alpha").to_str().unwrap().to_string();
    let inst = Instance::discover(tmp.path(), Some(&arg), tmp.path()).unwrap();
    assert_eq!(inst.meta.get_str("sdd"), Some("alpha"));
}

#[test]
fn repo_wide_search_skips_only_git_and_vitrify() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let sdd = |rel: &str, slug: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            p,
            format!("---\nsdd: {slug}\nschema_version: 2.0\nversion: 0.1.0\nstatus: Draft\n---\n"),
        )
        .unwrap();
    };
    sdd("docs/a/sdd.md", "a");
    sdd("node_modules/pkg/sdd.md", "pkg"); // no directory name is special
    sdd(".git/x/sdd.md", "git-internal"); // git's own: never searched
    sdd(".vitrify/mainline/docs/m/sdd.md", "m"); // vitrify's own: never searched
    let slugs = |found: Vec<model::SddHandle>| {
        let mut v: Vec<String> = found
            .iter()
            .map(|h| h.meta.get_str("sdd").unwrap().to_string())
            .collect();
        v.sort();
        v
    };

    // without git: every file on disk except under .git and .vitrify
    assert_eq!(slugs(model::discover(&root)), ["a", "pkg"]);

    // with git: the files git would commit decide instead
    let candidates = vec!["docs/a/sdd.md".to_string()];
    assert_eq!(slugs(model::discover_with(&root, Some(&candidates))), ["a"]);

    // the skip test is root-relative: a root inside .vitrify (vitrify's own
    // worktrees) still searches its whole tree
    assert_eq!(
        slugs(model::discover(&root.join(".vitrify/mainline"))),
        ["m"]
    );
}

const SINGLE_SDD: &str = r#"---
sdd: mini
schema_version: 2.0
title: mini single-file SDD
version: 0.2.0
status: Active
scope_roots:
  - src/
---

# mini

## Ground Rules

Rules text.

## Technology Decisions

Tech text.

## Interface Contracts

### C-001 — api

Contract body.

## Work Units

### U-001 — first

```yaml
unit: U-001
status: Ready
depends_on: []
owns: [src/a.rs]
consumes: [C-001]
```

Body of first.

### U-002 — second

```yaml
unit: U-002
status: Draft
depends_on: [U-001]
owns: [src/b.rs]
consumes: []
```

Body of second.

## Notes

Tail section.
"#;

#[test]
fn single_shape_regions_and_units() {
    let tmp = tempfile::tempdir().unwrap();
    let specs = tmp.path().join("specs");
    std::fs::create_dir_all(&specs).unwrap();
    std::fs::write(specs.join("mini.sdd.md"), SINGLE_SDD).unwrap();

    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    assert_eq!(inst.shape, Shape::Single);
    assert_eq!(inst.units.len(), 2);
    assert_eq!(inst.units[0].unit, "U-001");
    assert_eq!(inst.units[1].meta.get_str("status"), Some("Draft"));

    // region extraction
    let text = std::fs::read_to_string(&inst.file).unwrap();
    let (s, e) = model::find_unit_span(&text, "U-001").unwrap();
    let region = &text[s..e];
    assert!(region.starts_with("### U-001"));
    assert!(region.contains("Body of first."));
    assert!(!region.contains("U-002 — second"));

    // contract region
    let ct = model::contract_text(&inst.handle(), "C-001")
        .unwrap()
        .unwrap();
    assert!(ct.starts_with("### C-001"));
    assert!(ct.contains("Contract body."));

    // section extraction stops at the next H2
    let gr = model::section(&text, "Ground Rules");
    assert!(gr.starts_with("## Ground Rules"));
    assert!(gr.contains("Rules text."));
    assert!(!gr.contains("Technology"));

    // readset over the single shape names regions
    let items = readset::resolve(&inst, "U-001", &Peers::of(Vec::new())).unwrap();
    assert_eq!(items[2].region.as_deref(), Some("Work Units / U-001"));
    assert_eq!(
        items[3].region.as_deref(),
        Some("Interface Contracts / C-001")
    );
}

#[test]
fn stamp_parse_and_render() {
    let stamp = model::render_stamp("2026-10-30", "1.0.0", "abc1234");
    assert_eq!(stamp, "2026-10-30 @ SDD 1.0.0, commit abc1234");
    assert_eq!(model::stamp_version(&stamp), "1.0.0");
    assert_eq!(model::stamp_version("—"), "—");
    assert_eq!(model::stamp_version(""), "—");
    for marker in ["", "-", "—", "~", "null", "None"] {
        assert!(model::is_empty_marker(marker));
    }
    assert!(!model::is_empty_marker("x"));
}

#[test]
fn utc_time_parse_formats() {
    let t = UtcTime::parse("2026-10-30T12:34:56Z").unwrap();
    assert_eq!(t.format(), "2026-10-30T12:34:56Z");
    assert_eq!(
        UtcTime::parse("2026-10-30T12:34Z").unwrap().format(),
        "2026-10-30T12:34:00Z"
    );
    assert_eq!(
        UtcTime::parse("2026-10-30 12:34:56Z").unwrap().format(),
        "2026-10-30T12:34:56Z"
    );
    assert_eq!(
        UtcTime::parse("2026-10-30T12:34:56+00:00")
            .unwrap()
            .format(),
        "2026-10-30T12:34:56Z"
    );
    assert!(UtcTime::parse("—").is_none());
    assert!(UtcTime::parse("garbage").is_none());
    assert_eq!(
        UtcTime::parse("2026-10-30T12:00:00Z")
            .unwrap()
            .add_minutes(90)
            .format(),
        "2026-10-30T13:30:00Z"
    );
}
