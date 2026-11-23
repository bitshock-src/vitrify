//! The re-check cascade: which Verified units a code change since their
//! last recorded check can reach. Pure — git, and the dependency closures,
//! are the caller's: this module takes journal events, the changed paths per
//! anchor, the closures, and each SDD's declared ownership, and says which
//! units are dirty and why.
//!
//! Two cascades answer two questions after a change. A unit is
//! **regenerated** (a model call) when its recorded trace stops matching its
//! reading set — its own entry, the contracts it consumes, the shared
//! regions; that is the trace mechanism ([`crate::trace`]), and it crosses
//! SDDs through pins. A Verified unit is **re-checked** (its Verification
//! rows run again, no model call) when a path it is *checked against*
//! changed since its **anchor**. This module is the second.
//!
//! # Anchors
//!
//! A unit's anchor is the commit of its latest `verified` journal event, from
//! any actor: its integration, or a later re-check (a `verified` event with
//! actor [`journal::RECHECK_ACTOR`]). The **suite anchor** is the commit of
//! the latest `audit` event carrying `suite: "pass"`. See [`Anchors`]. The
//! caller runs `git diff --name-only <anchor> <head>` once per distinct
//! anchor, drops protocol-state-only paths, and hands in the lists ([`Changes`]).
//!
//! # What a unit is checked against
//!
//! * the files owned by the unit and by every unit in its dependency closure
//!   (transitive `depends_on`, across SDDs through pins — computed by the
//!   caller, [`Closures`]) — except test paths of another SDD: a test path
//!   (a path a pattern of an SDD's §6 `tests:` list matches, schema 2.2)
//!   reaches units of its own SDD only. A **generated** test, one a unit
//!   owns, reaches its owner and the units of the owner's SDD whose closure
//!   contains the owner;
//! * its SDD's `unmanaged:` files, its SDD's **pre-existing** tests (test
//!   paths no unit owns: they reach every unit of the SDD), and any path
//!   inside the SDD's `scope_roots` that no unit owns, that is not a test
//!   path and that is not listed unmanaged (the conservative reading: nobody
//!   vouches for it, so it counts as the SDD's unmanaged input);
//! * paths outside every SDD's `scope_roots` — or, when the unit's SDD
//!   declares `shared_inputs` in its front matter, only the outside paths
//!   those patterns match.
//!
//! Spec files — anything under an SDD's own root (a bundle's directory, a
//! single-file SDD's file) — are the regeneration cascade's inputs and never
//! dirty a re-check. Early cutoff falls out of the input: a unit regenerated
//! into byte-identical files changes no path, so nothing downstream is
//! dirty; a commit that changed only protocol state lists no path at all.
//!
//! # Ownership
//!
//! [`Ownership`] is built once from the repository's SDDs and classifies a
//! repository-relative path as [`Owner::Spec`], [`Owner::Unit`],
//! [`Owner::Test`], [`Owner::Unmanaged`] or [`Owner::Outside`]. Patterns
//! match as `owns` does ([`audit::matches`]): an exact path, a shell-style
//! glob, or a directory prefix (a trailing `/` is allowed). Exclusive ownership makes overlaps a
//! lint error elsewhere; here they resolve deterministically, in this order:
//!
//! 1. an SDD's own files ([`Owner::Spec`]) — whatever a glob matches, as the
//!    write-set law has it. Nested SDDs: the longest matching spec path
//!    wins, then the smaller slug;
//! 2. a unit's `owns`, SDDs by slug and units by id: the first match wins —
//!    a generated test ([`Owner::Test`] with the unit) when the unit's own
//!    SDD lists the path under `tests:`, the unit's file ([`Owner::Unit`])
//!    otherwise;
//! 3. an SDD's `tests:`, by slug: the first match is that SDD's
//!    pre-existing test ([`Owner::Test`] with no unit). Overlapping lists of
//!    two SDDs are not linted; the smaller slug wins;
//! 4. an SDD's `unmanaged:`, by slug: the first match wins;
//! 5. an SDD's `scope_roots`, by slug: the first match is that SDD's
//!    unmanaged input ([`Owner::Unmanaged`]);
//! 6. [`Owner::Outside`].
//!
//! # Dirty
//!
//! [`dirty_set`] maps each changed path to its owner once, then marks a
//! Verified unit dirty when an owner is the unit itself or in its closure, or
//! is [`Owner::Unmanaged`] of the unit's own SDD, or is a generated test of
//! the unit's own SDD whose owner is the unit or in its closure, or is a
//! pre-existing test of the unit's own SDD, or is [`Owner::Outside`]
//! (and, if the SDD declares `shared_inputs`, matched by one of its
//! patterns; a declared empty list excludes every outside path). A Verified
//! unit with **no anchor** — no `verified` event, or one without a commit,
//! for instance a hand-edited status — is dirty: nothing recorded vouches
//! for it. So is a unit whose anchor has no diff in [`Changes`] (a commit the
//! caller could not diff): the cascade never trusts what it cannot compare.
//! The output is sorted by node, each dirty unit with the [`Why`] that made
//! it so: the changed paths and their owners (what `vitrify check` lists and
//! `vitrify run --dry-run` prints). [`dirty_everything`] is the
//! `vitrify run --recheck` escape hatch: every Verified unit dirty, every
//! recorded check distrusted. [`suite_dirty`] asks the same of an SDD's §5
//! Test command against the suite anchor.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::CoreError;
use crate::audit;
use crate::graph::Node;
use crate::journal::{self, Event};
use crate::model::{self, Instance};
use crate::writeset;

// ----------------------------------------------------------------- anchors

/// The commits recorded checks are anchored at, read from one SDD's journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchors {
    /// Unit id → the commit of its latest `verified` event. A unit whose
    /// latest `verified` event carries no commit has no entry: the record
    /// does not say where the check ran.
    pub units: BTreeMap<String, String>,
    /// The commit of the latest `audit` event with `suite: "pass"`; `None`
    /// when there is none, or it carries no commit.
    pub suite: Option<String>,
}

impl Anchors {
    /// Read the anchors from `events`, in journal order — later events win.
    pub fn from_events(events: &[Event]) -> Anchors {
        let mut out = Anchors::default();
        for e in events {
            if e.event == "verified"
                && let Some(unit) = &e.unit
            {
                match &e.commit {
                    Some(c) => {
                        out.units.insert(unit.clone(), c.clone());
                    }
                    None => {
                        out.units.remove(unit);
                    }
                }
            } else if e.is_suite_pass() {
                out.suite = e.commit.clone();
            }
        }
        out
    }

    /// The anchor of `unit`, when its journal records one.
    pub fn unit(&self, unit: &str) -> Option<&str> {
        self.units.get(unit).map(String::as_str)
    }
}

// --------------------------------------------------------------- ownership

/// Who a repository-relative path belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// A file of the SDD itself, by slug: never a re-check input.
    Spec(String),
    /// A file a unit owns.
    Unit(Node),
    /// A test path of the SDD `slug`: generated, with `unit` its owner, or
    /// pre-existing, with `unit` `None`.
    Test { slug: String, unit: Option<Node> },
    /// A file the SDD, by slug, lists `unmanaged:` — or lies under its
    /// `scope_roots` and nobody owns.
    Unmanaged(String),
    /// A path under no SDD's `scope_roots`.
    Outside,
}

impl Owner {
    /// `spec of <slug>`, `unit <slug> <unit>`, `test of unit <slug> <unit>`,
    /// `pre-existing test of <slug>`, `unmanaged in <slug>` or `outside every
    /// scope`, as a report names an owner.
    pub fn describe(&self) -> String {
        match self {
            Owner::Spec(slug) => format!("spec of {slug}"),
            Owner::Unit(node) => format!("unit {node}"),
            Owner::Test {
                unit: Some(node), ..
            } => format!("test of unit {node}"),
            Owner::Test { slug, unit: None } => format!("pre-existing test of {slug}"),
            Owner::Unmanaged(slug) => format!("unmanaged in {slug}"),
            Owner::Outside => "outside every scope".to_string(),
        }
    }
}

/// What one SDD declares about the paths of the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SddOwnership {
    pub slug: String,
    /// The SDD's own files and directories ([`writeset::spec_paths`]).
    pub spec_paths: Vec<String>,
    pub scope_roots: Vec<String>,
    /// The `unmanaged:` globs of its System Layout & Ownership section.
    pub unmanaged: Vec<String>,
    /// The `tests:` globs of the same section: read at schema 2.2 or later,
    /// empty below.
    pub tests: Vec<String>,
    /// Each unit's id and its `owns` globs.
    pub units: Vec<(String, Vec<String>)>,
    /// `shared_inputs` from front matter: `None` when the SDD declares none
    /// (every outside path is an input), `Some` — possibly empty — when it
    /// does (only the outside paths those patterns match are).
    pub shared_inputs: Option<Vec<String>>,
}

impl SddOwnership {
    /// Read `inst`'s declarations (`root` is the repository root `inst` was
    /// loaded under). Reads the SDD's core file for its `unmanaged:` list,
    /// and its `tests:` list when the SDD declares schema 2.2 or later.
    pub fn of(slug: &str, inst: &Instance, root: &Path) -> Result<SddOwnership, CoreError> {
        let core_text = model::read_text(&inst.file)?;
        Ok(SddOwnership {
            slug: slug.to_string(),
            spec_paths: writeset::spec_paths(inst, root),
            scope_roots: inst.meta.get_list("scope_roots"),
            unmanaged: audit::unmanaged_globs(&core_text),
            tests: if inst.schema().tests_list() {
                audit::tests_globs(&core_text)
            } else {
                Vec::new()
            },
            units: inst
                .units
                .iter()
                .map(|u| {
                    let owns = u
                        .meta
                        .get_list("owns")
                        .iter()
                        .map(|g| g.trim_end().to_string())
                        .collect();
                    (u.unit.clone(), owns)
                })
                .collect(),
            shared_inputs: inst
                .meta
                .has("shared_inputs")
                .then(|| inst.meta.get_list("shared_inputs")),
        })
    }
}

/// Every SDD's declared ownership, classifying paths ([`Ownership::classify`]).
#[derive(Debug, Clone, Default)]
pub struct Ownership {
    /// Sorted by slug, each one's units by id.
    sdds: Vec<SddOwnership>,
}

/// Whether `pattern` — an exact path, a glob, or a directory prefix — covers
/// `path`: the matcher `owns` uses.
fn covers(path: &str, pattern: &str) -> bool {
    audit::matches(path, pattern)
}

/// A scope root additionally may name the repository root itself.
fn in_scope(path: &str, scope_root: &str) -> bool {
    let g = scope_root.trim().trim_start_matches("./");
    g.is_empty() || g == "." || covers(path, g)
}

impl Ownership {
    /// Build from the repository's SDDs, each with its slug; `root` is the
    /// repository root the instances were loaded under.
    pub fn build<'a, I>(root: &Path, sdds: I) -> Result<Ownership, CoreError>
    where
        I: IntoIterator<Item = (&'a str, &'a Instance)>,
    {
        let parts = sdds
            .into_iter()
            .map(|(slug, inst)| SddOwnership::of(slug, inst, root))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Ownership::new(parts))
    }

    /// Build from declarations already read.
    pub fn new(mut sdds: Vec<SddOwnership>) -> Ownership {
        sdds.sort_by(|a, b| a.slug.cmp(&b.slug));
        for s in &mut sdds {
            s.units.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Ownership { sdds }
    }

    /// The `shared_inputs` patterns of `slug`'s SDD; `None` when it declares
    /// none, or is unknown.
    pub fn shared_inputs(&self, slug: &str) -> Option<&[String]> {
        self.sdds
            .iter()
            .find(|s| s.slug == slug)
            .and_then(|s| s.shared_inputs.as_deref())
    }

    /// Whether `slug`'s SDD takes the outside `path` as an input: it does
    /// when it declares no `shared_inputs`, or a pattern matches.
    pub fn takes_outside(&self, slug: &str, path: &str) -> bool {
        match self.shared_inputs(slug) {
            None => true,
            Some(patterns) => patterns.iter().any(|p| covers(path, p)),
        }
    }

    /// Classify a repository-relative path (`/`-separated), by the order in
    /// the module doc.
    pub fn classify(&self, path: &str) -> Owner {
        let spec_len = |s: &SddOwnership| {
            s.spec_paths
                .iter()
                .filter(|p| path == p.as_str() || path.starts_with(&format!("{p}/")))
                .map(|p| p.len())
                .max()
        };
        // longest spec path first; the scan order breaks ties by slug
        let mut spec: Option<(usize, &str)> = None;
        for s in &self.sdds {
            if let Some(len) = spec_len(s)
                && spec.is_none_or(|(best, _)| len > best)
            {
                spec = Some((len, &s.slug));
            }
        }
        if let Some((_, slug)) = spec {
            return Owner::Spec(slug.to_string());
        }
        for s in &self.sdds {
            for (unit, owns) in &s.units {
                if owns.iter().any(|g| covers(path, g)) {
                    let node = Node::new(&s.slug, unit);
                    if s.tests.iter().any(|g| covers(path, g)) {
                        return Owner::Test {
                            slug: s.slug.clone(),
                            unit: Some(node),
                        };
                    }
                    return Owner::Unit(node);
                }
            }
        }
        for s in &self.sdds {
            if s.tests.iter().any(|g| covers(path, g)) {
                return Owner::Test {
                    slug: s.slug.clone(),
                    unit: None,
                };
            }
        }
        for s in &self.sdds {
            if s.unmanaged.iter().any(|g| covers(path, g)) {
                return Owner::Unmanaged(s.slug.clone());
            }
        }
        for s in &self.sdds {
            if s.scope_roots.iter().any(|r| in_scope(path, r)) {
                return Owner::Unmanaged(s.slug.clone());
            }
        }
        Owner::Outside
    }
}

// ------------------------------------------------------------------- dirty

/// The changed paths per distinct anchor: `git diff --name-only <anchor>
/// <head>` minus protocol-state-only changes, computed once per anchor by
/// the caller (units integrated in one commit share one). Repository-relative,
/// `/`-separated.
pub type Changes = BTreeMap<String, Vec<String>>;

/// Each unit's transitive dependencies, across SDDs, excluding the unit
/// itself; a unit with no entry depends on nothing.
pub type Closures = BTreeMap<Node, BTreeSet<Node>>;

/// A Verified unit and where its last recorded check was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub node: Node,
    /// The commit of its latest `verified` event ([`Anchors::unit`]).
    pub anchor: Option<String>,
}

/// One changed path and the owner that made it an input.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reason {
    pub path: String,
    pub owner: Owner,
}

impl Reason {
    /// `<path> (<owner>)`.
    pub fn describe(&self) -> String {
        format!("{} ({})", self.path, self.owner.describe())
    }
}

/// Why a unit — or a suite — is dirty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Every recorded check is distrusted (`--recheck`).
    Everything,
    /// No anchor: nothing recorded vouches for it.
    NoAnchor,
    /// The anchor is a commit the caller supplied no diff for.
    NoDiff(String),
    /// Paths it is checked against changed since its anchor, sorted.
    Paths(Vec<Reason>),
}

impl Why {
    /// One human line: `every recorded check is distrusted`, `no recorded
    /// check`, `no diff for anchor <c>`, or the changed paths with owners.
    pub fn describe(&self) -> String {
        match self {
            Why::Everything => "every recorded check is distrusted".to_string(),
            Why::NoAnchor => "no recorded check".to_string(),
            Why::NoDiff(anchor) => format!("no diff for anchor {anchor}"),
            Why::Paths(reasons) => reasons
                .iter()
                .map(Reason::describe)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// A dirty unit with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirty {
    pub node: Node,
    pub why: Why,
}

/// The reasons `path_owners` (sorted by path) make the node set `relevant`
/// of the SDD `slug` dirty.
fn reasons(
    path_owners: &[(&str, &Owner)],
    slug: &str,
    relevant: &BTreeSet<&Node>,
    ownership: &Ownership,
) -> Vec<Reason> {
    path_owners
        .iter()
        .filter(|(path, owner)| match owner {
            Owner::Spec(_) => false,
            Owner::Unit(n) => relevant.contains(n),
            Owner::Test {
                slug: s,
                unit: Some(n),
            } => s == slug && relevant.contains(n),
            Owner::Test {
                slug: s,
                unit: None,
            } => s == slug,
            Owner::Unmanaged(s) => s == slug,
            Owner::Outside => ownership.takes_outside(slug, path),
        })
        .map(|(path, owner)| Reason {
            path: path.to_string(),
            owner: (*owner).clone(),
        })
        .collect()
}

/// Each changed path of `lists`, once, with its owner; sorted by path.
fn owners_of<'a>(
    lists: impl IntoIterator<Item = &'a Vec<String>>,
    ownership: &Ownership,
) -> BTreeMap<&'a str, Owner> {
    let mut out = BTreeMap::new();
    for path in lists.into_iter().flatten() {
        out.entry(path.as_str())
            .or_insert_with(|| ownership.classify(path));
    }
    out
}

/// The Verified `candidates` a change reaches: those with a path they are
/// checked against changed since their anchor, and those whose anchor
/// vouches for nothing ([`Why`]). Sorted by node; each node appears once.
/// Candidates with a clean cascade are not in the output.
pub fn dirty_set(
    candidates: &[Candidate],
    changes: &Changes,
    closures: &Closures,
    ownership: &Ownership,
) -> Vec<Dirty> {
    // every changed path is classified once, however many anchors list it
    let wanted: BTreeSet<&String> = candidates
        .iter()
        .filter_map(|c| c.anchor.as_ref())
        .collect();
    let owners = owners_of(
        changes
            .iter()
            .filter(|(anchor, _)| wanted.contains(anchor))
            .map(|(_, paths)| paths),
        ownership,
    );
    let mut ordered: Vec<&Candidate> = candidates.iter().collect();
    ordered.sort_by(|a, b| a.node.cmp(&b.node));
    ordered.dedup_by(|a, b| a.node == b.node);
    let mut out = Vec::new();
    for c in ordered {
        let why = match &c.anchor {
            None => Some(Why::NoAnchor),
            Some(anchor) => match changes.get(anchor) {
                None => Some(Why::NoDiff(anchor.clone())),
                Some(paths) => {
                    let mut relevant: BTreeSet<&Node> = BTreeSet::new();
                    relevant.insert(&c.node);
                    relevant.extend(closures.get(&c.node).into_iter().flatten());
                    let sorted: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
                    let path_owners: Vec<(&str, &Owner)> = sorted
                        .iter()
                        .filter_map(|p| owners.get(p).map(|o| (*p, o)))
                        .collect();
                    let found = reasons(&path_owners, &c.node.slug, &relevant, ownership);
                    (!found.is_empty()).then_some(Why::Paths(found))
                }
            },
        };
        if let Some(why) = why {
            out.push(Dirty {
                node: c.node.clone(),
                why,
            });
        }
    }
    out
}

/// Every candidate dirty, with [`Why::Everything`]: `vitrify run --recheck`
/// distrusts every recorded check. Sorted by node.
pub fn dirty_everything(candidates: &[Candidate]) -> Vec<Dirty> {
    let nodes: BTreeSet<&Node> = candidates.iter().map(|c| &c.node).collect();
    nodes
        .into_iter()
        .map(|n| Dirty {
            node: n.clone(),
            why: Why::Everything,
        })
        .collect()
}

/// Whether the SDD `slug`'s §5 Test command must run again: it is trusted
/// until a path any of the SDD's `units` is checked against changes since
/// the suite anchor, so it is dirty on the union of their reasons — and,
/// like a unit, when no anchor vouches for it. `changed` is the diff from
/// `anchor` to the head, when the caller has it. `None` means the recorded
/// pass still holds.
pub fn suite_dirty(
    slug: &str,
    units: &[Node],
    anchor: Option<&str>,
    changed: Option<&[String]>,
    closures: &Closures,
    ownership: &Ownership,
) -> Option<Why> {
    let anchor = match anchor {
        Some(a) => a,
        None => return Some(Why::NoAnchor),
    };
    let Some(paths) = changed else {
        return Some(Why::NoDiff(anchor.to_string()));
    };
    let mut relevant: BTreeSet<&Node> = BTreeSet::new();
    for u in units {
        relevant.insert(u);
        relevant.extend(closures.get(u).into_iter().flatten());
    }
    let sorted: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    let classified: Vec<(&str, Owner)> =
        sorted.iter().map(|p| (*p, ownership.classify(p))).collect();
    let path_owners: Vec<(&str, &Owner)> = classified.iter().map(|(p, o)| (*p, o)).collect();
    let found = reasons(&path_owners, slug, &relevant, ownership);
    (!found.is_empty()).then_some(Why::Paths(found))
}

/// Re-exported for callers that hold the journal: the actor of a re-check.
pub use journal::RECHECK_ACTOR;

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: i64, event: &str, unit: Option<&str>, commit: Option<&str>) -> Event {
        Event {
            seq,
            event: event.into(),
            unit: unit.map(Into::into),
            commit: commit.map(Into::into),
            ..Default::default()
        }
    }

    fn sdd(slug: &str, scope: &[&str], units: &[(&str, &[&str])]) -> SddOwnership {
        SddOwnership {
            slug: slug.into(),
            spec_paths: vec![format!("specs/{slug}")],
            scope_roots: scope.iter().map(|s| s.to_string()).collect(),
            unmanaged: Vec::new(),
            tests: Vec::new(),
            units: units
                .iter()
                .map(|(u, o)| (u.to_string(), o.iter().map(|g| g.to_string()).collect()))
                .collect(),
            shared_inputs: None,
        }
    }

    #[test]
    fn anchors_are_the_latest_verified_commit_and_the_latest_suite_pass() {
        let mut recheck = ev(3, "verified", Some("U-001"), Some("c3"));
        recheck.actor = Some(journal::RECHECK_ACTOR.into());
        let mut pass = ev(5, "audit", None, Some("c5"));
        pass.suite = Some("pass".into());
        let mut plain_audit = ev(6, "audit", None, Some("c6"));
        plain_audit.findings = Some(0);
        let events = vec![
            ev(1, "verified", Some("U-001"), Some("c1")),
            ev(2, "verified", Some("U-002"), Some("c1")),
            recheck,
            ev(4, "built", Some("U-002"), Some("c4")),
            pass,
            plain_audit,
        ];
        let a = Anchors::from_events(&events);
        assert_eq!(a.unit("U-001"), Some("c3"));
        assert_eq!(a.unit("U-002"), Some("c1"));
        assert_eq!(a.unit("U-003"), None);
        assert_eq!(a.suite.as_deref(), Some("c5"));
        // a latest `verified` with no commit vouches for nothing
        let a = Anchors::from_events(&[
            ev(1, "verified", Some("U-001"), Some("c1")),
            ev(2, "verified", Some("U-001"), None),
        ]);
        assert_eq!(a.unit("U-001"), None);
        assert_eq!(Anchors::from_events(&[]), Anchors::default());
    }

    #[test]
    fn overlapping_patterns_resolve_in_the_documented_order() {
        let mut a = sdd(
            "a",
            &["src/"],
            &[("U-002", &["src/lib/"]), ("U-001", &["src/*.rs"])],
        );
        a.unmanaged = vec!["src/gen/".into()];
        let mut b = sdd("b", &["src/"], &[("U-001", &["src/lib/x.rs"])]);
        b.spec_paths = vec!["src/lib/spec".into(), "specs/b".into()];
        let own = Ownership::new(vec![b, a]);
        // an SDD's own files beat every glob
        assert_eq!(own.classify("src/lib/spec/n.md"), Owner::Spec("b".into()));
        assert_eq!(own.classify("specs/a/sdd.md"), Owner::Spec("a".into()));
        // slug, then unit id: a U-001 beats a U-002, a beats b
        assert_eq!(
            own.classify("src/main.rs"),
            Owner::Unit(Node::new("a", "U-001"))
        );
        assert_eq!(
            own.classify("src/lib/x.rs"),
            Owner::Unit(Node::new("a", "U-001"))
        );
        assert_eq!(
            own.classify("src/lib/y.txt"),
            Owner::Unit(Node::new("a", "U-002"))
        );
        // unmanaged beats the scope fallback
        assert_eq!(own.classify("src/gen/z.txt"), Owner::Unmanaged("a".into()));
        // inside a scope, owned by no one: the first SDD's unmanaged input
        assert_eq!(
            own.classify("src/other/q.txt"),
            Owner::Unmanaged("a".into())
        );
        assert_eq!(own.classify("tools/x.sh"), Owner::Outside);
    }

    #[test]
    fn a_repository_wide_scope_root_leaves_nothing_outside() {
        let own = Ownership::new(vec![sdd("a", &["."], &[])]);
        assert_eq!(own.classify("tools/x.sh"), Owner::Unmanaged("a".into()));
        assert_eq!(own.classify("specs/a/sdd.md"), Owner::Spec("a".into()));
    }

    #[test]
    fn shared_inputs_filter_only_the_outside_paths() {
        let mut a = sdd("a", &["src/"], &[("U-001", &["src/a.rs"])]);
        a.shared_inputs = Some(vec!["tools/".into(), "ci/shared.env".into()]);
        let b = sdd("b", &["lib/"], &[("U-001", &["lib/b.rs"])]);
        let mut c = sdd("c", &["app/"], &[("U-001", &["app/c.rs"])]);
        c.shared_inputs = Some(Vec::new());
        let own = Ownership::new(vec![a, b, c]);
        let cands: Vec<Candidate> = ["a", "b", "c"]
            .iter()
            .map(|s| Candidate {
                node: Node::new(*s, "U-001"),
                anchor: Some("x".into()),
            })
            .collect();
        let run = |paths: &[&str]| {
            let changes = Changes::from([(
                "x".to_string(),
                paths.iter().map(|p| p.to_string()).collect(),
            )]);
            dirty_set(&cands, &changes, &Closures::new(), &own)
                .into_iter()
                .map(|d| d.node.slug)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&["tools/x.sh"]), ["a", "b"]);
        assert_eq!(run(&["ci/shared.env"]), ["a", "b"]);
        assert_eq!(run(&["ci/other.env"]), ["b"]);
        assert_eq!(run(&["README.md"]), ["b"]);
        assert!(run(&[]).is_empty());
    }

    #[test]
    fn a_unit_without_an_anchor_or_a_diff_is_dirty_and_says_why() {
        let own = Ownership::new(vec![sdd("a", &["src/"], &[("U-001", &["src/a.rs"])])]);
        let cands = vec![
            Candidate {
                node: Node::new("a", "U-002"),
                anchor: Some("gone".into()),
            },
            Candidate {
                node: Node::new("a", "U-001"),
                anchor: None,
            },
        ];
        let dirty = dirty_set(&cands, &Changes::new(), &Closures::new(), &own);
        assert_eq!(dirty.len(), 2);
        assert_eq!(dirty[0].node, Node::new("a", "U-001"));
        assert_eq!(dirty[0].why, Why::NoAnchor);
        assert_eq!(dirty[1].why, Why::NoDiff("gone".into()));
        assert_eq!(dirty_everything(&cands).len(), 2);
        assert_eq!(dirty_everything(&cands)[0].why, Why::Everything);
    }

    #[test]
    fn the_suite_is_dirty_on_any_unit_input_and_without_an_anchor() {
        let own = Ownership::new(vec![
            sdd("a", &["src/"], &[("U-001", &["src/a.rs"])]),
            sdd("b", &["lib/"], &[("U-001", &["lib/b.rs"])]),
        ]);
        let mut closures = Closures::new();
        closures.insert(
            Node::new("a", "U-001"),
            BTreeSet::from([Node::new("b", "U-001")]),
        );
        let units = [Node::new("a", "U-001")];
        let paths = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let check =
            |p: &[&str]| suite_dirty("a", &units, Some("x"), Some(&paths(p)), &closures, &own);
        assert_eq!(check(&["specs/a/sdd.md"]), None);
        let why = check(&["lib/b.rs", "specs/b/sdd.md"]).unwrap();
        assert_eq!(why.describe(), "lib/b.rs (unit b U-001)");
        assert_eq!(
            suite_dirty("a", &units, None, None, &closures, &own),
            Some(Why::NoAnchor)
        );
        assert_eq!(
            suite_dirty("a", &units, Some("x"), None, &closures, &own),
            Some(Why::NoDiff("x".into()))
        );
    }
}
