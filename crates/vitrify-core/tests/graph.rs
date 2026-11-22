//! The unit graph across SDDs: `depends_on` entries classified, pins
//! followed to the units that produce their authority, Kahn's order, every
//! cycle as the path that closes it, closures, Hu's priorities and the
//! critical path, the derived SDD graph, and the claimable predicate
//! extended through pins — on small repositories written to temp dirs, and
//! on random ones checked against brute force.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use vitrify_core::claims::{self, Op};
use vitrify_core::graph::{
    self, Entry, EntryProblem, Node, PinProblem, ProblemKind, UnitGraph, render_path,
};
use vitrify_core::model::{self, Instance, UtcTime};

// ------------------------------------------------------------ scaffolding

/// A tiny seeded PRNG (xorshift64), so every random case reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        Repo { _tmp: tmp, root }
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// Every SDD of the repository, loaded.
    fn instances(&self) -> Vec<Instance> {
        model::discover(&self.root)
            .into_iter()
            .map(|h| Instance::load(h).unwrap())
            .collect()
    }

    fn graph(&self) -> UnitGraph {
        UnitGraph::build(&self.instances()).unwrap()
    }
}

/// A unit, as a spec declares it.
struct U {
    id: String,
    status: String,
    depends_on: Vec<String>,
    consumes: Vec<String>,
}

fn unit(id: &str, depends_on: &[&str], consumes: &[&str]) -> U {
    U {
        id: id.to_string(),
        status: "Ready".to_string(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        consumes: consumes.iter().map(|s| s.to_string()).collect(),
    }
}

impl U {
    fn status(mut self, s: &str) -> U {
        self.status = s.to_string();
        self
    }

    fn block(&self) -> String {
        format!(
            "unit: {}\ntitle: {} t\nstatus: {}\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [{}]\nparallel_group: —\nowns: [x/{}]\nconsumes: [{}]\n",
            self.id,
            self.id,
            self.status,
            self.depends_on.join(", "),
            self.id,
            self.consumes.join(", ")
        )
    }
}

/// A contract: produced here by units, or a pin of another SDD's, or a
/// transcription.
enum C {
    Internal(&'static str, String),
    Pin(&'static str, String, String),
    Transcription(&'static str),
}

fn internal(id: &'static str, producers: &str) -> C {
    C::Internal(id, producers.to_string())
}

fn pin(id: &'static str, slug: &str, cid: &str) -> C {
    C::Pin(id, slug.to_string(), cid.to_string())
}

impl C {
    fn id(&self) -> &str {
        match self {
            C::Internal(id, _) | C::Pin(id, _, _) | C::Transcription(id) => id,
        }
    }

    /// The contract's header and body, after its title.
    fn body(&self) -> String {
        match self {
            C::Internal(_, producers) => format!(
                "- Producer: {producers}\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\nsurface of {}\n",
                self.id()
            ),
            C::Pin(_, slug, cid) => format!(
                "- Producer: external — {slug}, {cid}\n- Consumers: —\n- Stability: Frozen by its producer\n- Pin: {slug} {cid} @ 000000000000\n"
            ),
            C::Transcription(_) => "- Producer: external — a vendor API\n- Consumers: —\n- Stability: Frozen by its producer\n\n## Surface\n\ntranscribed\n".to_string(),
        }
    }
}

fn front(slug: &str, schema: &str) -> String {
    format!(
        "---\nsdd: {slug}\ntitle: {slug}\nschema_version: {schema}\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - x/\n---\n\n# {slug}\n\n## Ground Rules\n\nOne implementer per unit.\n"
    )
}

/// Write a bundle SDD at `specs/<slug>/`.
fn bundle(repo: &Repo, slug: &str, schema: &str, contracts: &[C], units: &[U]) {
    repo.write(&format!("specs/{slug}/sdd.md"), &front(slug, schema));
    for c in contracts {
        repo.write(
            &format!("specs/{slug}/contracts/{}-c.md", c.id()),
            &format!("# {} — contract\n\n{}", c.id(), c.body()),
        );
    }
    for u in units {
        repo.write(
            &format!("specs/{slug}/units/{}-u.md", u.id),
            &format!("---\n{}---\n\n# {} — unit\n", u.block(), u.id),
        );
    }
}

/// Write a single-file SDD at `specs/<slug>.sdd.md`.
fn single(repo: &Repo, slug: &str, schema: &str, contracts: &[C], units: &[U]) {
    let mut text = front(slug, schema);
    text.push_str("\n## Interface Contracts\n\n");
    for c in contracts {
        text.push_str(&format!(
            "### {} — contract\n\n{}\n",
            c.id(),
            c.body().replace("## Surface", "#### Surface")
        ));
    }
    text.push_str("## Work Units\n\n");
    for u in units {
        text.push_str(&format!(
            "### {} — unit\n\n```yaml\n{}```\n\n#### Requirements\n\n- R1\n\n",
            u.id,
            u.block()
        ));
    }
    repo.write(&format!("specs/{slug}.sdd.md"), &text);
}

fn n(slug: &str, unit: &str) -> Node {
    Node::new(slug, unit)
}

fn names(nodes: &[&Node]) -> Vec<String> {
    nodes.iter().map(|n| n.to_string()).collect()
}

/// The worked example: two SDDs with three units each, chained
/// U-001 → U-002 → U-003 inside each; api U-001 depends on a pin of the
/// core contract core U-001 produces, api U-002 on a pin of the one core
/// U-002 produces. core is a bundle, api a single file.
fn pipeline(repo: &Repo, core: [&str; 3], api: [&str; 3]) {
    bundle(
        repo,
        "core",
        "2.1",
        &[internal("C-001", "U-001"), internal("C-002", "U-002")],
        &[
            unit("U-001", &[], &[]).status(core[0]),
            unit("U-002", &["U-001"], &["C-001"]).status(core[1]),
            unit("U-003", &["U-002"], &["C-002"]).status(core[2]),
        ],
    );
    single(
        repo,
        "api",
        "2.1",
        &[pin("C-001", "core", "C-001"), pin("C-002", "core", "C-002")],
        &[
            unit("U-001", &["C-001"], &["C-001"]).status(api[0]),
            unit("U-002", &["U-001", "C-002"], &["C-002"]).status(api[1]),
            unit("U-003", &["U-002"], &[]).status(api[2]),
        ],
    );
}

// ------------------------------------------------------------ worked example

#[test]
fn the_worked_example_gives_hu_priorities_and_the_critical_path() {
    let repo = Repo::new();
    pipeline(&repo, ["Ready"; 3], ["Ready"; 3]);
    let g = repo.graph();
    assert!(g.problems().is_empty(), "{:?}", g.problems());
    assert_eq!(g.slugs(), ["api", "core"]);

    let r = g.resolve().unwrap();
    assert_eq!(
        names(&r.order()),
        [
            "core U-001",
            "api U-001",
            "core U-002",
            "api U-002",
            "api U-003",
            "core U-003"
        ]
    );
    let p = r.priorities(|_| false);
    let expect = [
        ("core", "U-001", 4),
        ("core", "U-002", 3),
        ("api", "U-001", 3),
        ("api", "U-002", 2),
        ("core", "U-003", 1),
        ("api", "U-003", 1),
    ];
    for (s, u, want) in expect {
        assert_eq!(p.of(&n(s, u)), Some(want), "{s} {u}");
    }
    let ranked: Vec<String> = p.ranked().iter().map(|(n, k)| format!("{n}={k}")).collect();
    assert_eq!(
        ranked,
        [
            "core U-001=4",
            "api U-001=3",
            "core U-002=3",
            "api U-002=2",
            "api U-003=1",
            "core U-003=1"
        ]
    );
    // ties between api U-001 and core U-002 break by slug
    assert_eq!(
        render_path(&p.critical_path().into_iter().cloned().collect::<Vec<_>>()),
        "core U-001 → api U-001 → api U-002 → api U-003"
    );

    // with core U-001 Verified, the chain behind it shortens
    let p = r.priorities(|x| *x == n("core", "U-001"));
    assert_eq!(p.of(&n("core", "U-001")), None);
    assert_eq!(p.of(&n("core", "U-002")), Some(3));
    assert_eq!(p.of(&n("api", "U-001")), Some(3));
    assert_eq!(
        names(&p.critical_path()),
        ["api U-001", "api U-002", "api U-003"]
    );

    // closures: api U-002 reaches through both pins
    assert_eq!(
        names(&r.closure(&n("api", "U-002"))),
        ["api U-001", "core U-001", "core U-002"]
    );
    assert!(r.depends_on(&n("api", "U-003"), &n("core", "U-001")));
    assert!(!r.depends_on(&n("core", "U-003"), &n("api", "U-001")));
    assert_eq!(
        names(&r.dependents_closure(&n("core", "U-002"))),
        ["api U-002", "api U-003", "core U-003"]
    );
}

#[test]
fn the_sdd_graph_carries_pins_and_unit_edges() {
    let repo = Repo::new();
    pipeline(&repo, ["Ready"; 3], ["Ready"; 3]);
    let g = repo.graph();
    let edges = g.sdd_edges();
    assert_eq!(edges.len(), 1);
    assert_eq!(
        (edges[0].from.as_str(), edges[0].to.as_str()),
        ("api", "core")
    );
    assert_eq!(edges[0].pins, ["C-001", "C-002"]);
    let units: Vec<String> = edges[0]
        .units
        .iter()
        .map(|e| format!("{} → {} via {:?}", e.from, e.to, e.via))
        .collect();
    assert_eq!(
        units,
        [
            "api U-001 → core U-001 via Some(\"C-001\")",
            "api U-002 → core U-002 via Some(\"C-002\")"
        ]
    );
    assert_eq!(g.upstream("api"), ["core"]);
    assert!(g.upstream("core").is_empty());

    let view = g.view();
    assert_eq!(view.sdds, ["api", "core"]);
    assert_eq!(view.unit_edges.len(), 6);
    assert_eq!(view.order.len(), 6);
    assert!(view.cycles.is_empty() && view.waiting.is_empty());
    let json = serde_json::to_string(&view.sdd_edges[0]).unwrap();
    assert!(
        json.starts_with(r#"{"from":"api","to":"core","pins":["C-001","C-002"],"units":[{"from":{"sdd":"api","unit":"U-001"},"to":{"sdd":"core","unit":"U-001"},"via":"C-001"}"#),
        "{json}"
    );
}

#[test]
fn mutual_pins_are_allowed_while_the_unit_graph_is_acyclic() {
    let repo = Repo::new();
    // a waits on b U-001; b U-002 waits on a U-001
    bundle(
        &repo,
        "a",
        "2.1",
        &[internal("C-001", "U-001"), pin("C-002", "b", "C-001")],
        &[unit("U-001", &["C-002"], &["C-002"])],
    );
    bundle(
        &repo,
        "b",
        "2.1",
        &[internal("C-001", "U-001"), pin("C-002", "a", "C-001")],
        &[
            unit("U-001", &[], &[]),
            unit("U-002", &["C-002"], &["C-002"]),
        ],
    );
    let g = repo.graph();
    assert!(g.problems().is_empty(), "{:?}", g.problems());
    let r = g.resolve().unwrap();
    assert_eq!(names(&r.order()), ["b U-001", "a U-001", "b U-002"]);
    let pairs: Vec<(String, String)> = g.sdd_edges().into_iter().map(|e| (e.from, e.to)).collect();
    assert_eq!(pairs, [("a".into(), "b".into()), ("b".into(), "a".into())]);
    assert_eq!(g.upstream("a"), ["b"]);
    assert_eq!(g.upstream("b"), ["a"]);
}

// --------------------------------------------------------- claimability

#[test]
fn a_pin_in_depends_on_waits_for_its_producers() {
    let now = UtcTime::parse("2026-11-01T12:00:00Z").unwrap();
    for (core_one, claimable) in [
        ("Ready", false),
        ("Claimed", false),
        ("Stale", false),
        ("Retired", false),
        ("Built", true),
        ("Verified", true),
    ] {
        let repo = Repo::new();
        pipeline(&repo, [core_one, "Ready", "Ready"], ["Ready"; 3]);
        let insts = repo.instances();
        let g = UnitGraph::build(&insts).unwrap();
        let api = insts.iter().find(|i| graph::slug_of(i) == "api").unwrap();
        let node = n("api", "U-001");
        let resolve = g.resolver(&node);
        let u = api.unit("U-001").unwrap();
        let view = model::unit_view_with(u, &api.units, now, &resolve);
        assert_eq!(view.claimable, claimable, "core U-001 {core_one}");
        assert_eq!(
            view.depends_on,
            [("C-001".to_string(), core_one.to_string())]
        );
        assert_eq!(view.through.len(), 1);
        assert_eq!(view.through[0].entry, "C-001");
        assert_eq!(
            view.through[0].units,
            [("core U-001".to_string(), core_one.to_string())]
        );
        assert_eq!(
            model::is_claimable_with(u, &api.units, now, &resolve),
            claimable
        );

        // without the other SDDs a pin entry is MISSING: never claimable
        let plain = model::unit_view(u, &api.units, now);
        assert!(!plain.claimable);
        assert_eq!(plain.depends_on[0].1, "MISSING");
        assert!(plain.through.is_empty());

        // the claim plan agrees, and names the producer it waits for
        let planned = claims::plan_with(api, Op::Claim, "U-001", "w1", 30, now, false, &resolve);
        match planned {
            Ok(_) => assert!(claimable),
            Err(e) => {
                assert!(!claimable);
                assert_eq!(e.exit_code(), 4);
                assert!(
                    e.to_string()
                        .contains(&format!("unmet deps: C-001 (core U-001={core_one})")),
                    "{e}"
                );
            }
        }
        assert!(claims::plan(api, Op::Claim, "U-001", "w1", 30, now, false).is_err());
    }
}

#[test]
fn a_pin_with_several_producers_waits_for_all_of_them() {
    let now = UtcTime::parse("2026-11-01T12:00:00Z").unwrap();
    let cases = [
        (["Built", "Ready"], false, "Ready"),
        (["Verified", "Built"], true, "Built"),
        (["Verified", "Verified"], true, "Verified"),
    ];
    for (statuses, claimable, summary) in cases {
        let repo = Repo::new();
        single(
            &repo,
            "core",
            "2.1",
            &[internal("C-001", "U-001, U-002")],
            &[
                unit("U-001", &[], &[]).status(statuses[0]),
                unit("U-002", &[], &[]).status(statuses[1]),
            ],
        );
        bundle(
            &repo,
            "api",
            "2.1",
            &[pin("C-001", "core", "C-001")],
            &[unit("U-001", &["C-001"], &["C-001"])],
        );
        let insts = repo.instances();
        let g = UnitGraph::build(&insts).unwrap();
        let node = n("api", "U-001");
        assert_eq!(
            names(&g.producers(&node, "C-001").unwrap()),
            ["core U-001", "core U-002"]
        );
        let api = insts.iter().find(|i| graph::slug_of(i) == "api").unwrap();
        let view = model::unit_view_with(
            api.unit("U-001").unwrap(),
            &api.units,
            now,
            &g.resolver(&node),
        );
        assert_eq!(view.claimable, claimable, "{statuses:?}");
        assert_eq!(view.depends_on[0].1, summary, "{statuses:?}");
    }
}

// ------------------------------------------------------- classification

#[test]
fn every_depends_on_entry_is_classified() {
    let repo = Repo::new();
    bundle(
        &repo,
        "core",
        "2.1",
        &[internal("C-001", "U-001")],
        &[unit("U-001", &[], &[])],
    );
    bundle(
        &repo,
        "api",
        "2.1",
        &[
            internal("C-001", "U-001"),
            pin("C-002", "core", "C-001"),
            C::Transcription("C-003"),
            pin("C-004", "core", "C-001"),
            internal("C-005", "—"),
        ],
        &[
            unit("U-001", &[], &[]),
            unit(
                "U-002",
                &[
                    "U-001", "C-002", "U-009", "C-001", "C-003", "C-004", "C-099", "C-005",
                ],
                &["C-002"],
            ),
        ],
    );
    let insts = repo.instances();
    let api = insts.iter().find(|i| graph::slug_of(i) == "api").unwrap();
    let classified = graph::classify(api).unwrap();
    assert_eq!(classified.len(), 2);
    assert!(classified[0].entries.is_empty());
    let got: Vec<(&str, &Entry)> = classified[1]
        .entries
        .iter()
        .map(|(d, e)| (d.as_str(), e))
        .collect();
    let core_pin = vitrify_core::contract::Pin {
        slug: "core".into(),
        contract: "C-001".into(),
        hash: Some("000000000000".into()),
    };
    assert_eq!(
        got,
        [
            ("U-001", &Entry::Unit),
            ("C-002", &Entry::Pin(core_pin)),
            ("U-009", &Entry::Invalid(EntryProblem::Unknown)),
            (
                "C-001",
                &Entry::Invalid(EntryProblem::Internal {
                    producers: vec!["U-001".into()]
                })
            ),
            ("C-003", &Entry::Invalid(EntryProblem::Transcription)),
            ("C-004", &Entry::Invalid(EntryProblem::NotConsumed)),
            ("C-099", &Entry::Invalid(EntryProblem::Unknown)),
            (
                "C-005",
                &Entry::Invalid(EntryProblem::Internal { producers: vec![] })
            ),
        ]
    );

    // the graph reports each invalid entry as an L1 problem, with a cause
    // and a remediation, and keeps the edges that resolve
    let g = UnitGraph::build(&insts).unwrap();
    let lines: Vec<String> = g
        .problems()
        .iter()
        .map(|p| {
            format!(
                "[{}] {} {} {}: {} → {}",
                p.check(),
                p.slug,
                p.unit,
                p.entry,
                p.cause(),
                p.remediation()
            )
        })
        .collect();
    assert_eq!(lines.len(), 6, "{lines:#?}");
    assert!(lines.iter().all(|l| l.starts_with("[L1] api U-002 ")));
    let has = |s: &str| lines.iter().any(|l| l.contains(s));
    assert!(has(
        "depends_on references nonexistent unit U-009 → remove U-009 from U-002's depends_on, or add unit U-009"
    ));
    assert!(has(
        "C-001, an internal contract of this SDD — name the unit instead → replace C-001 in U-002's depends_on with U-001"
    ));
    assert!(has("C-003, a transcription"));
    assert!(has(
        "pin C-004, which U-002 does not consume → add C-004 to U-002's consumes"
    ));
    assert!(has(
        "C-099, which is neither a unit nor a contract of this SDD"
    ));
    assert!(has(
        "replace C-005 in U-002's depends_on with the unit that produces it"
    ));
    assert_eq!(
        names(&g.dependencies(&n("api", "U-002"))),
        ["api U-001", "core U-001"]
    );
    assert_eq!(
        g.entries(&n("api", "U-002")).unwrap().len(),
        8,
        "the entries as listed"
    );
    // an invalid entry has no producers to wait for
    assert!(g.producers(&n("api", "U-002"), "C-004").is_none());
    assert!(g.producers(&n("api", "U-002"), "C-002").is_some());
}

#[test]
fn a_pin_in_depends_on_is_refused_under_schema_2_0() {
    let repo = Repo::new();
    bundle(
        &repo,
        "core",
        "2.0",
        &[internal("C-001", "U-001")],
        &[unit("U-001", &[], &[])],
    );
    bundle(
        &repo,
        "api",
        "2.0",
        &[pin("C-001", "core", "C-001")],
        &[
            unit("U-001", &["C-001"], &["C-001"]),
            unit("U-002", &[], &["C-001"]),
        ],
    );
    let insts = repo.instances();
    let g = UnitGraph::build(&insts).unwrap();
    assert_eq!(g.problems().len(), 1);
    let p = &g.problems()[0];
    assert_eq!(p.kind, ProblemKind::Entry(EntryProblem::Schema2_0));
    assert_eq!((p.check(), p.unit.as_str()), ("L1", "U-001"));
    assert!(p.remediation().contains("schema_version: 2.1"));
    // no edge crosses the SDDs
    assert!(g.sdd_edges().is_empty());

    // a run across SDDs names the 2.0 SDDs whose units consume pins
    let named: Vec<&str> = graph::pins_on_2_0(&insts)
        .unwrap()
        .into_iter()
        .map(graph::slug_of)
        .collect();
    assert_eq!(named, ["api"]);
}

#[test]
fn the_seeded_pin_fixture_is_2_0_with_pins_consumed() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/seeded/pin-multi/project");
    let insts: Vec<Instance> = model::discover(&root)
        .into_iter()
        .map(|h| Instance::load(h).unwrap())
        .collect();
    let named: Vec<&str> = graph::pins_on_2_0(&insts)
        .unwrap()
        .into_iter()
        .map(graph::slug_of)
        .collect();
    assert_eq!(named, ["beta", "gamma"]);
    // under 2.0 nothing orders the SDDs: the graph is theirs, unjoined
    let g = UnitGraph::build(&insts).unwrap();
    assert!(g.problems().is_empty(), "{:?}", g.problems());
    assert!(g.sdd_edges().is_empty());
    assert_eq!(g.slugs(), ["alpha", "beta", "gamma"]);
    assert!(g.resolve().is_ok());
}

#[test]
fn a_pin_whose_authority_names_no_units_is_an_l12_problem() {
    let repo = Repo::new();
    bundle(
        &repo,
        "core",
        "2.1",
        &[
            internal("C-001", "—"),
            internal("C-002", "U-001, U-007"),
            pin("C-003", "else", "C-001"),
        ],
        &[unit("U-001", &[], &[])],
    );
    bundle(
        &repo,
        "api",
        "2.1",
        &[
            pin("C-001", "core", "C-001"),
            pin("C-002", "core", "C-002"),
            pin("C-003", "nowhere", "C-001"),
            pin("C-004", "core", "C-009"),
            pin("C-005", "core", "C-003"),
        ],
        &[unit(
            "U-001",
            &["C-001", "C-002", "C-003", "C-004", "C-005"],
            &["C-001", "C-002", "C-003", "C-004", "C-005"],
        )],
    );
    let g = repo.graph();
    let got: Vec<(&str, PinProblem)> = g
        .problems()
        .iter()
        .map(|p| match &p.kind {
            ProblemKind::Pin { problem, .. } => (p.entry.as_str(), problem.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        got,
        [
            ("C-001", PinProblem::NoProducers),
            ("C-002", PinProblem::UnknownProducers(vec!["U-007".into()])),
            ("C-003", PinProblem::NoSdd),
            ("C-004", PinProblem::NoContract),
            ("C-005", PinProblem::NoProducers),
        ]
    );
    assert!(g.problems().iter().all(|p| p.check() == "L12"));
    let causes: Vec<String> = g.problems().iter().map(|p| p.cause()).collect();
    assert!(causes[0].contains("the Producer line of core C-001 names no unit"));
    assert!(causes[1].contains("names U-007, which is no unit of core"));
    assert!(causes[2].contains("no SDD in this repository has the slug `nowhere`"));
    assert!(causes[3].contains("core has no contract C-009"));
    // the producers that do exist still order the unit; none is claimable
    // through an entry that does not wholly resolve
    let u = n("api", "U-001");
    assert_eq!(names(&g.dependencies(&u)), ["core U-001"]);
    assert!(g.producers(&u, "C-002").is_none());
}

#[test]
fn slugs_held_by_several_sdds_are_left_out_and_named() {
    let repo = Repo::new();
    bundle(
        &repo,
        "core",
        "2.1",
        &[internal("C-001", "U-001")],
        &[unit("U-001", &[], &[])],
    );
    single(&repo, "core", "2.1", &[], &[unit("U-001", &[], &[])]);
    bundle(
        &repo,
        "api",
        "2.1",
        &[pin("C-001", "core", "C-001")],
        &[unit("U-001", &["C-001"], &["C-001"])],
    );
    let g = repo.graph();
    assert_eq!(g.clashes().len(), 1);
    assert_eq!(g.clashes()[0].slug, "core");
    assert_eq!(g.clashes()[0].files.len(), 2);
    assert_eq!(g.slugs(), ["api"]);
    assert_eq!(g.problems().len(), 1);
    assert!(matches!(
        g.problems()[0].kind,
        ProblemKind::Pin {
            problem: PinProblem::SharedSlug,
            ..
        }
    ));
}

// ------------------------------------------------------------- cycles

#[test]
fn every_cycle_is_reported_as_its_closing_path_and_waiters_apart() {
    let repo = Repo::new();
    // through pins: core U-004 → api U-002 → core U-006 → core U-004,
    // reported from its smallest unit, api U-002
    // locally: core U-001 ⇄ core U-002; a self-loop: api U-005
    // waiting: core U-003 behind U-002; api U-003 behind api U-002
    // free: api U-001, core U-005
    bundle(
        &repo,
        "core",
        "2.1",
        &[internal("C-001", "U-006"), pin("C-002", "api", "C-001")],
        &[
            unit("U-001", &["U-002"], &[]),
            unit("U-002", &["U-001"], &[]),
            unit("U-003", &["U-002"], &[]),
            unit("U-004", &["C-002"], &["C-002"]),
            unit("U-005", &[], &[]),
            unit("U-006", &["U-004"], &[]),
        ],
    );
    single(
        &repo,
        "api",
        "2.1",
        &[internal("C-001", "U-002"), pin("C-002", "core", "C-001")],
        &[
            unit("U-001", &[], &[]),
            unit("U-002", &["C-002"], &["C-002"]),
            unit("U-003", &["U-002", "U-001"], &[]),
            unit("U-005", &["U-005"], &[]),
        ],
    );
    let g = repo.graph();
    assert!(g.problems().is_empty(), "{:?}", g.problems());
    let c = g.resolve().unwrap_err();
    let paths: Vec<String> = c.cycles.iter().map(|p| render_path(p)).collect();
    assert_eq!(
        paths,
        [
            "api U-002 → core U-006 → core U-004 → api U-002",
            "api U-005 → api U-005",
            "core U-001 → core U-002 → core U-001",
        ]
    );
    let waiting: Vec<String> = c.waiting.iter().map(Node::to_string).collect();
    assert_eq!(waiting, ["api U-003", "core U-003"]);
    let view = g.view();
    assert_eq!(view.cycles.len(), 3);
    assert_eq!(view.waiting.len(), 2);
    assert!(view.order.is_empty());
}

// ------------------------------------------------------------ random graphs

/// A random repository: `sdds` SDDs of up to `per` units; edges kept when
/// `keep(from, to)` (by global index) says so. A cross-SDD edge becomes a
/// contract its target produces and a pin of it its source depends on.
/// Returns the nodes (sorted) and the edges as node pairs.
fn random_repo(
    repo: &Repo,
    rng: &mut Rng,
    sdds: usize,
    per: usize,
    density: u64,
    keep: &dyn Fn(usize, usize) -> bool,
) -> (Vec<Node>, BTreeSet<(Node, Node)>) {
    let slugs: Vec<String> = (0..sdds).map(|i| format!("s{i}")).collect();
    let mut nodes: Vec<Node> = Vec::new();
    for s in &slugs {
        for u in 1..=1 + rng.below(per) {
            nodes.push(n(s, &format!("U-{u:03}")));
        }
    }
    // a random global position per node, so edges run across SDDs both ways
    let mut pos: Vec<usize> = (0..nodes.len()).collect();
    for i in (1..pos.len()).rev() {
        let j = rng.below(i + 1);
        pos.swap(i, j);
    }
    let mut edges: BTreeSet<(Node, Node)> = BTreeSet::new();
    for a in 0..nodes.len() {
        for b in 0..nodes.len() {
            if a != b && keep(pos[a], pos[b]) && rng.chance(density) {
                edges.insert((nodes[a].clone(), nodes[b].clone()));
            }
        }
    }
    // occasionally a self-loop, when `keep` allows one
    for a in 0..nodes.len() {
        if keep(pos[a], pos[a]) && rng.chance(5) {
            edges.insert((nodes[a].clone(), nodes[a].clone()));
        }
    }

    // contracts: one authority per cross edge's target, one pin per edge
    for (si, s) in slugs.iter().enumerate() {
        let mut contracts: Vec<(String, String)> = Vec::new(); // (id, body)
        let mut units: Vec<U> = Vec::new();
        let mine: Vec<&Node> = nodes.iter().filter(|x| &x.slug == s).collect();
        // authorities: C-1## for each of my units, produced by it
        for x in &mine {
            let num = &x.unit[2..];
            contracts.push((format!("C-1{num}"), internal("C-000", &x.unit).body()));
        }
        let mut next_pin = 0;
        for x in &mine {
            let mut u = unit(&x.unit, &[], &[]);
            for (_, to) in edges.iter().filter(|(f, _)| f == *x) {
                if to.slug == *s {
                    u.depends_on.push(to.unit.clone());
                } else {
                    next_pin += 1;
                    let id = format!("C-5{next_pin:02}");
                    contracts.push((
                        id.clone(),
                        pin("C-000", &to.slug, &format!("C-1{}", &to.unit[2..])).body(),
                    ));
                    u.consumes.push(id.clone());
                    u.depends_on.push(id);
                }
            }
            units.push(u);
        }
        let shape_single = si % 2 == 1;
        if shape_single {
            let mut text = front(s, "2.1");
            text.push_str("\n## Interface Contracts\n\n");
            for (id, body) in &contracts {
                text.push_str(&format!(
                    "### {id} — contract\n\n{}\n",
                    body.replace("## Surface", "#### Surface")
                ));
            }
            text.push_str("## Work Units\n\n");
            for u in &units {
                text.push_str(&format!(
                    "### {} — unit\n\n```yaml\n{}```\n\n",
                    u.id,
                    u.block()
                ));
            }
            repo.write(&format!("specs/{s}.sdd.md"), &text);
        } else {
            repo.write(&format!("specs/{s}/sdd.md"), &front(s, "2.1"));
            for (id, body) in &contracts {
                repo.write(
                    &format!("specs/{s}/contracts/{id}-c.md"),
                    &format!("# {id} — contract\n\n{body}"),
                );
            }
            for u in &units {
                repo.write(
                    &format!("specs/{s}/units/{}-u.md", u.id),
                    &format!("---\n{}---\n\n# {}\n", u.block(), u.id),
                );
            }
        }
    }
    nodes.sort();
    (nodes, edges)
}

fn reach(edges: &BTreeSet<(Node, Node)>, from: &Node) -> BTreeSet<Node> {
    let mut seen: BTreeSet<Node> = BTreeSet::new();
    let mut stack = vec![from.clone()];
    while let Some(x) = stack.pop() {
        for (_, to) in edges.iter().filter(|(f, _)| *f == x) {
            if seen.insert(to.clone()) {
                stack.push(to.clone());
            }
        }
    }
    seen
}

#[test]
fn random_dags_order_closures_and_priorities_match_brute_force() {
    let mut across = 0;
    for seed in 1..=24u64 {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let repo = Repo::new();
        // a dependency only on a node earlier in the hidden order: a DAG
        let sdds = 2 + rng.below(3);
        let (nodes, edges) = random_repo(&repo, &mut rng, sdds, 6, 18, &|a, b| a > b);
        let g = repo.graph();
        assert!(g.problems().is_empty(), "seed {seed}: {:?}", g.problems());
        assert_eq!(g.nodes(), nodes.as_slice(), "seed {seed}");
        let got: BTreeSet<(Node, Node)> = g
            .edges()
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        assert_eq!(got, edges, "seed {seed}: edges");
        across += g.edges().iter().filter(|e| e.via.is_some()).count();

        let r = g.resolve().unwrap();
        let order: Vec<Node> = r.order().into_iter().cloned().collect();
        assert_eq!(order.len(), nodes.len());
        let at = |x: &Node| order.iter().position(|y| y == x).unwrap();
        for (from, to) in &edges {
            assert!(at(to) < at(from), "seed {seed}: {to} before {from}");
        }
        // the same as a reference Kahn taking the smallest ready node
        let mut done: Vec<Node> = Vec::new();
        while done.len() < nodes.len() {
            let next = nodes
                .iter()
                .filter(|x| !done.contains(x))
                .find(|x| {
                    edges
                        .iter()
                        .filter(|(f, _)| f == *x)
                        .all(|(_, t)| done.contains(t))
                })
                .unwrap()
                .clone();
            done.push(next);
        }
        assert_eq!(order, done, "seed {seed}: order");
        // and identical on a second build
        let again: Vec<Node> = repo
            .graph()
            .resolve()
            .unwrap()
            .order()
            .into_iter()
            .cloned()
            .collect();
        assert_eq!(order, again, "seed {seed}: determinism");

        for x in &nodes {
            let want: Vec<Node> = reach(&edges, x).into_iter().collect();
            let have: Vec<Node> = r.closure(x).into_iter().cloned().collect();
            assert_eq!(have, want, "seed {seed}: closure of {x}");
        }

        // priorities: the longest chain of unfinished units through
        // unfinished dependents, counted in units
        let finished: BTreeSet<Node> = nodes.iter().filter(|_| rng.chance(30)).cloned().collect();
        let p = r.priorities(|x| finished.contains(x));
        fn chain(x: &Node, edges: &BTreeSet<(Node, Node)>, finished: &BTreeSet<Node>) -> usize {
            1 + edges
                .iter()
                .filter(|(f, t)| t == x && !finished.contains(f))
                .map(|(f, _)| chain(f, edges, finished))
                .max()
                .unwrap_or(0)
        }
        for x in &nodes {
            let want = (!finished.contains(x)).then(|| chain(x, &edges, &finished));
            assert_eq!(p.of(x), want, "seed {seed}: priority of {x}");
        }
        // ranked: priority desc, then slug, then unit
        let ranked = p.ranked();
        for w in ranked.windows(2) {
            assert!(
                w[0].1 > w[1].1 || (w[0].1 == w[1].1 && w[0].0 < w[1].0),
                "seed {seed}: rank"
            );
        }
        // the critical path is a chain of dependents as long as the top
        let path = p.critical_path();
        assert_eq!(path.len(), ranked.first().map_or(0, |r| r.1), "seed {seed}");
        for w in path.windows(2) {
            assert!(edges.contains(&(w[1].clone(), w[0].clone())), "seed {seed}");
        }
    }
    assert!(across >= 24, "too few edges through pins: {across}");
}

#[test]
fn weighted_priorities_match_the_brute_force_longest_weighted_chain() {
    for seed in 1..=24u64 {
        let mut rng = Rng(0x51ed_270b_a3c1_9d47 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let repo = Repo::new();
        let sdds = 2 + rng.below(3);
        let (nodes, edges) = random_repo(&repo, &mut rng, sdds, 6, 18, &|a, b| a > b);
        let r = repo.graph().resolve().unwrap();
        let finished: BTreeSet<Node> = nodes.iter().filter(|_| rng.chance(30)).cloned().collect();
        let weights: BTreeMap<Node, u64> = nodes
            .iter()
            .map(|x| (x.clone(), 1 + rng.below(9) as u64))
            .collect();
        let p = r.priorities_weighted(|x| finished.contains(x), |x| weights[x]);
        fn chain(
            x: &Node,
            edges: &BTreeSet<(Node, Node)>,
            finished: &BTreeSet<Node>,
            w: &BTreeMap<Node, u64>,
        ) -> u64 {
            w[x] + edges
                .iter()
                .filter(|(f, t)| t == x && !finished.contains(f))
                .map(|(f, _)| chain(f, edges, finished, w))
                .max()
                .unwrap_or(0)
        }
        for x in &nodes {
            let want = (!finished.contains(x)).then(|| chain(x, &edges, &finished, &weights));
            assert_eq!(p.of_weighted(x), want, "seed {seed}: priority of {x}");
        }
        // ranked: weighted priority desc, then slug, then unit
        let ranked = p.ranked_weighted();
        for w in ranked.windows(2) {
            assert!(
                w[0].1 > w[1].1 || (w[0].1 == w[1].1 && w[0].0 < w[1].0),
                "seed {seed}: rank"
            );
            assert_eq!(p.cmp(w[0].0, w[1].0), Ordering::Less, "seed {seed}");
        }
        // the critical path follows the weighted labels: it starts at the
        // top, and its weights sum to the top priority
        let path = p.critical_path();
        let top = ranked.first().map_or(0, |r| r.1);
        assert_eq!(
            path.iter().map(|x| weights[*x]).sum::<u64>(),
            top,
            "seed {seed}"
        );
        for w in path.windows(2) {
            assert!(edges.contains(&(w[1].clone(), w[0].clone())), "seed {seed}");
        }

        // all weights 1 is the unweighted answer, exactly
        let unit = r.priorities_weighted(|x| finished.contains(x), |_| 1);
        let plain = r.priorities(|x| finished.contains(x));
        assert_eq!(unit.ranked(), plain.ranked(), "seed {seed}");
        assert_eq!(unit.critical_path(), plain.critical_path(), "seed {seed}");
        // a weight of 0 is taken as 1
        let zero = r.priorities_weighted(|x| finished.contains(x), |_| 0);
        assert_eq!(zero.ranked(), plain.ranked(), "seed {seed}");
    }
}

#[test]
fn a_long_job_outranks_a_longer_chain_of_short_ones_and_ties_keep_slug_order() {
    let repo = Repo::new();
    pipeline(&repo, ["Ready"; 3], ["Ready"; 3]);
    let r = repo.graph().resolve().unwrap();
    // by count the critical path runs through api; one heavy leaf on the
    // core side pulls it there
    let heavy = n("core", "U-003");
    let p = r.priorities_weighted(|_| false, |x| if *x == heavy { 10 } else { 1 });
    assert_eq!(p.of_weighted(&heavy), Some(10));
    assert_eq!(p.of_weighted(&n("core", "U-001")), Some(12));
    assert_eq!(
        p.ranked_weighted().first().map(|r| r.0),
        Some(&n("core", "U-001"))
    );
    assert_eq!(
        names(&p.critical_path()),
        ["core U-001", "core U-002", "core U-003"]
    );
    // equal priorities keep (slug, unit id) order
    let p = r.priorities_weighted(|_| false, |_| 5);
    assert_eq!(p.of_weighted(&n("api", "U-003")), Some(5));
    assert_eq!(
        p.cmp(&n("api", "U-003"), &n("core", "U-003")),
        Ordering::Less
    );
}

#[test]
fn random_cyclic_graphs_report_every_cycle_and_every_waiter() {
    let (mut cyclic, mut several, mut through_pins) = (0, 0, 0);
    for seed in 1..=24u64 {
        let mut rng = Rng(0xd1b5_4a32_d192_ed03 ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let repo = Repo::new();
        let sdds = 2 + rng.below(3);
        let (nodes, edges) = random_repo(&repo, &mut rng, sdds, 5, 9, &|_, _| true);
        let g = repo.graph();
        assert!(g.problems().is_empty(), "seed {seed}: {:?}", g.problems());

        // brute force: strongly connected by mutual reachability
        let reaches: Vec<BTreeSet<Node>> = nodes.iter().map(|x| reach(&edges, x)).collect();
        let on_cycle: Vec<bool> = (0..nodes.len())
            .map(|i| reaches[i].contains(&nodes[i]))
            .collect();
        let cycle_nodes: BTreeSet<Node> = (0..nodes.len())
            .filter(|&i| on_cycle[i])
            .map(|i| nodes[i].clone())
            .collect();
        let waiting: Vec<Node> = (0..nodes.len())
            .filter(|&i| !on_cycle[i] && reaches[i].iter().any(|y| cycle_nodes.contains(y)))
            .map(|i| nodes[i].clone())
            .collect();
        // one component per class of mutually reachable cycle nodes
        let mut components: Vec<BTreeSet<Node>> = Vec::new();
        for (i, x) in nodes.iter().enumerate() {
            if !on_cycle[i] || components.iter().any(|c| c.contains(x)) {
                continue;
            }
            let comp: BTreeSet<Node> = cycle_nodes
                .iter()
                .filter(|y| *y == x || (reaches[i].contains(*y) && reach(&edges, y).contains(x)))
                .cloned()
                .collect();
            components.push(comp);
        }

        match g.resolve() {
            Ok(_) => assert!(cycle_nodes.is_empty(), "seed {seed}"),
            Err(c) => {
                cyclic += 1;
                several += usize::from(c.cycles.len() > 1);
                through_pins += c
                    .cycles
                    .iter()
                    .filter(|p| p.iter().any(|x| x.slug != p[0].slug))
                    .count();
                assert_eq!(c.waiting, waiting, "seed {seed}: waiting");
                assert_eq!(c.cycles.len(), components.len(), "seed {seed}: cycles");
                for (path, comp) in c.cycles.iter().zip(&components) {
                    // starts and ends at the component's smallest unit
                    assert_eq!(path.first(), comp.first(), "seed {seed}");
                    assert_eq!(path.last(), comp.first(), "seed {seed}");
                    assert!(path.len() >= 2);
                    for w in path.windows(2) {
                        assert!(comp.contains(&w[1]), "seed {seed}");
                        assert!(
                            edges.contains(&(w[0].clone(), w[1].clone())),
                            "seed {seed}: {} → {} is no edge",
                            w[0],
                            w[1]
                        );
                    }
                    // a simple cycle: no unit twice before it closes
                    let inner: BTreeSet<&Node> = path[..path.len() - 1].iter().collect();
                    assert_eq!(inner.len(), path.len() - 1, "seed {seed}");
                }
            }
        }
    }
    assert!(cyclic >= 12, "too few cyclic cases: {cyclic}");
    assert!(several >= 3, "too few cases with several cycles: {several}");
    assert!(
        through_pins >= 3,
        "too few cycles through pins: {through_pins}"
    );
}
