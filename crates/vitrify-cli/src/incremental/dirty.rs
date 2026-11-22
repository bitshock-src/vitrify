//! Dirty units from git: anchors, one diff per anchor, the core cascade.
//!
//! A [`Snapshot`] is the repository at one commit: every SDD loaded from a
//! checkout of it, the unit graph over them, each SDD's anchors from its
//! journal, and the declared ownership of every path. Asked which Verified
//! units a change reaches ([`Snapshot::dirty`]), it resolves each recorded
//! anchor — a short sha — to a commit, runs `git diff --name-only <anchor>
//! <head>` once per distinct commit (units integrated in one commit share
//! one), drops the paths whose change is protocol state only
//! ([`classify_changes`]), and hands the rest to
//! [`cascade::dirty_set`]. An anchor git cannot resolve — history
//! rewritten, a shallow clone — gets no diff, so its units are dirty with
//! that reason: the cascade never trusts what it cannot compare. Each SDD's
//! §5 Test command is judged the same way against its suite anchor.
//!
//! A graph with a cycle has no closures to cascade along: the cycles are
//! returned instead of a guess. Output is sorted by unit (slug, then id) and
//! SDDs by slug, so the same repository state gives the same answer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::cascade::{self, Anchors, Candidate, Changes, Closures, Dirty, Ownership, Why};
use vitrify_core::graph::{self, Cycles, Node, Resolution, UnitGraph};
use vitrify_core::journal;
use vitrify_core::model::{self, Instance, Shape};
use vitrify_core::trace;

use crate::cmd;
use crate::git::Git;

// ------------------------------------------------------ protocol state

/// Where the SDDs of a repository keep their files, repository-relative:
/// what tells protocol state from everything else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecFiles {
    /// Each bundle's directory (`""` for a bundle at the repository root),
    /// sorted.
    pub bundles: Vec<String>,
    /// Each single-file SDD's file, sorted.
    pub singles: Vec<String>,
}

impl SpecFiles {
    /// The spec files of `sdds`, loaded from the checkout at `root`.
    pub fn of(root: &Path, sdds: &[Instance]) -> SpecFiles {
        let mut out = SpecFiles::default();
        for inst in sdds {
            match inst.shape {
                Shape::Bundle => {
                    let dir = cmd::shown(root, &inst.root);
                    out.bundles
                        .push(if dir == "." { String::new() } else { dir });
                }
                Shape::Single => out.singles.push(cmd::shown(root, &inst.file)),
            }
        }
        out.bundles.sort();
        out.bundles.dedup();
        out.singles.sort();
        out.singles.dedup();
        out
    }

    fn is_bundle(&self, dir: &str) -> bool {
        self.bundles
            .binary_search_by(|b| b.as_str().cmp(dir))
            .is_ok()
    }

    fn is_single(&self, path: &str) -> bool {
        self.singles
            .binary_search_by(|s| s.as_str().cmp(path))
            .is_ok()
    }
}

/// The paths that differ between `old` and `new`, sorted, each with whether
/// its change is protocol state only: a bundle's journal, ledger, decision
/// or change log; a bundle's unit file, or a single-file SDD, that differs
/// in nothing but vitrify-managed fields (and, in a single file, those
/// sections). Such changes — claims, renewals, stamps, recorded questions —
/// change nothing a build, a test or a lint reads.
pub fn classify_changes(
    git: &Git,
    old: &str,
    new: &str,
    spec: &SpecFiles,
) -> Result<Vec<(String, bool)>, CoreError> {
    let both = |path: &str| -> Option<(String, String)> {
        Some((
            git.show_text(old, path).ok().flatten()?,
            git.show_text(new, path).ok().flatten()?,
        ))
    };
    let mut paths = git.changed_paths(old, new).map_err(CoreError::from)?;
    paths.sort();
    paths.dedup();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        let state_only = if matches!(
            name,
            "journal.ndjson" | "ledger.md" | "decisions.md" | "changelog.md"
        ) {
            spec.is_bundle(dir)
        } else if spec.is_single(&path) {
            both(&path).is_some_and(|(a, b)| single_state_masked(&a) == single_state_masked(&b))
        } else if (dir == "units" || dir.ends_with("/units"))
            && name.starts_with("U-")
            && name.ends_with(".md")
            && spec.is_bundle(dir.trim_end_matches("units").trim_end_matches('/'))
        {
            both(&path).is_some_and(|(a, b)| {
                trace::mask_unit_fields(Shape::Bundle, &a, "", &trace::VOLATILE)
                    == trace::mask_unit_fields(Shape::Bundle, &b, "", &trace::VOLATILE)
            })
        } else {
            false
        };
        out.push((path, state_only));
    }
    Ok(out)
}

/// What a run's dirty sets found at one head, kept for the next: each
/// anchor's commit, and the paths that differ between it and the head.
/// Moving to a new head diffs only the commits in between; a path they
/// touched is compared, anchor by anchor, in one batch.
#[derive(Debug, Default)]
pub struct DiffCache {
    head: String,
    /// Anchor as recorded → its commit (`None`: it names none).
    full_of: BTreeMap<String, Option<String>>,
    /// Anchor commit → the paths that differ between it and `head`, sorted.
    paths: BTreeMap<String, Vec<String>>,
}

impl DiffCache {
    /// Bring every kept diff from the old head to `head`. Anything that
    /// cannot be carried over is dropped, and diffed afresh when asked.
    fn advance(&mut self, git: &Git, head: &str) -> Result<(), CoreError> {
        if self.head == head {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.head, head.to_string());
        if old.is_empty()
            || self.paths.is_empty()
            || !git.is_ancestor(&old, head).map_err(CoreError::from)?
        {
            self.paths.clear();
            return Ok(());
        }
        let moved = git.changed_paths(&old, head).map_err(CoreError::from)?;
        if moved.is_empty() {
            return Ok(());
        }
        if moved.iter().any(|p| p.contains('\n')) {
            self.paths.clear();
            return Ok(());
        }
        // a path the new commits touched differs from an anchor when its
        // object there is not its object at the head
        let mut names: Vec<String> = moved.iter().map(|p| format!("{head}:{p}")).collect();
        let anchors: Vec<String> = self.paths.keys().cloned().collect();
        for a in &anchors {
            names.extend(moved.iter().map(|p| format!("{a}:{p}")));
        }
        let ids = git.object_ids(&names).map_err(CoreError::from)?;
        let at_head = &ids[..moved.len()];
        for (k, a) in anchors.iter().enumerate() {
            let at_anchor = &ids[(k + 1) * moved.len()..(k + 2) * moved.len()];
            let Some(paths) = self.paths.get_mut(a) else {
                continue;
            };
            let mut set: BTreeSet<String> = std::mem::take(paths).into_iter().collect();
            for (i, p) in moved.iter().enumerate() {
                if at_anchor[i] == at_head[i] {
                    set.remove(p);
                } else {
                    set.insert(p.clone());
                }
            }
            *paths = set.into_iter().collect();
        }
        Ok(())
    }
}

/// The paths that differ between `old` and `new` beyond protocol state,
/// sorted.
pub fn substantive_changes(
    git: &Git,
    old: &str,
    new: &str,
    spec: &SpecFiles,
) -> Result<Vec<String>, CoreError> {
    Ok(classify_changes(git, old, new, spec)?
        .into_iter()
        .filter(|(_, state_only)| !state_only)
        .map(|(path, _)| path)
        .collect())
}

/// Whether `old..new` moved nothing but protocol state: `new` descends
/// from `old`, and no path changed beyond it. What was established at
/// `old` then holds at `new`.
pub fn benign(git: &Git, old: &str, new: &str, spec: &SpecFiles) -> Result<bool, CoreError> {
    if old == new {
        return Ok(true);
    }
    if !git.is_ancestor(old, new).map_err(CoreError::from)? {
        return Ok(false);
    }
    Ok(substantive_changes(git, old, new, spec)?.is_empty())
}

/// A single-file SDD's text with everything vitrify itself writes removed:
/// each unit's managed fields, the journal, the ledger, the decision and
/// change logs.
fn single_state_masked(text: &str) -> String {
    let mut t = text.to_string();
    let ids: Vec<String> = model::split_h3_regions(&t, "Work Units")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        if let Some(masked) = trace::mask_unit_fields(Shape::Single, &t, &id, &trace::VOLATILE) {
            t = masked;
        }
    }
    for heading in [
        "Journal",
        "State Ledger",
        "Decisions & Open Questions",
        "Changelog",
    ] {
        if let Some((s, e)) = model::section_span(&t, heading) {
            t = format!("{}{}", &t[..s], &t[e..]);
        }
    }
    t
}

// ------------------------------------------------------------ snapshot

/// One SDD of a [`Snapshot`].
#[derive(Debug, Clone)]
pub struct SddAt {
    /// Its `sdd:` slug.
    pub slug: String,
    /// Its root as output names it: repository-relative (`specs/calc`,
    /// `specs/x.sdd.md`).
    pub root: String,
    pub inst: Instance,
    /// Its recorded checks, from its journal.
    pub anchors: Anchors,
}

/// The repository at one commit, as the re-check cascade reads it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The checkout the SDDs were loaded from.
    pub checkout: PathBuf,
    /// The commit, in full.
    pub head: String,
    /// Every SDD of the repository, sorted by slug, then root.
    pub sdds: Vec<SddAt>,
    pub graph: UnitGraph,
    /// The graph's order and closures, or its cycles.
    pub resolved: Result<Resolution, Cycles>,
    pub ownership: Ownership,
    pub spec: SpecFiles,
}

/// How far recorded checks are trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Each until a path it is checked against changes.
    Changes,
    /// Not at all: every Verified unit and every suite is dirty
    /// (`vitrify run --recheck`).
    Everything,
}

/// One SDD's §5 Test command, against its suite anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteState {
    pub slug: String,
    /// The anchor its journal records: the commit of its latest `audit`
    /// event with `suite: "pass"`.
    pub anchor: Option<String>,
    /// Why it must run again; `None` when the recorded pass holds.
    pub dirty: Option<Why>,
}

/// Which Verified units — and which suites — a change reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirtySet {
    /// The commit judged, in full.
    pub head: String,
    /// The dirty units, sorted by node, each with its reason.
    pub units: Vec<Dirty>,
    /// Every SDD's suite state, sorted by slug.
    pub suites: Vec<SuiteState>,
    /// The recorded anchors git could not resolve, sorted.
    pub unresolved: Vec<String>,
}

impl DirtySet {
    /// The dirty units of the SDD `slug`, sorted by unit id.
    pub fn of_sdd<'a>(&'a self, slug: &'a str) -> impl Iterator<Item = &'a Dirty> + 'a {
        self.units.iter().filter(move |d| d.node.slug == slug)
    }

    /// Whether `node` is dirty.
    pub fn is_dirty(&self, node: &Node) -> bool {
        self.units.binary_search_by(|d| d.node.cmp(node)).is_ok()
    }

    /// The suite state of `slug`'s SDD.
    pub fn suite(&self, slug: &str) -> Option<&SuiteState> {
        self.suites.iter().find(|s| s.slug == slug)
    }
}

/// What the cascade made of a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cascade {
    Resolved(DirtySet),
    /// The unit graph has a cycle: there is no closure to cascade along,
    /// so nothing is computed.
    Cycles(Cycles),
}

/// One human line for why a unit or a suite is dirty: the changed paths
/// with their owners, `no recorded check`, `every recorded check is
/// distrusted`, or `anchor <c> cannot be resolved …`.
pub fn describe(why: &Why) -> String {
    match why {
        Why::NoDiff(anchor) => {
            format!("anchor {anchor} cannot be resolved (rewritten history, or a shallow clone)")
        }
        other => other.describe(),
    }
}

impl Snapshot {
    /// Load every SDD of the repository from `checkout`, which holds the
    /// tree of the commit `head` names (a lane checkout reset to it, or the
    /// primary checkout at `HEAD`). An SDD that cannot be read is an error:
    /// without it the graph, and every closure through it, is incomplete.
    pub fn load(git: &Git, checkout: &Path, head: &str) -> Result<Snapshot, CoreError> {
        let head = git
            .resolve_commit(head)
            .map_err(CoreError::from)?
            .ok_or_else(|| CoreError::refusal(1, format!("error: {head} names no commit")))?;
        let mut sdds: Vec<SddAt> = Vec::new();
        for handle in cmd::discover(checkout) {
            let inst = Instance::load(handle)?;
            let (events, _) = journal::read(&inst.handle())?;
            sdds.push(SddAt {
                slug: graph::slug_of(&inst).to_string(),
                root: cmd::shown(checkout, &inst.root),
                anchors: Anchors::from_events(&events),
                inst,
            });
        }
        sdds.sort_by(|a, b| (&a.slug, &a.root).cmp(&(&b.slug, &b.root)));
        let instances: Vec<Instance> = sdds.iter().map(|s| s.inst.clone()).collect();
        let graph = UnitGraph::build(&instances)?;
        let resolved = graph.resolve();
        let ownership =
            Ownership::build(checkout, sdds.iter().map(|s| (s.slug.as_str(), &s.inst)))?;
        let spec = SpecFiles::of(checkout, &instances);
        Ok(Snapshot {
            checkout: checkout.to_path_buf(),
            head,
            sdds,
            graph,
            resolved,
            ownership,
            spec,
        })
    }

    /// [`Snapshot::load`] for a run that must go on whatever one SDD holds:
    /// an SDD that cannot be read is left out, and when the unit graph has
    /// a cycle the SDDs with a unit on one are left out of the graph — so
    /// the rest still get their closures, dirty sets and priorities — and
    /// the cycles are returned. Ownership and spec files still count every
    /// SDD read, so a change to a left-out SDD's files is still its own.
    pub fn load_resolvable(
        git: &Git,
        checkout: &Path,
        head: &str,
    ) -> Result<(Snapshot, Option<Cycles>), CoreError> {
        let full = git
            .resolve_commit(head)
            .map_err(CoreError::from)?
            .ok_or_else(|| CoreError::refusal(1, format!("error: {head} names no commit")))?;
        let mut all: Vec<SddAt> = Vec::new();
        for handle in cmd::discover(checkout) {
            let Ok(inst) = Instance::load(handle) else {
                continue;
            };
            let (events, _) = journal::read(&inst.handle()).unwrap_or_default();
            all.push(SddAt {
                slug: graph::slug_of(&inst).to_string(),
                root: cmd::shown(checkout, &inst.root),
                anchors: Anchors::from_events(&events),
                inst,
            });
        }
        all.sort_by(|a, b| (&a.slug, &a.root).cmp(&(&b.slug, &b.root)));
        let every: Vec<Instance> = all.iter().map(|s| s.inst.clone()).collect();
        let ownership = Ownership::build(checkout, all.iter().map(|s| (s.slug.as_str(), &s.inst)))?;
        let spec = SpecFiles::of(checkout, &every);
        let mut graph = UnitGraph::build(&every)?;
        let mut resolved = graph.resolve();
        let mut cycles = None;
        if let Err(c) = &resolved {
            let on_cycle: BTreeSet<&str> =
                c.cycles.iter().flatten().map(|n| n.slug.as_str()).collect();
            let kept: Vec<SddAt> = all
                .iter()
                .filter(|s| !on_cycle.contains(s.slug.as_str()))
                .cloned()
                .collect();
            cycles = Some(c.clone());
            let instances: Vec<Instance> = kept.iter().map(|s| s.inst.clone()).collect();
            graph = UnitGraph::build(&instances)?;
            resolved = graph.resolve();
            all = kept;
        }
        Ok((
            Snapshot {
                checkout: checkout.to_path_buf(),
                head: full,
                sdds: all,
                graph,
                resolved,
                ownership,
                spec,
            },
            cycles,
        ))
    }

    /// The SDD with `slug`.
    pub fn sdd(&self, slug: &str) -> Option<&SddAt> {
        self.sdds.iter().find(|s| s.slug == slug)
    }

    /// Every unit Verified at the head, with its anchor; sorted by node.
    pub fn candidates(&self) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self
            .sdds
            .iter()
            .flat_map(|s| {
                s.inst
                    .units
                    .iter()
                    .filter(|u| u.meta.get_str_or("status", "Draft") == "Verified")
                    .map(|u| Candidate {
                        node: Node::new(&s.slug, &u.unit),
                        anchor: s.anchors.unit(&u.unit).map(str::to_string),
                    })
            })
            .collect();
        out.sort_by(|a, b| a.node.cmp(&b.node));
        out
    }

    /// Which Verified units, and which suites, must be checked again at the
    /// head. In [`Mode::Changes`] git is asked for one diff per distinct
    /// anchor; in [`Mode::Everything`] nothing is asked and everything is
    /// dirty.
    pub fn dirty(&self, git: &Git, mode: Mode) -> Result<Cascade, CoreError> {
        self.dirty_with(git, mode, &mut DiffCache::default())
    }

    /// [`Snapshot::dirty`], with what an earlier head's diffs found kept in
    /// `cache`: a run reads every head, and most of what changed since an
    /// anchor had changed by the head before.
    pub fn dirty_with(
        &self,
        git: &Git,
        mode: Mode,
        cache: &mut DiffCache,
    ) -> Result<Cascade, CoreError> {
        let resolution = match &self.resolved {
            Ok(r) => r,
            Err(cycles) => return Ok(Cascade::Cycles(cycles.clone())),
        };
        let candidates = self.candidates();
        if mode == Mode::Everything {
            return Ok(Cascade::Resolved(DirtySet {
                head: self.head.clone(),
                units: cascade::dirty_everything(&candidates),
                suites: self
                    .sdds
                    .iter()
                    .map(|s| SuiteState {
                        slug: s.slug.clone(),
                        anchor: s.anchors.suite.clone(),
                        dirty: Some(Why::Everything),
                    })
                    .collect(),
                unresolved: Vec::new(),
            }));
        }
        let closures: Closures = self
            .graph
            .nodes()
            .iter()
            .map(|n| {
                let deps: BTreeSet<Node> = resolution.closure(n).into_iter().cloned().collect();
                (n.clone(), deps)
            })
            .collect();

        // every recorded anchor, once: units and suites
        let mut recorded: BTreeSet<&str> = candidates
            .iter()
            .filter_map(|c| c.anchor.as_deref())
            .collect();
        recorded.extend(self.sdds.iter().filter_map(|s| s.anchors.suite.as_deref()));
        let mut full_of: BTreeMap<&str, String> = BTreeMap::new();
        let mut unresolved: Vec<String> = Vec::new();
        let unknown: Vec<&str> = recorded
            .iter()
            .copied()
            .filter(|a| !cache.full_of.contains_key(*a))
            .collect();
        let names: Vec<String> = unknown.iter().map(|a| format!("{a}^{{commit}}")).collect();
        for (anchor, full) in unknown
            .iter()
            .zip(git.object_ids(&names).map_err(CoreError::from)?)
        {
            cache.full_of.insert(anchor.to_string(), full);
        }
        for anchor in recorded {
            match cache.full_of.get(anchor).cloned().flatten() {
                Some(full) => {
                    full_of.insert(anchor, full);
                }
                None => unresolved.push(anchor.to_string()),
            }
        }
        cache.advance(git, &self.head)?;
        // one diff per distinct commit, whatever names it. Protocol state
        // lives under the SDDs' own roots, and the cascade never takes a
        // spec path as an input (`Owner::Spec`): a plain diff gives the
        // same answer as one with protocol state filtered out, without
        // reading every changed spec file at both ends for every anchor.
        let mut diffs: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for full in full_of.values() {
            if !diffs.contains_key(full.as_str()) {
                let paths = match cache.paths.get(full.as_str()) {
                    Some(paths) => paths.clone(),
                    None => {
                        let mut paths = git
                            .changed_paths(full, &self.head)
                            .map_err(CoreError::from)?;
                        paths.sort();
                        paths.dedup();
                        cache.paths.insert(full.clone(), paths.clone());
                        paths
                    }
                };
                diffs.insert(full.as_str(), paths);
            }
        }
        // what no anchor names any more is let go
        let named: BTreeSet<&String> = full_of.values().collect();
        cache.paths.retain(|full, _| named.contains(full));
        let changes: Changes = full_of
            .iter()
            .map(|(anchor, full)| {
                let paths = diffs.get(full.as_str()).cloned().unwrap_or_default();
                (anchor.to_string(), paths)
            })
            .collect();

        let units = cascade::dirty_set(&candidates, &changes, &closures, &self.ownership);
        let suites = self
            .sdds
            .iter()
            .map(|s| {
                let nodes: Vec<Node> = s
                    .inst
                    .units
                    .iter()
                    .map(|u| Node::new(&s.slug, &u.unit))
                    .collect();
                let anchor = s.anchors.suite.as_deref();
                let changed = anchor.and_then(|a| changes.get(a)).map(Vec::as_slice);
                SuiteState {
                    slug: s.slug.clone(),
                    anchor: s.anchors.suite.clone(),
                    dirty: cascade::suite_dirty(
                        &s.slug,
                        &nodes,
                        anchor,
                        changed,
                        &closures,
                        &self.ownership,
                    ),
                }
            })
            .collect();
        Ok(Cascade::Resolved(DirtySet {
            head: self.head.clone(),
            units,
            suites,
            unresolved,
        }))
    }
}
