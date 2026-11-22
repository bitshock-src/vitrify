//! The repository's one unit graph: every SDD's units as nodes, joined by
//! their `depends_on` edges — to units of the same SDD, and through pins to
//! the units that produce the pinned contract in another SDD.
//!
//! A `depends_on` entry names a unit of its own SDD or, from schema 2.1, a
//! pin of its own SDD: a contract whose header carries `- Pin: <slug> C-###
//! @ <hash>`. A pin entry waits for every unit named on the `Producer:` line
//! of the pin's authority — contract `C-###` of the SDD with that slug. That
//! is the only source of order between SDDs; consuming a pin orders nothing.
//!
//! Over the graph this module computes everything a build across SDDs
//! needs, deterministically: an order (Kahn's algorithm, ready units taken
//! by slug, then unit id), every cycle when there is no order (Tarjan's
//! strongly connected components over what Kahn leaves, each cycle the path
//! that closes it), each unit's transitive dependencies, critical-path
//! priorities (Hu's labels over the unfinished units), and the SDD graph
//! derived from the unit edges.
//!
//! A retired unit stays a node with its edges. Its status is never Built or
//! Verified, so a unit that depends on it is never claimable — what
//! [`model::unit_view`](crate::model::unit_view) already makes of such a dependency.

use std::cmp::{Ordering, Reverse};
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

use crate::CoreError;
use crate::contract::{self, Naming, Pin};
use crate::model::{Instance, Schema};

/// One unit of the repository's unit graph: its SDD's slug and its id.
/// Ordered by slug, then unit id — the tie-break of every order the graph
/// yields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Node {
    pub slug: String,
    pub unit: String,
}

impl Node {
    pub fn new(slug: impl Into<String>, unit: impl Into<String>) -> Node {
        Node {
            slug: slug.into(),
            unit: unit.into(),
        }
    }
}

impl fmt::Display for Node {
    /// `<slug> <unit>`, as every report names a unit of another SDD.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.slug, self.unit)
    }
}

/// A path of units as reports print it: `core U-004 → api U-002 → core U-004`.
pub fn render_path(path: &[Node]) -> String {
    let names: Vec<String> = path.iter().map(Node::to_string).collect();
    names.join(" → ")
}

/// An SDD's slug: its `sdd:` front-matter value.
pub fn slug_of(inst: &Instance) -> &str {
    inst.meta.get_str("sdd").map(str::trim).unwrap_or("")
}

/// `U-001`; `U-001 and U-002`; `U-001, U-002 and U-003`.
fn listed(ids: &[String]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

// ------------------------------------------------------------ contracts

/// What a contract of an SDD is, as far as `depends_on` is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContractKind {
    /// Produced in this SDD by the units its `Producer:` line names.
    Internal { producers: Vec<String> },
    /// A pin of another SDD's contract.
    Pin(Pin),
    /// External, and its `Producer:` line names an authority, but it has
    /// no `Pin:` line that reads `<slug> C-###`.
    Unpinned,
    /// External, naming no contract: there is no authority to wait for.
    Transcription,
}

fn kind_of(text: &str) -> ContractKind {
    let producer = contract::field(text, "Producer").unwrap_or_default();
    if let Some(value) = contract::field(text, "Pin") {
        return match contract::parse_pin(&value) {
            Some(pin) => ContractKind::Pin(pin),
            None => ContractKind::Unpinned,
        };
    }
    if !contract::is_external(&producer) {
        return ContractKind::Internal {
            producers: contract::unit_ids(&producer),
        };
    }
    match contract::naming(&producer, &[]) {
        Naming::Nothing => ContractKind::Transcription,
        _ => ContractKind::Unpinned,
    }
}

/// Every contract of `inst` by id, with its kind; an id held twice is the
/// one [`contract::list`] keeps.
fn contract_kinds(inst: &Instance) -> Result<Vec<(String, ContractKind)>, CoreError> {
    Ok(contract::list(&inst.handle())?
        .into_iter()
        .map(|c| {
            let kind = kind_of(&c.text);
            (c.id, kind)
        })
        .collect())
}

// ------------------------------------------------------ depends_on entries

/// What one `depends_on` entry of a unit names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A unit of the same SDD.
    Unit,
    /// A pin of the same SDD, the unit consumes it, and its SDD is at
    /// schema 2.1: the unit waits for the units producing the authority.
    Pin(Pin),
    /// Anything else: lint L1's finding.
    Invalid(EntryProblem),
}

/// Why a `depends_on` entry is not one a unit may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryProblem {
    /// No unit and no contract of the SDD has this id.
    Unknown,
    /// An internal contract of the SDD: the unit names the units that
    /// produce it (its `Producer:` line's), not the contract.
    Internal { producers: Vec<String> },
    /// A transcription: no SDD of the repository produces it, so there is
    /// nothing to wait for.
    Transcription,
    /// An external contract with no `Pin:` line that reads `<slug> C-###`.
    Unpinned,
    /// A pin the unit does not also list in `consumes`.
    NotConsumed,
    /// A pin, in an SDD that declares schema 2.0: a pin in `depends_on` is
    /// 2.1's.
    Schema2_0,
}

impl EntryProblem {
    /// The finding's cause, for `unit`'s entry `entry`.
    pub fn cause(&self, unit: &str, entry: &str) -> String {
        match self {
            EntryProblem::Unknown if entry.starts_with("U-") => {
                format!("depends_on references nonexistent unit {entry}")
            }
            EntryProblem::Unknown => {
                format!(
                    "depends_on names {entry}, which is neither a unit nor a contract of this SDD"
                )
            }
            EntryProblem::Internal { .. } => format!(
                "depends_on names {entry}, an internal contract of this SDD — name the unit instead"
            ),
            EntryProblem::Transcription => format!(
                "depends_on names {entry}, a transcription — it has no authority, so there is nothing to wait for"
            ),
            EntryProblem::Unpinned => format!(
                "depends_on names {entry}, an external contract without a `Pin:` line that reads `<slug> C-###`"
            ),
            EntryProblem::NotConsumed => {
                format!("depends_on names pin {entry}, which {unit} does not consume")
            }
            EntryProblem::Schema2_0 => format!(
                "depends_on names pin {entry}, and this SDD declares schema_version 2.0 — a pin in depends_on is schema 2.1"
            ),
        }
    }

    /// The edit that resolves it.
    pub fn remediation(&self, unit: &str, entry: &str) -> String {
        match self {
            EntryProblem::Unknown if entry.starts_with("U-") => {
                format!("remove {entry} from {unit}'s depends_on, or add unit {entry}")
            }
            EntryProblem::Unknown | EntryProblem::Transcription => {
                format!("remove {entry} from {unit}'s depends_on")
            }
            EntryProblem::Internal { producers } if !producers.is_empty() => format!(
                "replace {entry} in {unit}'s depends_on with {}",
                listed(producers)
            ),
            EntryProblem::Internal { .. } => {
                format!("replace {entry} in {unit}'s depends_on with the unit that produces it")
            }
            EntryProblem::Unpinned => {
                format!("write {entry}'s `- Pin: <slug> C-###` line, then run vitrify pin sync")
            }
            EntryProblem::NotConsumed => {
                format!("add {entry} to {unit}'s consumes, or remove it from {unit}'s depends_on")
            }
            EntryProblem::Schema2_0 => format!(
                "set schema_version: 2.1 in the SDD's core front matter (a PATCH change entry), or remove {entry} from {unit}'s depends_on"
            ),
        }
    }
}

/// One unit's `depends_on`, each entry classified, in listed order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitEntries {
    pub unit: String,
    pub entries: Vec<(String, Entry)>,
}

/// Classify every `depends_on` entry of every unit of `inst` against its
/// own units and contracts — lint L1, which needs no other SDD. Units in
/// the order the SDD holds them; a unit id held twice is classified once.
pub fn classify(inst: &Instance) -> Result<Vec<UnitEntries>, CoreError> {
    let contracts = contract_kinds(inst)?;
    let schema = inst.schema();
    let mut out: Vec<UnitEntries> = Vec::new();
    for u in &inst.units {
        if out.iter().any(|e| e.unit == u.unit) {
            continue;
        }
        let consumes = u.meta.get_list("consumes");
        let entries = u
            .meta
            .get_list("depends_on")
            .into_iter()
            .map(|d| {
                let entry = classify_entry(inst, &contracts, schema, &consumes, &d);
                (d, entry)
            })
            .collect();
        out.push(UnitEntries {
            unit: u.unit.clone(),
            entries,
        });
    }
    Ok(out)
}

fn classify_entry(
    inst: &Instance,
    contracts: &[(String, ContractKind)],
    schema: Schema,
    consumes: &[String],
    d: &str,
) -> Entry {
    if inst.unit(d).is_some() {
        return Entry::Unit;
    }
    let Some((_, kind)) = contracts.iter().find(|(id, _)| id == d) else {
        return Entry::Invalid(EntryProblem::Unknown);
    };
    match kind {
        ContractKind::Internal { producers } => Entry::Invalid(EntryProblem::Internal {
            producers: producers.clone(),
        }),
        ContractKind::Transcription => Entry::Invalid(EntryProblem::Transcription),
        ContractKind::Unpinned => Entry::Invalid(EntryProblem::Unpinned),
        ContractKind::Pin(_) if !schema.pins_in_depends_on() => {
            Entry::Invalid(EntryProblem::Schema2_0)
        }
        ContractKind::Pin(_) if !consumes.iter().any(|c| c == d) => {
            Entry::Invalid(EntryProblem::NotConsumed)
        }
        ContractKind::Pin(pin) => Entry::Pin(pin.clone()),
    }
}

/// The SDDs at schema 2.0 whose units consume pins — what a run across
/// SDDs refuses on (exit 4), naming them: under 2.0 nothing can say which
/// of those pins a unit needs built first. Ordered by slug, then file.
pub fn pins_on_2_0(sdds: &[Instance]) -> Result<Vec<&Instance>, CoreError> {
    let mut out: Vec<&Instance> = Vec::new();
    for inst in sdds {
        if inst.schema() != Schema::V2_0 {
            continue;
        }
        let pins: Vec<String> = contract_kinds(inst)?
            .into_iter()
            .filter(|(_, k)| matches!(k, ContractKind::Pin(_) | ContractKind::Unpinned))
            .map(|(id, _)| id)
            .collect();
        if inst
            .units
            .iter()
            .any(|u| u.meta.get_list("consumes").iter().any(|c| pins.contains(c)))
        {
            out.push(inst);
        }
    }
    out.sort_by(|a, b| (slug_of(a), &a.file).cmp(&(slug_of(b), &b.file)));
    Ok(out)
}

// ------------------------------------------------------------- problems

/// Why a pin in `depends_on` does not lead to units to wait for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinProblem {
    /// No SDD of the repository has the pin's slug.
    NoSdd,
    /// Several SDDs have the pin's slug.
    SharedSlug,
    /// The authority SDD has no such contract.
    NoContract,
    /// The authority's `Producer:` line names no unit (it is absent, names
    /// none, or makes the authority external itself).
    NoProducers,
    /// The authority's `Producer:` line names ids that are no units of the
    /// authority SDD.
    UnknownProducers(Vec<String>),
}

/// What is wrong with one `depends_on` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    /// The entry names nothing a unit may wait on (lint L1).
    Entry(EntryProblem),
    /// The entry is a pin whose authority leads to no units (lint L12).
    Pin { pin: Pin, problem: PinProblem },
}

/// A `depends_on` entry the graph cannot use: the SDD, the unit, the entry
/// and the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub slug: String,
    pub unit: String,
    pub entry: String,
    pub kind: ProblemKind,
}

impl Problem {
    /// The lint check that reports it.
    pub fn check(&self) -> &'static str {
        match self.kind {
            ProblemKind::Entry(_) => "L1",
            ProblemKind::Pin { .. } => "L12",
        }
    }

    pub fn cause(&self) -> String {
        let (unit, entry) = (&self.unit, &self.entry);
        match &self.kind {
            ProblemKind::Entry(p) => p.cause(unit, entry),
            ProblemKind::Pin { pin, problem } => {
                let at = format!("{} {}", pin.slug, pin.contract);
                let what = match problem {
                    PinProblem::NoSdd => {
                        format!("no SDD in this repository has the slug `{}`", pin.slug)
                    }
                    PinProblem::SharedSlug => {
                        format!("several SDDs have the slug `{}`", pin.slug)
                    }
                    PinProblem::NoContract => {
                        format!("{} has no contract {}", pin.slug, pin.contract)
                    }
                    PinProblem::NoProducers => {
                        format!("the Producer line of {at} names no unit")
                    }
                    PinProblem::UnknownProducers(ids) => format!(
                        "the Producer line of {at} names {}, which {} no unit of {}",
                        listed(ids),
                        if ids.len() == 1 { "is" } else { "are" },
                        pin.slug
                    ),
                };
                format!("depends_on names pin {entry} ({at}), and {what}")
            }
        }
    }

    pub fn remediation(&self) -> String {
        let (unit, entry) = (&self.unit, &self.entry);
        match &self.kind {
            ProblemKind::Entry(p) => p.remediation(unit, entry),
            ProblemKind::Pin { pin, problem } => match problem {
                PinProblem::NoSdd | PinProblem::NoContract => format!(
                    "point {entry}'s `Pin:` line at a contract another SDD owns, or remove {entry} from {unit}'s depends_on"
                ),
                PinProblem::SharedSlug => format!(
                    "give each SDD with the slug `{}` a slug of its own",
                    pin.slug
                ),
                PinProblem::NoProducers | PinProblem::UnknownProducers(_) => format!(
                    "name the units of {} that produce {} on its Producer line",
                    pin.slug, pin.contract
                ),
            },
        }
    }
}

/// Several SDDs with one slug: none of them joins the graph, since a node
/// is named by slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub slug: String,
    /// Each SDD's core file, sorted.
    pub files: Vec<PathBuf>,
}

// ---------------------------------------------------------------- graph

/// One edge of the unit graph: `from` depends on `to`, directly or — `via`
/// the pin of `from`'s SDD that names it — through a pin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    pub from: Node,
    pub to: Node,
    pub via: Option<String>,
}

/// A pin entry resolved: the units a unit waits on through it.
#[derive(Debug, Clone)]
struct PinUse {
    unit: usize,
    entry: String,
    producers: Vec<usize>,
    /// Every producer the authority names is a unit of its SDD.
    whole: bool,
}

/// The unit graph over every SDD of a repository.
#[derive(Debug, Clone)]
pub struct UnitGraph {
    /// Sorted by (slug, unit); a node's index is its position.
    nodes: Vec<Node>,
    /// Each node's status, as its SDD declared it when the graph was built.
    status: Vec<String>,
    /// Dependencies (outgoing edges), sorted and without repeats.
    deps: Vec<Vec<usize>>,
    /// Dependents (incoming edges), sorted and without repeats.
    dependents: Vec<Vec<usize>>,
    /// Every edge with its provenance, sorted.
    edges: Vec<Edge>,
    /// Every unit's classified `depends_on`, by node.
    entries: Vec<(usize, Vec<(String, Entry)>)>,
    /// Sorted by (unit, entry).
    pins: Vec<PinUse>,
    problems: Vec<Problem>,
    clashes: Vec<Clash>,
    /// The slugs of the SDDs in the graph, sorted.
    slugs: Vec<String>,
}

impl UnitGraph {
    /// Build the graph over `sdds`, every SDD of one repository. Each
    /// `depends_on` entry is classified ([`classify`]); a pin entry is
    /// followed to its authority among `sdds`. Entries that lead nowhere
    /// are [`problems`](UnitGraph::problems), not errors: the graph holds
    /// every edge that does resolve. `Err` only when a spec file cannot be
    /// read.
    pub fn build(sdds: &[Instance]) -> Result<UnitGraph, CoreError> {
        // SDDs by slug; a slug held by several keeps all of them out
        let mut by_slug: Vec<(&str, Vec<&Instance>)> = Vec::new();
        for inst in sdds {
            let s = slug_of(inst);
            match by_slug.iter_mut().find(|(k, _)| *k == s) {
                Some((_, v)) => v.push(inst),
                None => by_slug.push((s, vec![inst])),
            }
        }
        by_slug.sort_by(|a, b| a.0.cmp(b.0));
        let mut clashes: Vec<Clash> = Vec::new();
        let mut members: Vec<(&str, &Instance)> = Vec::new();
        for (s, insts) in &by_slug {
            match insts.as_slice() {
                [one] => members.push((s, one)),
                several => {
                    let mut files: Vec<PathBuf> = several.iter().map(|i| i.file.clone()).collect();
                    files.sort();
                    clashes.push(Clash {
                        slug: s.to_string(),
                        files,
                    });
                }
            }
        }

        // nodes, sorted
        let mut nodes: Vec<(Node, String)> = Vec::new();
        for (s, inst) in &members {
            for u in &inst.units {
                let node = Node::new(*s, u.unit.clone());
                if !nodes.iter().any(|(n, _)| *n == node) {
                    nodes.push((node, u.meta.get_str_or("status", "Draft").to_string()));
                }
            }
        }
        nodes.sort();
        let (nodes, status): (Vec<Node>, Vec<String>) = nodes.into_iter().unzip();
        let index = |slug: &str, unit: &str| -> Option<usize> {
            nodes
                .binary_search_by(|n| (n.slug.as_str(), n.unit.as_str()).cmp(&(slug, unit)))
                .ok()
        };

        let mut edges: Vec<(usize, usize, Option<String>)> = Vec::new();
        let mut entries: Vec<(usize, Vec<(String, Entry)>)> = Vec::new();
        let mut pins: Vec<PinUse> = Vec::new();
        let mut problems: Vec<Problem> = Vec::new();
        // each authority looked up once: (slug, contract) → its producers
        let mut authorities: Vec<((String, String), Producers)> = Vec::new();

        for (s, inst) in &members {
            for ue in classify(inst)? {
                // invariant: every unit of a member SDD is a node
                let Some(from) = index(s, &ue.unit) else {
                    continue;
                };
                for (d, entry) in &ue.entries {
                    let problem = |kind: ProblemKind| Problem {
                        slug: s.to_string(),
                        unit: ue.unit.clone(),
                        entry: d.clone(),
                        kind,
                    };
                    match entry {
                        Entry::Unit => {
                            if let Some(to) = index(s, d) {
                                edges.push((from, to, None));
                            }
                        }
                        Entry::Invalid(p) => problems.push(problem(ProblemKind::Entry(p.clone()))),
                        Entry::Pin(pin) => {
                            let key = (pin.slug.clone(), pin.contract.clone());
                            let found = match authorities.iter().find(|(k, _)| *k == key) {
                                Some((_, r)) => r.clone(),
                                None => {
                                    let r = authority(pin, &by_slug, &index)?;
                                    authorities.push((key, r.clone()));
                                    r
                                }
                            };
                            let (producers, unknown) = match found {
                                Ok(found) => found,
                                Err(p) => {
                                    problems.push(problem(ProblemKind::Pin {
                                        pin: pin.clone(),
                                        problem: p,
                                    }));
                                    continue;
                                }
                            };
                            for &to in &producers {
                                edges.push((from, to, Some(d.clone())));
                            }
                            if !unknown.is_empty() {
                                problems.push(problem(ProblemKind::Pin {
                                    pin: pin.clone(),
                                    problem: PinProblem::UnknownProducers(unknown.clone()),
                                }));
                            }
                            pins.push(PinUse {
                                unit: from,
                                entry: d.clone(),
                                producers,
                                whole: unknown.is_empty(),
                            });
                        }
                    }
                }
                entries.push((from, ue.entries));
            }
        }

        let n = nodes.len();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (from, to, _) in &edges {
            deps[*from].push(*to);
            dependents[*to].push(*from);
        }
        for list in deps.iter_mut().chain(dependents.iter_mut()) {
            list.sort_unstable();
            list.dedup();
        }
        let mut edges: Vec<Edge> = edges
            .into_iter()
            .map(|(from, to, via)| Edge {
                from: nodes[from].clone(),
                to: nodes[to].clone(),
                via,
            })
            .collect();
        edges.sort();
        edges.dedup();
        entries.sort_by_key(|(i, _)| *i);
        pins.sort_by(|a, b| (a.unit, &a.entry).cmp(&(b.unit, &b.entry)));
        pins.dedup_by(|a, b| a.unit == b.unit && a.entry == b.entry);
        problems.sort_by(|a, b| {
            (&a.slug, &a.unit, &a.entry, a.check()).cmp(&(&b.slug, &b.unit, &b.entry, b.check()))
        });
        problems.dedup();
        let slugs = members.iter().map(|(s, _)| s.to_string()).collect();

        Ok(UnitGraph {
            nodes,
            status,
            deps,
            dependents,
            edges,
            entries,
            pins,
            problems,
            clashes,
            slugs,
        })
    }

    fn index(&self, n: &Node) -> Option<usize> {
        self.nodes.binary_search(n).ok()
    }

    /// Every unit, sorted by slug, then unit id.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The slugs of the SDDs in the graph, sorted.
    pub fn slugs(&self) -> &[String] {
        &self.slugs
    }

    /// Every edge, dependent → dependency, sorted.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The units `n` depends on directly, sorted.
    pub fn dependencies(&self, n: &Node) -> Vec<&Node> {
        self.index(n)
            .map(|i| self.deps[i].iter().map(|&d| &self.nodes[d]).collect())
            .unwrap_or_default()
    }

    /// The units that depend on `n` directly, sorted.
    pub fn dependents(&self, n: &Node) -> Vec<&Node> {
        self.index(n)
            .map(|i| self.dependents[i].iter().map(|&d| &self.nodes[d]).collect())
            .unwrap_or_default()
    }

    /// The status `n`'s SDD declared for it when the graph was built.
    pub fn status(&self, n: &Node) -> Option<&str> {
        self.index(n).map(|i| self.status[i].as_str())
    }

    /// `n`'s `depends_on`, each entry classified, in listed order.
    pub fn entries(&self, n: &Node) -> Option<&[(String, Entry)]> {
        let i = self.index(n)?;
        self.entries
            .binary_search_by_key(&i, |(j, _)| *j)
            .ok()
            .map(|k| self.entries[k].1.as_slice())
    }

    /// The units `n` waits on through its `depends_on` entry `entry`, a
    /// pin: the units on the authority's `Producer:` line. `None` when the
    /// entry is no pin `n` may wait on, or its authority does not name
    /// units of its SDD alone.
    pub fn producers(&self, n: &Node, entry: &str) -> Option<Vec<&Node>> {
        let i = self.index(n)?;
        let k = self
            .pins
            .binary_search_by(|p| (p.unit, p.entry.as_str()).cmp(&(i, entry)))
            .ok()?;
        let pin = &self.pins[k];
        (pin.whole && !pin.producers.is_empty())
            .then(|| pin.producers.iter().map(|&p| &self.nodes[p]).collect())
    }

    /// A resolver for [`model::unit_view_with`](crate::model::unit_view_with) and [`claims::plan_with`]:
    /// `n`'s pin entries answered with their producers and the statuses
    /// the graph was built with.
    ///
    /// [`claims::plan_with`]: crate::claims::plan_with
    pub fn resolver<'a>(
        &'a self,
        n: &'a Node,
    ) -> impl Fn(&str) -> Option<Vec<(String, String)>> + 'a {
        move |entry: &str| {
            let producers = self.producers(n, entry)?;
            Some(
                producers
                    .into_iter()
                    .map(|p| {
                        let status = self.status(p).unwrap_or("Draft").to_string();
                        (p.to_string(), status)
                    })
                    .collect(),
            )
        }
    }

    /// Every `depends_on` entry that leads nowhere, by SDD, unit and entry.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// The slugs held by several SDDs, whose units the graph leaves out.
    pub fn clashes(&self) -> &[Clash] {
        &self.clashes
    }

    /// Kahn's order over the graph, with each unit's transitive
    /// dependencies; or, when the graph has a cycle, every cycle and the
    /// units that wait behind one.
    pub fn resolve(&self) -> Result<Resolution, Cycles> {
        let n = self.nodes.len();
        let mut pending: Vec<usize> = self.deps.iter().map(Vec::len).collect();
        let mut ready: BTreeSet<usize> = (0..n).filter(|&i| pending[i] == 0).collect();
        let mut order: Vec<usize> = Vec::with_capacity(n);
        while let Some(i) = ready.pop_first() {
            order.push(i);
            for &m in &self.dependents[i] {
                pending[m] -= 1;
                if pending[m] == 0 {
                    ready.insert(m);
                }
            }
        }
        if order.len() < n {
            return Err(self.cycles(&order));
        }

        // closures in order: a unit's dependencies come before it
        let mut closure: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut mark: Vec<usize> = vec![usize::MAX; n];
        for &i in &order {
            let mut set: Vec<usize> = Vec::new();
            for &d in &self.deps[i] {
                for &x in std::iter::once(&d).chain(closure[d].iter()) {
                    if mark[x] != i {
                        mark[x] = i;
                        set.push(x);
                    }
                }
            }
            set.sort_unstable();
            closure[i] = set;
        }

        Ok(Resolution {
            nodes: self.nodes.clone(),
            deps: self.deps.clone(),
            dependents: self.dependents.clone(),
            order,
            closure,
        })
    }

    /// What Kahn's algorithm left — the units on a cycle or behind one —
    /// split into Tarjan's strongly connected components.
    fn cycles(&self, order: &[usize]) -> Cycles {
        let n = self.nodes.len();
        let mut left = vec![true; n];
        for &i in order {
            left[i] = false;
        }
        let mut cycles: Vec<Vec<Node>> = Vec::new();
        let mut on_cycle = vec![false; n];
        let mut components = tarjan(&self.deps, &left);
        components.sort_by_key(|c| c[0]);
        for comp in components {
            let looped = comp.len() > 1 || self.deps[comp[0]].contains(&comp[0]);
            if !looped {
                continue;
            }
            let mut inside = vec![false; n];
            for &i in &comp {
                inside[i] = true;
                on_cycle[i] = true;
            }
            let path = closing_path(comp[0], &self.deps, &inside);
            cycles.push(path.into_iter().map(|i| self.nodes[i].clone()).collect());
        }
        let waiting = (0..n)
            .filter(|&i| left[i] && !on_cycle[i])
            .map(|i| self.nodes[i].clone())
            .collect();
        Cycles { cycles, waiting }
    }

    /// The SDD graph the unit edges imply: S → T when a unit of S depends
    /// on a unit of T (S ≠ T), with the edges behind it. Mutual pairs are
    /// allowed; only the unit graph must be acyclic. Sorted by (S, T).
    pub fn sdd_edges(&self) -> Vec<SddEdge> {
        let mut out: Vec<SddEdge> = Vec::new();
        for e in &self.edges {
            if e.from.slug == e.to.slug {
                continue;
            }
            let at = match out
                .iter()
                .position(|s| s.from == e.from.slug && s.to == e.to.slug)
            {
                Some(at) => at,
                None => {
                    out.push(SddEdge {
                        from: e.from.slug.clone(),
                        to: e.to.slug.clone(),
                        pins: Vec::new(),
                        units: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let s = &mut out[at];
            if let Some(via) = &e.via
                && !s.pins.contains(via)
            {
                s.pins.push(via.clone());
            }
            s.units.push(e.clone());
        }
        for s in &mut out {
            s.pins.sort();
        }
        out.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        out
    }

    /// The SDDs `slug` depends on, directly or transitively, sorted; never
    /// `slug` itself.
    pub fn upstream(&self, slug: &str) -> Vec<String> {
        let edges = self.sdd_edges();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = vec![slug];
        while let Some(s) = stack.pop() {
            for e in edges.iter().filter(|e| e.from == s) {
                if e.to != slug && seen.insert(e.to.as_str()) {
                    stack.push(&e.to);
                }
            }
        }
        seen.into_iter().map(str::to_string).collect()
    }

    /// The graph as `vitrify locate --graph --json` shows it: the SDDs, the
    /// SDD edges with their pins and unit edges, the unit edges, and the
    /// order — or, with a cycle, the cycles and the units waiting.
    pub fn view(&self) -> GraphView {
        let (order, cycles, waiting) = match self.resolve() {
            Ok(r) => (
                r.order().into_iter().map(UnitRef::of).collect(),
                Vec::new(),
                Vec::new(),
            ),
            Err(c) => (
                Vec::new(),
                c.cycles
                    .iter()
                    .map(|p| p.iter().map(UnitRef::of).collect())
                    .collect(),
                c.waiting.iter().map(UnitRef::of).collect(),
            ),
        };
        GraphView {
            sdds: self.slugs.clone(),
            sdd_edges: self
                .sdd_edges()
                .iter()
                .map(|s| SddEdgeView {
                    from: s.from.clone(),
                    to: s.to.clone(),
                    pins: s.pins.clone(),
                    units: s.units.iter().map(EdgeView::of).collect(),
                })
                .collect(),
            unit_edges: self.edges.iter().map(EdgeView::of).collect(),
            order,
            cycles,
            waiting,
        }
    }
}

/// What a pin's authority leads to: the nodes its `Producer:` line names,
/// and the ids it names that are no unit of the authority's SDD.
type Producers = Result<(Vec<usize>, Vec<String>), PinProblem>;

/// Follow `pin` to its authority among the repository's SDDs (`by_slug`).
fn authority(
    pin: &Pin,
    by_slug: &[(&str, Vec<&Instance>)],
    index: &dyn Fn(&str, &str) -> Option<usize>,
) -> Result<Producers, CoreError> {
    let sdd = match by_slug.iter().find(|(s, _)| *s == pin.slug) {
        None => return Ok(Err(PinProblem::NoSdd)),
        Some((_, insts)) => match insts.as_slice() {
            [one] => *one,
            _ => return Ok(Err(PinProblem::SharedSlug)),
        },
    };
    let Some(c) = contract::find(&sdd.handle(), &pin.contract)? else {
        return Ok(Err(PinProblem::NoContract));
    };
    let producer = contract::field(&c.text, "Producer").unwrap_or_default();
    let ids = if contract::is_external(&producer) {
        Vec::new()
    } else {
        contract::unit_ids(&producer)
    };
    if ids.is_empty() {
        return Ok(Err(PinProblem::NoProducers));
    }
    let mut known: Vec<usize> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for id in ids {
        match index(&pin.slug, &id) {
            Some(i) => known.push(i),
            None => unknown.push(id),
        }
    }
    known.sort_unstable();
    known.dedup();
    Ok(Ok((known, unknown)))
}

/// Tarjan's strongly connected components of the subgraph on the nodes
/// `left` marks, iteratively (no recursion, so no depth limit). Each
/// component sorted.
fn tarjan(deps: &[Vec<usize>], left: &[bool]) -> Vec<Vec<usize>> {
    let n = deps.len();
    let mut index: Vec<Option<usize>> = vec![None; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut out: Vec<Vec<usize>> = Vec::new();
    for root in (0..n).filter(|&i| left[i]) {
        if index[root].is_some() {
            continue;
        }
        // (node, next successor to look at)
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = Some(next);
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&(v, at)) = calls.last() {
            if let Some(&w) = deps[v].get(at) {
                if let Some(top) = calls.last_mut() {
                    top.1 += 1;
                }
                if !left[w] {
                    continue;
                }
                match index[w] {
                    None => {
                        index[w] = Some(next);
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        calls.push((w, 0));
                    }
                    Some(iw) if on_stack[w] => low[v] = low[v].min(iw),
                    Some(_) => {}
                }
                continue;
            }
            calls.pop();
            if let Some(&(p, _)) = calls.last() {
                low[p] = low[p].min(low[v]);
            }
            if Some(low[v]) == index[v] {
                let mut comp: Vec<usize> = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                out.push(comp);
            }
        }
    }
    out
}

/// The path that closes a cycle through `start` inside the component
/// `inside` marks: a depth-first search from `start`, successors in order,
/// until an edge leads back to it. `start` is first and last.
fn closing_path(start: usize, deps: &[Vec<usize>], inside: &[bool]) -> Vec<usize> {
    let mut visited = vec![false; deps.len()];
    visited[start] = true;
    // (node, next successor to look at); the stack is the path
    let mut path: Vec<(usize, usize)> = vec![(start, 0)];
    while let Some(&(v, at)) = path.last() {
        let Some(&w) = deps[v].get(at) else {
            path.pop();
            continue;
        };
        if let Some(top) = path.last_mut() {
            top.1 += 1;
        }
        if w == start {
            let mut out: Vec<usize> = path.iter().map(|(v, _)| *v).collect();
            out.push(start);
            return out;
        }
        if inside[w] && !visited[w] {
            visited[w] = true;
            path.push((w, 0));
        }
    }
    // invariant: a component of a cycle holds a path back to each member
    vec![start, start]
}

/// Every cycle of a graph that has no order, and the units that only wait
/// behind one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycles {
    /// Each cycle as the path that closes it, from its smallest unit back
    /// to it; ordered by that unit.
    pub cycles: Vec<Vec<Node>>,
    /// Units on no cycle that depend, transitively, on one; sorted.
    pub waiting: Vec<Node>,
}

// ----------------------------------------------------------- resolution

/// An acyclic graph resolved: Kahn's order and each unit's transitive
/// dependencies, computed once.
#[derive(Debug, Clone)]
pub struct Resolution {
    nodes: Vec<Node>,
    deps: Vec<Vec<usize>>,
    dependents: Vec<Vec<usize>>,
    order: Vec<usize>,
    closure: Vec<Vec<usize>>,
}

impl Resolution {
    fn index(&self, n: &Node) -> Option<usize> {
        self.nodes.binary_search(n).ok()
    }

    /// Every unit, each after all it depends on; of the units ready at
    /// once, the smallest by slug, then unit id, first.
    pub fn order(&self) -> Vec<&Node> {
        self.order.iter().map(|&i| &self.nodes[i]).collect()
    }

    /// The units `n` depends on, directly or transitively, sorted.
    pub fn closure(&self, n: &Node) -> Vec<&Node> {
        self.index(n)
            .map(|i| self.closure[i].iter().map(|&d| &self.nodes[d]).collect())
            .unwrap_or_default()
    }

    /// Whether `n` depends on `on`, directly or transitively.
    pub fn depends_on(&self, n: &Node, on: &Node) -> bool {
        match (self.index(n), self.index(on)) {
            (Some(i), Some(j)) => self.closure[i].binary_search(&j).is_ok(),
            _ => false,
        }
    }

    /// The units that depend on `n`, directly or transitively, sorted.
    pub fn dependents_closure(&self, n: &Node) -> Vec<&Node> {
        let Some(j) = self.index(n) else {
            return Vec::new();
        };
        (0..self.nodes.len())
            .filter(|&i| self.closure[i].binary_search(&j).is_ok())
            .map(|i| &self.nodes[i])
            .collect()
    }

    /// Critical-path priorities over the units `finished` does not hold
    /// (normally: every unit not Verified). A unit's priority is Hu's
    /// label: the number of unfinished units on the longest chain from it
    /// through its unfinished dependents, itself included — 1 + the highest
    /// priority of an unfinished unit, in any SDD, that depends on it
    /// directly; 1 when none does.
    pub fn priorities(&self, finished: impl Fn(&Node) -> bool) -> Priorities {
        self.priorities_weighted(finished, |_| 1)
    }

    /// The same labels with each unit weighing `weight(unit)` instead of 1
    /// — a unit's priority is its weight plus the highest priority of an
    /// unfinished dependent, so the longest chain is counted by estimated
    /// duration, a longer job counting as a chain of unit jobs. Weights are
    /// positive integers (milliseconds, seconds — one unit throughout; 0 is
    /// taken as 1), and the caller supplies the default for a unit it knows
    /// nothing about. With every weight 1 this is [`Resolution::priorities`].
    /// Order and critical path follow the weighted labels; ties still break
    /// by slug, then unit id.
    pub fn priorities_weighted(
        &self,
        finished: impl Fn(&Node) -> bool,
        weight: impl Fn(&Node) -> u64,
    ) -> Priorities {
        let n = self.nodes.len();
        let open: Vec<bool> = self.nodes.iter().map(|x| !finished(x)).collect();
        let mut label: Vec<u64> = vec![0; n];
        for &i in self.order.iter().rev() {
            if !open[i] {
                continue;
            }
            let above = self.dependents[i]
                .iter()
                .filter(|&&v| open[v])
                .map(|&v| label[v])
                .max()
                .unwrap_or(0);
            label[i] = above.saturating_add(weight(&self.nodes[i]).max(1));
        }
        let first = |a: usize, b: usize| Reverse(label[a]).cmp(&Reverse(label[b])).then(a.cmp(&b));
        let mut ranked: Vec<usize> = (0..n).filter(|&i| open[i]).collect();
        ranked.sort_by(|&a, &b| first(a, b));
        let mut critical: Vec<usize> = Vec::new();
        let mut at = ranked.first().copied();
        while let Some(i) = at {
            critical.push(i);
            at = self.dependents[i]
                .iter()
                .copied()
                .filter(|&v| open[v])
                .min_by(|&a, &b| first(a, b));
        }
        Priorities {
            nodes: self.nodes.clone(),
            label,
            ranked,
            critical,
        }
    }

    /// The units `n` depends on directly, sorted.
    pub fn dependencies(&self, n: &Node) -> Vec<&Node> {
        self.index(n)
            .map(|i| self.deps[i].iter().map(|&d| &self.nodes[d]).collect())
            .unwrap_or_default()
    }
}

/// Hu's labels over the unfinished units, and the critical path they give.
#[derive(Debug, Clone)]
pub struct Priorities {
    nodes: Vec<Node>,
    /// 0 for a finished unit.
    label: Vec<u64>,
    ranked: Vec<usize>,
    critical: Vec<usize>,
}

impl Priorities {
    /// `n`'s priority; `None` when it is finished or no unit of the graph.
    pub fn of(&self, n: &Node) -> Option<usize> {
        self.of_weighted(n)
            .map(|w| usize::try_from(w).unwrap_or(usize::MAX))
    }

    /// `n`'s priority as the weighted labels carry it (the same number as
    /// [`Priorities::of`] when every weight is 1).
    pub fn of_weighted(&self, n: &Node) -> Option<u64> {
        let i = self.nodes.binary_search(n).ok()?;
        (self.label[i] > 0).then_some(self.label[i])
    }

    /// Dispatch order between two units: priority descending, then slug,
    /// then unit id. A finished or unknown unit ranks last.
    pub fn cmp(&self, a: &Node, b: &Node) -> Ordering {
        Reverse(self.of_weighted(a).unwrap_or(0))
            .cmp(&Reverse(self.of_weighted(b).unwrap_or(0)))
            .then_with(|| a.cmp(b))
    }

    /// Every unfinished unit with its priority, in dispatch order.
    pub fn ranked(&self) -> Vec<(&Node, usize)> {
        self.ranked
            .iter()
            .map(|&i| {
                (
                    &self.nodes[i],
                    usize::try_from(self.label[i]).unwrap_or(usize::MAX),
                )
            })
            .collect()
    }

    /// [`Priorities::ranked`] with the weighted labels.
    pub fn ranked_weighted(&self) -> Vec<(&Node, u64)> {
        self.ranked
            .iter()
            .map(|&i| (&self.nodes[i], self.label[i]))
            .collect()
    }

    /// The critical path: from the highest-priority unfinished unit, each
    /// step to the unfinished dependent with the highest priority (ties by
    /// slug, then unit id). Its length is the highest priority.
    pub fn critical_path(&self) -> Vec<&Node> {
        self.critical.iter().map(|&i| &self.nodes[i]).collect()
    }
}

// ------------------------------------------------------------- SDD graph

/// One edge of the derived SDD graph: units of `from` depend on units of
/// `to`, through the pins of `from` listed (and, for a pin that names its
/// own SDD's contract, directly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SddEdge {
    pub from: String,
    pub to: String,
    /// The pins of `from` the unit edges run through, sorted.
    pub pins: Vec<String>,
    /// The unit edges behind it, sorted.
    pub units: Vec<Edge>,
}

/// A unit as the graph view names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitRef {
    pub sdd: String,
    pub unit: String,
}

impl UnitRef {
    fn of(n: &Node) -> UnitRef {
        UnitRef {
            sdd: n.slug.clone(),
            unit: n.unit.clone(),
        }
    }
}

/// A unit edge as the graph view shows it: `from` depends on `to`, `via`
/// the pin it runs through, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeView {
    pub from: UnitRef,
    pub to: UnitRef,
    pub via: Option<String>,
}

impl EdgeView {
    fn of(e: &Edge) -> EdgeView {
        EdgeView {
            from: UnitRef::of(&e.from),
            to: UnitRef::of(&e.to),
            via: e.via.clone(),
        }
    }
}

/// An SDD edge as the graph view shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SddEdgeView {
    pub from: String,
    pub to: String,
    pub pins: Vec<String>,
    pub units: Vec<EdgeView>,
}

/// The whole graph, serializable field by field in this order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphView {
    pub sdds: Vec<String>,
    pub sdd_edges: Vec<SddEdgeView>,
    pub unit_edges: Vec<EdgeView>,
    /// Kahn's order; empty when the graph has a cycle.
    pub order: Vec<UnitRef>,
    /// Each cycle as the path that closes it.
    pub cycles: Vec<Vec<UnitRef>>,
    /// Units that wait behind a cycle.
    pub waiting: Vec<UnitRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring of `n` nodes, each depending on the next.
    fn ring(n: usize) -> Vec<Vec<usize>> {
        (0..n).map(|i| vec![(i + 1) % n]).collect()
    }

    #[test]
    fn tarjan_and_the_closing_path_need_no_recursion() {
        let n = 200_000;
        let deps = ring(n);
        let comps = tarjan(&deps, &vec![true; n]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].len(), n);
        let path = closing_path(0, &deps, &vec![true; n]);
        assert_eq!(path.len(), n + 1);
        assert_eq!((path[0], path[n]), (0, 0));
    }

    #[test]
    fn tarjan_splits_components_and_leaves_out_what_is_not_left() {
        // 0 ⇄ 1, 2 → 0, 3 → 3, 4 ⇄ 5 (5 not left)
        let deps = vec![vec![1], vec![0], vec![0], vec![3], vec![5], vec![4]];
        let left = [true, true, true, true, true, false];
        let mut comps = tarjan(&deps, &left);
        comps.sort();
        assert_eq!(comps, [vec![0, 1], vec![2], vec![3], vec![4]]);
        assert_eq!(
            closing_path(3, &deps, &[false, false, false, true, false, false]),
            [3, 3]
        );
    }
}
