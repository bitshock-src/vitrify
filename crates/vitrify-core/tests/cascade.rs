//! The re-check cascade against brute force: on random graphs, ownerships
//! and histories the dirty set equals what hashing each unit's input paths
//! at its anchor and at the head says, plus the named cases (protocol-only
//! commit, foundation change, early cutoff, outside change, spec change,
//! reasons) and the journal marks the cascade stores its anchors in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use vitrify_core::audit::{self, CommandRunner, Options};
use vitrify_core::cascade::{
    self, Anchors, Candidate, Changes, Closures, Dirty, Owner, Ownership, Reason, SddOwnership, Why,
};
use vitrify_core::graph::Node;
use vitrify_core::journal::{self, Event};
use vitrify_core::model::{Instance, UtcTime};

// ------------------------------------------------------------------ PRNG

/// xorshift64* — seeded, so a failing seed replays.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

// ------------------------------------------------------------ a world

type Snapshot = BTreeMap<String, Vec<u8>>;

struct World {
    ownership: Ownership,
    /// Every path of the universe with the owner its generation gave it.
    expected: BTreeMap<String, Owner>,
    nodes: Vec<Node>,
    closures: Closures,
    /// slug → `shared_inputs` patterns the brute force understands.
    shared: BTreeMap<String, Option<Vec<String>>>,
}

fn n(slug: &str, unit: &str) -> Node {
    Node::new(slug, unit)
}

fn world(rng: &mut Rng) -> World {
    let nsdd = 2 + rng.below(3);
    let mut parts = Vec::new();
    let mut expected = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut shared = BTreeMap::new();
    for si in 0..nsdd {
        let slug = format!("s{si}");
        let mut sdd = SddOwnership {
            slug: slug.clone(),
            scope_roots: vec![format!("src/{slug}/")],
            ..Default::default()
        };
        // spec files: a bundle directory or a single file
        if rng.chance(1, 2) {
            sdd.spec_paths = vec![format!("specs/{slug}")];
            for f in ["sdd.md", "journal.ndjson", "units/U-001.md"] {
                expected.insert(format!("specs/{slug}/{f}"), Owner::Spec(slug.clone()));
            }
        } else {
            sdd.spec_paths = vec![format!("specs/{slug}.md")];
            expected.insert(format!("specs/{slug}.md"), Owner::Spec(slug.clone()));
        }
        for ui in 0..1 + rng.below(4) {
            let unit = format!("U-{:03}", ui + 1);
            let dir = format!("src/{slug}/{unit}");
            let files: Vec<String> = (0..1 + rng.below(3))
                .map(|i| format!("{dir}/f{i}.rs"))
                .collect();
            let owns = match rng.below(3) {
                0 => vec![format!("{dir}/")],
                1 => files.clone(),
                _ => vec![format!("{dir}/*.rs")],
            };
            for f in &files {
                expected.insert(f.clone(), Owner::Unit(n(&slug, &unit)));
            }
            // a file no glob of the unit covers, when the style leaves one
            if owns[0].ends_with("*.rs") {
                expected.insert(format!("{dir}/notes.txt"), Owner::Unmanaged(slug.clone()));
            }
            sdd.units.push((unit.clone(), owns));
            nodes.push(n(&slug, &unit));
        }
        // under the scope, owned by no one, and listed unmanaged
        expected.insert(
            format!("src/{slug}/misc/orphan.txt"),
            Owner::Unmanaged(slug.clone()),
        );
        sdd.unmanaged = vec![format!("src/{slug}/gen/")];
        expected.insert(
            format!("src/{slug}/gen/out.bin"),
            Owner::Unmanaged(slug.clone()),
        );
        // shared inputs: none declared, or a subset of the outside paths
        let declared = match rng.below(3) {
            0 => None,
            1 => Some(vec!["tools/".to_string()]),
            _ => Some(vec!["ci/shared.env".to_string(), "tools/t1.sh".to_string()]),
        };
        sdd.shared_inputs = declared.clone();
        shared.insert(slug.clone(), declared);
        parts.push(sdd);
    }
    for p in [
        "tools/t0.sh",
        "tools/t1.sh",
        "ci/shared.env",
        "ci/other.env",
        "README.md",
    ] {
        expected.insert(p.to_string(), Owner::Outside);
    }
    // a random DAG: a unit depends on some earlier ones, across SDDs too
    let mut closures = Closures::new();
    let mut direct: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
    let mut order = nodes.clone();
    rng.shuffle(&mut order);
    for (i, node) in order.iter().enumerate() {
        let deps: Vec<Node> = order[..i]
            .iter()
            .filter(|_| rng.chance(1, 3))
            .cloned()
            .collect();
        direct.insert(node.clone(), deps);
    }
    for node in &nodes {
        let mut seen = BTreeSet::new();
        let mut stack = direct[node].clone();
        while let Some(d) = stack.pop() {
            if seen.insert(d.clone()) {
                stack.extend(direct[&d].iter().cloned());
            }
        }
        if !seen.is_empty() {
            closures.insert(node.clone(), seen);
        }
    }
    nodes.sort();
    World {
        ownership: Ownership::new(parts),
        expected,
        nodes,
        closures,
        shared,
    }
}

/// The paths a unit — or, with `slug_only`, the SDD itself — is checked
/// against, from the generated owners alone.
fn inputs_of(w: &World, units: &[&Node], slug: &str) -> BTreeSet<String> {
    let mut relevant: BTreeSet<Node> = BTreeSet::new();
    for u in units {
        relevant.insert((*u).clone());
        relevant.extend(w.closures.get(*u).into_iter().flatten().cloned());
    }
    let outside_ok = |p: &str| match &w.shared[slug] {
        None => true,
        Some(pats) => pats.iter().any(|g| match g.strip_suffix('/') {
            Some(dir) => p.starts_with(&format!("{dir}/")),
            None => p == g,
        }),
    };
    w.expected
        .iter()
        .filter(|(p, o)| match o {
            Owner::Spec(_) => false,
            Owner::Unit(node) => relevant.contains(node),
            Owner::Unmanaged(s) => s == slug,
            Owner::Outside => outside_ok(p),
        })
        .map(|(p, _)| p.clone())
        .collect()
}

fn digest(snap: &Snapshot, paths: &BTreeSet<String>) -> Vec<u8> {
    let mut h = Sha256::new();
    for p in paths {
        h.update(p.as_bytes());
        match snap.get(p) {
            Some(bytes) => {
                h.update([1]);
                h.update(bytes);
            }
            None => h.update([0]),
        }
        h.update([0xff]);
    }
    h.finalize().to_vec()
}

fn diff(a: &Snapshot, b: &Snapshot) -> Vec<String> {
    let all: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    all.into_iter()
        .filter(|p| a.get(*p) != b.get(*p))
        .cloned()
        .collect()
}

fn mutate(rng: &mut Rng, snap: &mut Snapshot, universe: &[String]) {
    // some commits change nothing in the universe (protocol state only)
    if rng.chance(1, 4) {
        return;
    }
    for _ in 0..1 + rng.below(3) {
        let p = &universe[rng.below(universe.len())];
        match rng.below(4) {
            0 => {
                snap.remove(p);
            }
            _ => {
                snap.insert(p.clone(), vec![rng.below(256) as u8, rng.below(256) as u8]);
            }
        }
    }
}

#[test]
fn classify_matches_the_generated_owners() {
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let w = world(&mut rng);
        for (path, owner) in &w.expected {
            assert_eq!(&w.ownership.classify(path), owner, "seed {seed}: {path}");
        }
    }
}

#[test]
fn the_dirty_set_equals_brute_force_hashing_of_each_units_inputs() {
    for seed in 1..=300 {
        let mut rng = Rng::new(seed);
        let w = world(&mut rng);
        let universe: Vec<String> = w.expected.keys().cloned().collect();
        // a history: snapshots c0..c4, then the head
        let mut snaps: Vec<Snapshot> = Vec::new();
        let mut cur = Snapshot::new();
        for p in &universe {
            if rng.chance(3, 4) {
                cur.insert(p.clone(), vec![rng.below(256) as u8]);
            }
        }
        snaps.push(cur.clone());
        for _ in 0..4 {
            mutate(&mut rng, &mut cur, &universe);
            snaps.push(cur.clone());
        }
        mutate(&mut rng, &mut cur, &universe);
        let head = cur;

        // each unit is anchored at a random commit; one diff per anchor
        let mut cands = Vec::new();
        let mut anchor_of: BTreeMap<Node, usize> = BTreeMap::new();
        for node in &w.nodes {
            let a = rng.below(snaps.len());
            anchor_of.insert(node.clone(), a);
            cands.push(Candidate {
                node: node.clone(),
                anchor: Some(format!("c{a}")),
            });
        }
        rng.shuffle(&mut cands);
        let mut changes = Changes::new();
        for (i, s) in snaps.iter().enumerate() {
            let mut paths = diff(s, &head);
            rng.shuffle(&mut paths);
            changes.insert(format!("c{i}"), paths);
        }

        let got = cascade::dirty_set(&cands, &changes, &w.closures, &w.ownership);
        let mut want: Vec<Dirty> = Vec::new();
        for node in &w.nodes {
            let inputs = inputs_of(&w, &[node], &node.slug);
            let snap = &snaps[anchor_of[node]];
            if digest(snap, &inputs) == digest(&head, &inputs) {
                continue;
            }
            let reasons: Vec<Reason> = diff(snap, &head)
                .into_iter()
                .filter(|p| inputs.contains(p))
                .map(|p| Reason {
                    owner: w.expected[&p].clone(),
                    path: p,
                })
                .collect();
            want.push(Dirty {
                node: node.clone(),
                why: Why::Paths(reasons),
            });
        }
        assert_eq!(got, want, "seed {seed}");

        // the suite: dirty when any of the SDD's units' inputs changed
        for si in w.shared.keys() {
            let units: Vec<&Node> = w.nodes.iter().filter(|x| &x.slug == si).collect();
            let a = rng.below(snaps.len());
            let inputs = inputs_of(&w, &units, si);
            let unit_nodes: Vec<Node> = units.iter().map(|x| (*x).clone()).collect();
            let got = cascade::suite_dirty(
                si,
                &unit_nodes,
                Some(&format!("c{a}")),
                Some(&changes[&format!("c{a}")]),
                &w.closures,
                &w.ownership,
            );
            let dirty = digest(&snaps[a], &inputs) != digest(&head, &inputs);
            assert_eq!(got.is_some(), dirty, "seed {seed}: suite of {si}");
            if let Some(Why::Paths(reasons)) = got {
                let got: BTreeSet<String> = reasons.into_iter().map(|r| r.path).collect();
                let wanted: BTreeSet<String> = diff(&snaps[a], &head)
                    .into_iter()
                    .filter(|p| inputs.contains(p))
                    .collect();
                assert_eq!(got, wanted, "seed {seed}: suite reasons of {si}");
            }
        }
    }
}

// ------------------------------------------------------------ named cases

/// Three SDDs. `core` is a chain U-001 <- U-002 <- U-003 with U-004
/// standalone; `api` U-001 depends on `core` U-001 through a pin; `docs`
/// stands alone.
struct Fixture {
    ownership: Ownership,
    closures: Closures,
    cands: Vec<Candidate>,
}

fn sdd(slug: &str, units: &[&str]) -> SddOwnership {
    SddOwnership {
        slug: slug.into(),
        spec_paths: vec![format!("specs/{slug}")],
        scope_roots: vec![format!("{slug}/")],
        units: units
            .iter()
            .map(|u| (u.to_string(), vec![format!("{slug}/{u}/")]))
            .collect(),
        ..Default::default()
    }
}

fn fixture() -> Fixture {
    let ownership = Ownership::new(vec![
        sdd("core", &["U-001", "U-002", "U-003", "U-004"]),
        sdd("api", &["U-001"]),
        sdd("docs", &["U-001"]),
    ]);
    let mut closures = Closures::new();
    closures.insert(n("core", "U-002"), BTreeSet::from([n("core", "U-001")]));
    closures.insert(
        n("core", "U-003"),
        BTreeSet::from([n("core", "U-001"), n("core", "U-002")]),
    );
    closures.insert(n("api", "U-001"), BTreeSet::from([n("core", "U-001")]));
    let cands = ["core", "api", "docs"]
        .iter()
        .flat_map(|s| {
            let units: &[&str] = match *s {
                "core" => &["U-001", "U-002", "U-003", "U-004"],
                _ => &["U-001"],
            };
            units.iter().map(move |u| Candidate {
                node: n(s, u),
                anchor: Some("a".to_string()),
            })
        })
        .collect::<Vec<_>>();
    Fixture {
        ownership,
        closures,
        cands,
    }
}

fn dirty_nodes(f: &Fixture, changed: &[&str]) -> Vec<String> {
    let changes = Changes::from([(
        "a".to_string(),
        changed.iter().map(|p| p.to_string()).collect(),
    )]);
    cascade::dirty_set(&f.cands, &changes, &f.closures, &f.ownership)
        .into_iter()
        .map(|d| d.node.to_string())
        .collect()
}

#[test]
fn a_protocol_state_only_change_dirties_nothing() {
    // protocol-only commits list no path once the caller has dropped them
    let f = fixture();
    assert!(dirty_nodes(&f, &[]).is_empty());
}

#[test]
fn a_foundation_change_dirties_its_dependents_in_every_sdd() {
    let f = fixture();
    assert_eq!(
        dirty_nodes(&f, &["core/U-001/lib.rs"]),
        ["api U-001", "core U-001", "core U-002", "core U-003"]
    );
    // a leaf of the chain reaches only itself
    assert_eq!(dirty_nodes(&f, &["core/U-003/x.rs"]), ["core U-003"]);
    // a unit nobody depends on, in another SDD
    assert_eq!(dirty_nodes(&f, &["docs/U-001/x.md"]), ["docs U-001"]);
    // the cross-SDD dependent's own files dirty nothing upstream
    assert_eq!(dirty_nodes(&f, &["api/U-001/x.rs"]), ["api U-001"]);
}

#[test]
fn identical_regeneration_dirties_nothing() {
    // the regenerated unit rewrote byte-identical files: git lists no path
    let f = fixture();
    assert!(dirty_nodes(&f, &[]).is_empty());
}

#[test]
fn a_change_outside_every_scope_dirties_everything_unless_shared_inputs_exclude_it() {
    let mut f = fixture();
    assert_eq!(dirty_nodes(&f, &["tools/gen.sh"]).len(), f.cands.len());

    // api declares what it reads; docs declares nothing outside is read
    let mut parts = vec![
        sdd("core", &["U-001", "U-002", "U-003", "U-004"]),
        sdd("api", &["U-001"]),
        sdd("docs", &["U-001"]),
    ];
    parts[1].shared_inputs = Some(vec!["ci/shared.env".into()]);
    parts[2].shared_inputs = Some(vec![]);
    f.ownership = Ownership::new(parts);
    let got = dirty_nodes(&f, &["tools/gen.sh"]);
    assert_eq!(
        got,
        ["core U-001", "core U-002", "core U-003", "core U-004"]
    );
    let got = dirty_nodes(&f, &["ci/shared.env"]);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got.contains(&"api U-001".to_string()));
    assert!(!got.contains(&"docs U-001".to_string()));
}

#[test]
fn spec_file_changes_dirty_nothing() {
    let f = fixture();
    assert!(
        dirty_nodes(
            &f,
            &[
                "specs/core/units/U-001.md",
                "specs/api/sdd.md",
                "specs/docs/journal.ndjson"
            ]
        )
        .is_empty()
    );
}

#[test]
fn reasons_name_the_changed_paths_and_their_owners() {
    let mut f = fixture();
    f.ownership = {
        let mut core = sdd("core", &["U-001", "U-002", "U-003", "U-004"]);
        core.unmanaged = vec!["core/gen/".into()];
        Ownership::new(vec![core, sdd("api", &["U-001"]), sdd("docs", &["U-001"])])
    };
    let changes = Changes::from([(
        "a".to_string(),
        vec![
            "tools/gen.sh".to_string(),
            "core/gen/x.txt".to_string(),
            "core/U-001/b.rs".to_string(),
            "core/U-001/a.rs".to_string(),
            "core/stray.txt".to_string(),
            "specs/core/sdd.md".to_string(),
        ],
    )]);
    let dirty = cascade::dirty_set(&f.cands, &changes, &f.closures, &f.ownership);
    let by: BTreeMap<String, &Why> = dirty.iter().map(|d| (d.node.to_string(), &d.why)).collect();
    // core U-002 depends on U-001: U-001's files, the SDD's unmanaged and
    // stray files, and the outside file; never the spec file
    assert_eq!(
        by["core U-002"].describe(),
        "core/U-001/a.rs (unit core U-001), core/U-001/b.rs (unit core U-001), \
         core/gen/x.txt (unmanaged in core), core/stray.txt (unmanaged in core), \
         tools/gen.sh (outside every scope)"
    );
    // core U-004 depends on nothing: no U-001 file among its reasons
    assert_eq!(
        by["core U-004"].describe(),
        "core/gen/x.txt (unmanaged in core), core/stray.txt (unmanaged in core), \
         tools/gen.sh (outside every scope)"
    );
    // another SDD sees core's unmanaged files only through its closure
    assert_eq!(
        by["api U-001"].describe(),
        "core/U-001/a.rs (unit core U-001), core/U-001/b.rs (unit core U-001), \
         tools/gen.sh (outside every scope)"
    );
    // docs reads nothing of core's, but the outside file reaches it
    assert_eq!(
        by["docs U-001"].describe(),
        "tools/gen.sh (outside every scope)"
    );
    // the output is sorted by node, one entry each
    assert_eq!(dirty.len(), 6);
    assert!(dirty.windows(2).all(|w| w[0].node < w[1].node));
}

#[test]
fn units_integrated_in_one_commit_share_one_diff() {
    let f = fixture();
    // a second anchor, with its own diff; units anchored at each see theirs
    let mut cands = f.cands.clone();
    for c in &mut cands {
        if c.node.slug == "docs" {
            c.anchor = Some("b".into());
        }
    }
    let changes = Changes::from([
        ("a".to_string(), vec!["core/U-004/x.rs".to_string()]),
        ("b".to_string(), vec!["tools/x.sh".to_string()]),
    ]);
    let got: Vec<String> = cascade::dirty_set(&cands, &changes, &f.closures, &f.ownership)
        .into_iter()
        .map(|d| d.node.to_string())
        .collect();
    assert_eq!(got, ["core U-004", "docs U-001"]);
}

// ------------------------------------------- real instances and the journal

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

#[test]
fn ownership_reads_an_instances_declarations() {
    let root = repo_root().join("fixtures/example");
    let inst = Instance::discover(&root, None, &root).unwrap();
    let own = Ownership::build(&root, [("calc", &inst)]).unwrap();
    assert_eq!(
        own.classify("calc/__main__.py"),
        Owner::Unit(n("calc", "U-005"))
    );
    assert_eq!(
        own.classify("tests/test_a.py"),
        Owner::Unit(n("calc", "U-001"))
    );
    assert_eq!(
        own.classify("calc/__pycache__/x.pyc"),
        Owner::Unmanaged("calc".into())
    );
    assert_eq!(
        own.classify("calc/zzz.txt"),
        Owner::Unmanaged("calc".into())
    );
    assert_eq!(
        own.classify("specs/calc/units/U-001-foundation.md"),
        Owner::Spec("calc".into())
    );
    assert_eq!(own.classify("tools/x.sh"), Owner::Outside);
    assert_eq!(own.shared_inputs("calc"), None);
}

#[test]
fn shared_inputs_come_from_front_matter() {
    let tmp = tempfile::tempdir().unwrap();
    let sdd_dir = tmp.path().join("specs/x");
    std::fs::create_dir_all(sdd_dir.join("units")).unwrap();
    std::fs::write(
        sdd_dir.join("sdd.md"),
        "---\nsdd: x\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\n\
         scope_roots:\n  - src/\nshared_inputs: [tools/, ci/shared.env]\n---\n",
    )
    .unwrap();
    let y_dir = tmp.path().join("specs/y");
    std::fs::create_dir_all(&y_dir).unwrap();
    std::fs::write(
        y_dir.join("sdd.md"),
        "---\nsdd: y\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\nshared_inputs: []\n---\n",
    )
    .unwrap();
    let insts: Vec<Instance> = vitrify_core::model::discover(tmp.path())
        .into_iter()
        .map(|h| Instance::load(h).unwrap())
        .collect();
    assert_eq!(insts.len(), 2);
    let own = Ownership::build(
        tmp.path(),
        insts.iter().map(|i| (i.meta.get_str_or("sdd", "?"), i)),
    )
    .unwrap();
    assert_eq!(
        own.shared_inputs("x"),
        Some(&["tools/".to_string(), "ci/shared.env".to_string()][..])
    );
    // declared, and empty: no outside path is an input
    assert_eq!(own.shared_inputs("y"), Some(&[][..]));
    assert_eq!(
        own.classify("specs/x/units/U-001.md"),
        Owner::Spec("x".into())
    );
    assert_eq!(own.classify("src/a.rs"), Owner::Unmanaged("x".into()));
    assert_eq!(own.classify("tools/a.sh"), Owner::Outside);
}

fn event(seq: i64, kind: &str, unit: Option<&str>, actor: Option<&str>, commit: &str) -> Event {
    Event {
        seq,
        at: "2026-11-01T00:00:00Z".into(),
        event: kind.into(),
        unit: unit.map(Into::into),
        actor: actor.map(Into::into),
        sdd: "0.1.0".into(),
        commit: Some(commit.into()),
        ..Default::default()
    }
}

#[test]
fn recheck_and_suite_marks_render_in_key_order_and_read_back() {
    let recheck = event(5, "verified", Some("U-001"), Some("recheck"), "abc1234");
    let line = journal::render_line(&recheck);
    assert_eq!(
        line,
        r#"{"seq":5,"at":"2026-11-01T00:00:00Z","event":"verified","unit":"U-001","actor":"recheck","sdd":"0.1.0","commit":"abc1234"}"#
    );
    assert!(journal::parse_line(&line).unwrap().is_recheck());

    let mut audit = event(6, "audit", None, Some("vitrify"), "abc1234");
    audit.findings = Some(0);
    audit.suite = Some("pass".into());
    let line = journal::render_line(&audit);
    assert_eq!(
        line,
        r#"{"seq":6,"at":"2026-11-01T00:00:00Z","event":"audit","actor":"vitrify","findings":0,"suite":"pass","sdd":"0.1.0","commit":"abc1234"}"#
    );
    let back = journal::parse_line(&line).unwrap();
    assert!(back.is_suite_pass());
    assert_eq!(journal::render_line(&back), line);

    // an audit event without the field renders as it always did
    audit.suite = None;
    assert!(!journal::render_line(&audit).contains("suite"));
}

#[test]
fn replay_keeps_verified_through_rechecks_and_they_are_no_finding() {
    let mut suite = event(6, "audit", None, Some("vitrify"), "c4");
    suite.suite = Some("pass".into());
    let events = vec![
        event(1, "claim", Some("U-001"), Some("w"), "c0"),
        event(2, "built", Some("U-001"), Some("w"), "c0"),
        event(3, "verified", Some("U-001"), Some("vitrify"), "c1"),
        event(4, "verified", Some("U-001"), Some("recheck"), "c3"),
        event(5, "verified", Some("U-001"), Some("recheck"), "c4"),
        suite,
    ];
    let proj = journal::replay(&events);
    assert_eq!(proj.status_of("U-001"), Some("Verified"));
    assert!(proj.order_findings.is_empty(), "{:?}", proj.order_findings);
    let a = Anchors::from_events(&events);
    assert_eq!(a.unit("U-001"), Some("c4"));
    assert_eq!(a.suite.as_deref(), Some("c4"));
}

struct Pass;
impl CommandRunner for Pass {
    fn run(&self, _cmd: &str, _cwd: &std::path::Path) -> i32 {
        0
    }
}

#[test]
fn the_audit_accepts_a_journal_with_rechecks_and_suite_passes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("specs/x");
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::write(
        dir.join("sdd.md"),
        "---\nsdd: x\nschema_version: 2.0\nversion: 0.1.0\nstatus: Active\nscope_roots:\n  - src/\n---\n\n# x\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("units/U-001.md"),
        "---\nunit: U-001\nstatus: Verified\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 1\n\
         depends_on: []\nowns:\n  - src/a.rs\nconsumes: []\n\
         verified: 2026-11-02 @ SDD 0.1.0, commit c3\n---\n\n# U-001\n",
    )
    .unwrap();
    let mut suite = event(6, "audit", None, Some("vitrify"), "c4");
    suite.suite = Some("pass".into());
    let mut claim = event(1, "claim", Some("U-001"), Some("w"), "c0");
    claim.claim_seq = Some(1);
    let events = [
        claim,
        event(2, "built", Some("U-001"), Some("w"), "c0"),
        event(3, "verified", Some("U-001"), Some("vitrify"), "c1"),
        event(4, "verified", Some("U-001"), Some("recheck"), "c3"),
        suite,
    ];
    let text: String = events
        .iter()
        .map(|e| format!("{}\n", journal::render_line(e)))
        .collect();
    std::fs::write(dir.join("journal.ndjson"), text).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/a.rs"), "x").unwrap();

    let inst = Instance::discover(tmp.path(), None, tmp.path()).unwrap();
    let (findings, _) = audit::run(
        &inst,
        tmp.path(),
        &Options {
            skip_rotten: true,
            ..Default::default()
        },
        &Pass,
        UtcTime::now(),
    )
    .unwrap();
    // the journal projects what the front matter says: no check-6 finding
    let six: Vec<_> = findings.iter().filter(|f| f.check == 6).collect();
    assert!(six.is_empty(), "{six:?}");
}
