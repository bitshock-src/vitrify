//! `vitrify lint` — the mechanical authoring checks L1–L12.
//! Named findings `{check, subject, cause, remediation}`; exit 0 clean / 2 on
//! any finding. The remediation is the edit or the command that resolves it.
//!
//! L1–L8 hold one SDD to itself. L9–L12 hold it to what lies outside it and
//! that it names: the contracts of other SDDs it pins (L9), its
//! own changelog (L10), the TDD it is derived from (L11), and the units of
//! other SDDs it waits on through pins (L12).

use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::audit;
use vitrify_core::config::Overrides;
use vitrify_core::contract::State;
use vitrify_core::graph::{self, Entry, Node, UnitGraph, render_path};
use vitrify_core::model::{self, Instance, SddHandle, Shape, is_empty_marker};
use vitrify_core::{changelog, contract, journal, questions, readset, tdd};

use crate::out::{J, emit};

/// The checks, as the clean line names them.
pub const CHECKS: &str = "L1–L12";

pub struct Finding {
    pub check: &'static str,
    pub subject: String,
    pub cause: String,
    /// The edit or the command that resolves it.
    pub remediation: String,
}

fn f(
    check: &'static str,
    subject: impl Into<String>,
    cause: impl Into<String>,
    remediation: impl Into<String>,
) -> Finding {
    Finding {
        check,
        subject: subject.into(),
        cause: cause.into(),
        remediation: remediation.into(),
    }
}

impl Finding {
    /// `[L4] C-001: <cause> → <remediation>`
    pub fn line(&self) -> String {
        format!(
            "[{}] {}: {} → {}",
            self.check, self.subject, self.cause, self.remediation
        )
    }
}

pub struct Flags {
    pub sdd: Option<String>,
    /// Every SDD of the repository.
    pub all: bool,
    /// Lint an SDD that is being built or has been: every check but L7.
    pub built: bool,
    pub json: bool,
}

fn findings_json(findings: &[Finding]) -> Vec<(String, J)> {
    let jf: Vec<J> = findings
        .iter()
        .map(|x| {
            J::Obj(vec![
                ("check".into(), J::s(x.check)),
                ("subject".into(), J::s(x.subject.clone())),
                ("cause".into(), J::s(x.cause.clone())),
                ("remediation".into(), J::s(x.remediation.clone())),
            ])
        })
        .collect();
    vec![
        ("clean".into(), J::Bool(findings.is_empty())),
        ("findings".into(), J::Arr(jf)),
    ]
}

pub fn run(flags: Flags) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, flags.sdd.as_deref(), flags.all, &super::cwd())?;
    let budget = super::load_config(&Overrides::default())?
        .lint
        .readset_budget_bytes;
    let lint = |sdd: &SddHandle| -> Result<Vec<Finding>, CoreError> {
        let inst = Instance::load(sdd.clone())?;
        if flags.built {
            pre_run_findings(&inst, &root, budget)
        } else {
            check_all(&inst, &root, budget)
        }
    };

    if !flags.all {
        let findings = lint(&sdds[0])?;
        if flags.json {
            emit(&J::Obj(findings_json(&findings)).dumps_indent2());
        } else {
            if findings.is_empty() {
                emit(&format!("lint clean ({CHECKS})"));
            }
            for x in &findings {
                emit(&x.line());
            }
        }
        return Ok(if findings.is_empty() { 0 } else { 2 });
    }

    // every SDD: one line per finding, each under its SDD's name
    let mut reports: Vec<J> = Vec::new();
    let code = super::each(&root, &sdds, |sdd| {
        let findings = lint(sdd)?;
        let name = super::label(sdd, &root);
        if flags.json {
            let mut pairs = vec![
                ("sdd".into(), J::s(name)),
                ("root".into(), J::s(super::shown(&root, &sdd.root))),
            ];
            pairs.extend(findings_json(&findings));
            reports.push(J::Obj(pairs));
        } else {
            if findings.is_empty() {
                emit(&format!("{name}: lint clean ({CHECKS})"));
            }
            for x in &findings {
                emit(&format!("{name}: {}", x.line()));
            }
        }
        Ok(if findings.is_empty() { 0 } else { 2 })
    });
    if flags.json {
        emit(&J::Arr(reports).dumps_indent2());
    }
    Ok(code)
}

/// `U-001`; `U-001 and U-002`; `U-001, U-002 and U-003`.
fn listed(ids: &[String]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

pub fn check_all(inst: &Instance, root: &Path, budget: i64) -> Result<Vec<Finding>, CoreError> {
    let mut findings: Vec<Finding> = Vec::new();
    let units = &inst.units;
    let unit_ids: Vec<&str> = units.iter().map(|u| u.unit.as_str()).collect();
    // the SDD as a `--sdd` value
    let at = super::shown(root, &inst.root);
    let handle = inst.handle();

    // L1 — DAG acyclic; every depends_on entry is a unit of this SDD or —
    // from schema 2.1 — a pin of it the unit also consumes
    for ue in graph::classify(inst)? {
        for (d, entry) in &ue.entries {
            if let Entry::Invalid(p) = entry {
                findings.push(f(
                    "L1",
                    ue.unit.clone(),
                    p.cause(&ue.unit, d),
                    p.remediation(&ue.unit, d),
                ));
            }
        }
    }
    if let Some(cycle) = find_cycle(units) {
        findings.push(f(
            "L1",
            cycle.join(" → "),
            "depends_on graph contains a cycle",
            "remove one of the cycle's depends_on edges",
        ));
    }

    // L2 — ownership exclusive + exhaustive over scope_roots
    let owns: Vec<(String, Vec<String>)> = units
        .iter()
        .map(|u| (u.unit.clone(), u.meta.get_list("owns")))
        .collect();
    for (i, (ua, gsa)) in owns.iter().enumerate() {
        for (ub, gsb) in owns.iter().skip(i + 1) {
            for ga in gsa {
                for gb in gsb {
                    if globs_overlap(ga, gb) {
                        findings.push(f(
                            "L2",
                            format!("{ua}/{ub}"),
                            format!("owns globs can match a common path: `{ga}` vs `{gb}`"),
                            format!("narrow {ua}'s or {ub}'s owns so no path matches both"),
                        ));
                    }
                }
            }
        }
    }
    let core_text = model::read_text(&inst.file)?;
    let unmanaged = unmanaged_globs(&core_text);
    let tests = audit::tests_globs(&core_text);
    let tests_counted = inst.schema().tests_list();
    if tests_counted {
        let scope_roots = inst.meta.get_list("scope_roots");
        for t in &tests {
            if !audit::tests_entry_in_scope(t, &scope_roots) {
                findings.push(f(
                    "L2",
                    t.clone(),
                    format!("tests: entry `{t}` lies outside scope_roots"),
                    format!("narrow `{t}` to a path under scope_roots, or remove it from tests:"),
                ));
            }
        }
        for t in &tests {
            for u in &unmanaged {
                if globs_overlap(t, u) {
                    findings.push(f(
                        "L2",
                        t.clone(),
                        format!(
                            "tests: entry `{t}` and unmanaged: entry `{u}` can match a common path"
                        ),
                        "narrow one of them so no path is both a test and unmanaged",
                    ));
                }
            }
        }
    } else if !tests.is_empty() {
        let version = inst.schema().as_str();
        findings.push(f(
            "L2",
            "tests:",
            format!(
                "§6 lists tests:, and this SDD declares schema_version {version} — a tests: list is schema 2.2"
            ),
            "set schema_version: 2.2 in the SDD's core front matter (a PATCH change entry), or remove the tests: list",
        ));
    }
    // the governed tree: what git would commit (its ignore rules decide,
    // whatever the ecosystem); outside a repository, every file on disk
    let scope_files = super::governed_files(root, inst)
        .unwrap_or_else(|| audit::walk_scope(root, &inst.meta.get_list("scope_roots")));
    for file in &scope_files {
        let owned = owns
            .iter()
            .any(|(_, gs)| gs.iter().any(|g| audit::matches(file, g)))
            || unmanaged.iter().any(|g| audit::matches(file, g))
            || (tests_counted && tests.iter().any(|g| audit::matches(file, g)));
        if !owned {
            findings.push(f(
                "L2",
                file.clone(),
                "file under scope_roots is neither owned nor unmanaged",
                "add a glob that matches it to one unit's owns, or list it under unmanaged: in System Layout & Ownership",
            ));
        }
    }

    // L3 — parallel groups mutually independent (no intra-group deps)
    for u in units {
        let group = u.meta.get_str_or("parallel_group", "");
        if is_empty_marker(group.trim()) {
            continue;
        }
        for d in u.meta.get_list("depends_on") {
            if let Some(dep) = units.iter().find(|x| x.unit == d)
                && dep.meta.get_str_or("parallel_group", "") == group
            {
                findings.push(f(
                    "L3",
                    u.unit.clone(),
                    format!("depends on {d} inside its own parallel group {group}"),
                    format!(
                        "move {} or {d} to another parallel_group, or drop the dependency",
                        u.unit
                    ),
                ));
            }
        }
    }

    // L4 — contract reference integrity
    for u in units {
        for cid in u.meta.get_list("consumes") {
            if model::contract_text(&handle, &cid)?.is_none() {
                findings.push(f(
                    "L4",
                    u.unit.clone(),
                    format!("consumes unresolvable contract {cid}"),
                    format!(
                        "write contract {cid}, or remove it from {}'s consumes",
                        u.unit
                    ),
                ));
            }
        }
    }
    for (cid, cref) in audit::contract_refs(inst)? {
        let mut refs: Vec<&String> = Vec::new();
        if let Some(p) = &cref.producer {
            refs.push(p);
        }
        refs.extend(cref.consumers.iter());
        for uid in refs {
            if !unit_ids.contains(&uid.as_str()) {
                findings.push(f(
                    "L4",
                    cid.clone(),
                    format!("references nonexistent unit {uid}"),
                    format!(
                        "make {cid}'s Producer and Consumers lines name only units the SDD has"
                    ),
                ));
            }
        }
    }
    // … and both ways round: a contract's Consumers line names exactly the
    // units that list it in `consumes` (its producer apart, which may
    // consume its own contract)
    let contracts = contract::list(&handle)?;
    for c in &contracts {
        if let Some((header_only, consumes_only)) = contract::consumer_mismatch(c, units) {
            let mut parts: Vec<String> = Vec::new();
            if !header_only.is_empty() {
                parts.push(format!(
                    "names {}, which {} not list it in `consumes`",
                    listed(&header_only),
                    if header_only.len() == 1 { "does" } else { "do" }
                ));
            }
            if !consumes_only.is_empty() {
                parts.push(format!(
                    "omits {}, which {} it",
                    listed(&consumes_only),
                    if consumes_only.len() == 1 {
                        "consumes"
                    } else {
                        "consume"
                    }
                ));
            }
            findings.push(f(
                "L4",
                c.id.clone(),
                format!("its Consumers line {}", parts.join(" and ")),
                format!(
                    "make {}'s Consumers line name exactly the units that list it in consumes",
                    c.id
                ),
            ));
        }
    }

    // L5 — reading-set budget
    let peers = super::peers(root);
    for u in units {
        match readset::resolve(inst, &u.unit, &peers) {
            Ok(items) => {
                let total: i64 = items.iter().map(|i| i.bytes as i64).sum();
                if total > budget {
                    findings.push(f(
                        "L5",
                        u.unit.clone(),
                        format!("reading set totals {total} bytes, over budget {budget}"),
                        format!(
                            "split {} or shrink what it reads (vitrify readset --unit {} --sdd {at} lists it)",
                            u.unit, u.unit
                        ),
                    ));
                }
            }
            // a missing consumed contract is L4's finding, not L5's — and a
            // pin that does not resolve, L9's — unless something else is
            // wrong with the set as well
            Err(e)
                if e.to_string().lines().all(|l| {
                    l.contains("consumed contract missing") || l.contains(readset::PIN_PROBLEM)
                }) => {}
            Err(e) => findings.push(f(
                "L5",
                u.unit.clone(),
                format!(
                    "reading set does not resolve: {}",
                    e.to_string().replace('\n', "; ")
                ),
                format!(
                    "make every path {}'s reading set names exist (vitrify readset --unit {} --sdd {at})",
                    u.unit, u.unit
                ),
            )),
        }
    }

    // L6 — requirement coverage in the Verification Covers column
    for u in units {
        let text = unit_text(inst, u)?;
        let declared = declared_requirements(&text, &u.unit);
        let covers = covers_tokens(&text);
        let covers_all = covers.iter().any(|t| t.eq_ignore_ascii_case("all"));
        for r in &declared {
            let short = format!("R{r}");
            let full = format!("{}.R{r}", u.unit);
            if !covers_all && !covers.contains(&short) && !covers.contains(&full) {
                findings.push(f(
                    "L6",
                    u.unit.clone(),
                    format!("requirement {full} not covered by any Verification row"),
                    format!(
                        "add a Verification row whose Covers column names {short}, or remove {full}"
                    ),
                ));
            }
        }
    }

    // L7 — pre-build state sanity. An SDD with a journal is being built, or
    // has been: its build state is lint --built's to judge.
    let built = journal::read(&handle)?.1;
    let l7 = |unit: &str, cause: &str, clear: &str| {
        let remediation = if built {
            format!("the SDD is being built: vitrify lint --built --sdd {at}")
        } else {
            format!("{clear} in {unit}'s entry: the build sets it")
        };
        f("L7", unit, cause, remediation)
    };
    for u in units {
        let status = u.meta.get_str_or("status", "Draft");
        if status != "Draft" && status != "Ready" {
            findings.push(l7(
                &u.unit,
                &format!("status {status} is beyond Ready before the build"),
                "set status to Draft or Ready",
            ));
        }
        if !is_empty_marker(u.meta.get_str_or("claimed_by", "").trim()) {
            findings.push(l7(
                &u.unit,
                "claimed_by set before the build",
                "set claimed_by to —",
            ));
        }
        if !is_empty_marker(u.meta.get_str_or("claim_expires", "").trim()) {
            findings.push(l7(
                &u.unit,
                "claim_expires set before the build",
                "set claim_expires to —",
            ));
        }
        if u.meta.get_map("trace").is_some_and(|m| !m.is_empty()) {
            findings.push(l7(
                &u.unit,
                "trace recorded before the build",
                "remove the trace map",
            ));
        }
    }

    // L8 — no OPEN(Q-###) inside any Ready unit's region; and whatever its
    // status, a marker names a question the Decision Log holds
    let asked: Vec<String> = questions::of(&handle)?.into_iter().map(|q| q.id).collect();
    for u in units {
        let text = unit_text(inst, u)?;
        let markers = questions::markers(&text);
        if u.meta.get_str_or("status", "Draft") == "Ready" && text.contains("OPEN(Q-") {
            let named = if markers.is_empty() {
                "the question".to_string()
            } else {
                listed(&markers)
            };
            findings.push(f(
                "L8",
                u.unit.clone(),
                "OPEN(Q-###) marker inside a Ready unit's region",
                format!(
                    "resolve {named} in the Decision Log and remove the marker, or set {} back to Draft",
                    u.unit
                ),
            ));
        }
        for q in markers {
            if !asked.contains(&q) {
                findings.push(f(
                    "L8",
                    u.unit.clone(),
                    format!("OPEN({q}) names a question with no entry in the Decision Log"),
                    format!("add {q} to the Decision Log, or correct the marker"),
                ));
            }
        }
    }

    // L9 — an external contract is its authority's: a pin's hash is the
    // hash of the body it stands for, and the authority it names is there
    // to be found
    for (subject, cause, remediation) in pin_findings(inst, &peers, &at)? {
        findings.push(f("L9", subject, cause, remediation));
    }

    // L10 — the front matter's version and date are the last change
    // entry's, and change entries are in id order
    let entries = changelog::entries(&handle)?;
    let last = entries.iter().max_by_key(|e| e.number);
    for (subject, cause) in changelog::findings(
        &entries,
        inst.meta.get_str_or("version", ""),
        inst.meta.get_str_or("updated", ""),
    ) {
        let remediation = match (subject.as_str(), last) {
            ("version" | "updated", Some(last)) => format!(
                "set version and updated in the front matter to {}'s ({} and {})",
                last.id,
                last.to.as_deref().unwrap_or("its target version"),
                last.date.as_deref().unwrap_or("its date")
            ),
            _ => format!(
                "give {subject} the next unused id: change entries are appended in id order, newest last"
            ),
        };
        findings.push(f("L10", subject, cause, remediation));
    }
    // … and a single file's entries are where they are read
    for (id, hashes) in changelog::misplaced(&handle)? {
        let (cause, remediation) = changelog::misplaced_finding(&id, hashes);
        findings.push(f("L10", id, cause, remediation));
    }

    // L11 — the TDD pin: `tdd:` names a file, and an Active SDD pins the
    // current revision of an Approved TDD
    for (subject, cause) in
        tdd::findings(&inst.meta, &inst.file, root, |p| model::read_text(p).ok())
    {
        let remediation = match subject.as_str() {
            "tdd_version" => {
                "re-pin tdd_version to the TDD's Revision in a change entry (vitrify brief change)"
            }
            _ if cause.contains("names no file") => {
                "set tdd: to the TDD's path, from the SDD's directory or the repository root"
            }
            _ => "approve the TDD (Status: Approved), or keep the SDD Draft until it is",
        };
        findings.push(f("L11", subject, cause, remediation));
    }

    // L12 — across SDDs: every pin in depends_on leads to the units that
    // produce its authority, and no cycle runs through pins. Every SDD on
    // such a cycle, or behind one, has a pin in depends_on of its own.
    if graph::classify(inst)?
        .iter()
        .any(|ue| ue.entries.iter().any(|(_, e)| matches!(e, Entry::Pin(_))))
    {
        let unit_graph = super::repo_graph(root, Some(inst))?;
        findings.extend(graph_findings(inst, &unit_graph));
    }

    Ok(findings)
}

/// L12's findings for `inst` in the repository's unit graph: its pins in
/// `depends_on` whose authority names no units of its SDD; every cycle
/// through pins that runs through one of its units, as the path that closes
/// it; and its units that only wait behind such a cycle. A cycle through
/// pins crosses SDDs, or takes an edge that only a pin makes — a pin of the
/// SDD's own contract, too. A cycle of the SDD's units alone is L1's.
pub fn graph_findings(inst: &Instance, unit_graph: &UnitGraph) -> Vec<Finding> {
    let slug = graph::slug_of(inst);
    let mut findings: Vec<Finding> = unit_graph
        .problems()
        .iter()
        .filter(|p| p.slug == slug && p.check() == "L12")
        .map(|p| f("L12", p.unit.clone(), p.cause(), p.remediation()))
        .collect();
    let Err(cycles) = unit_graph.resolve() else {
        return findings;
    };
    let crosses = |path: &[Node]| path.iter().any(|n| n.slug != path[0].slug);
    let through_pins: Vec<&Vec<Node>> = cycles
        .cycles
        .iter()
        .filter(|path| crosses(path) || path.windows(2).any(|w| pin_only(unit_graph, &w[0], &w[1])))
        .collect();
    for path in &through_pins {
        if path.iter().any(|n| n.slug == slug) {
            let cause = if crosses(path) {
                "depends_on graph across SDDs contains a cycle through pins"
            } else {
                "depends_on graph contains a cycle through a pin of this SDD's own contract"
            };
            findings.push(f(
                "L12",
                render_path(path),
                cause,
                "remove one of the cycle's depends_on edges — a pin a unit only reads stays in consumes alone",
            ));
        }
    }
    for n in cycles.waiting.iter().filter(|n| n.slug == slug) {
        if let Some(path) = through_pins
            .iter()
            .find(|path| waits_on(unit_graph, n, path))
        {
            findings.push(f(
                "L12",
                n.unit.clone(),
                format!("waits behind the cycle {}", render_path(path)),
                "break the cycle its own L12 finding names",
            ));
        }
    }
    findings
}

/// Whether `from` depends on `to` only through a pin: no `depends_on` entry
/// names `to` directly.
fn pin_only(unit_graph: &UnitGraph, from: &Node, to: &Node) -> bool {
    let edges: Vec<_> = unit_graph
        .edges()
        .iter()
        .filter(|e| &e.from == from && &e.to == to)
        .collect();
    !edges.is_empty() && edges.iter().all(|e| e.via.is_some())
}

/// Whether `n` depends, transitively, on a unit of `path`.
fn waits_on(unit_graph: &UnitGraph, n: &Node, path: &[Node]) -> bool {
    let mut seen: Vec<&Node> = Vec::new();
    let mut stack: Vec<&Node> = vec![n];
    while let Some(x) = stack.pop() {
        for d in unit_graph.dependencies(x) {
            if path.contains(d) {
                return true;
            }
            if !seen.contains(&d) {
                seen.push(d);
                stack.push(d);
            }
        }
    }
    false
}

/// The external contracts of `inst` that are not `ok`, as
/// `(contract, cause, remediation)`; `at` is the SDD as a `--sdd` value.
/// Lint reports them as L9, and `vitrify check` as its pin source.
pub fn pin_findings(
    inst: &Instance,
    peers: &contract::Peers,
    at: &str,
) -> Result<Vec<(String, String, String)>, CoreError> {
    Ok(contract::examine(inst, peers)?
        .into_iter()
        .filter(|ext| ext.state.is_finding())
        .map(|ext| {
            let remediation = match ext.state {
                State::Stale => format!("vitrify pin sync --sdd {at}"),
                State::MissingAuthority => format!(
                    "make {}'s Pin: line name an SDD and contract this repository has (vitrify pin list --all)",
                    ext.id
                ),
                _ => format!(
                    "correct {}'s header as its cause says (vitrify schema show, Interface Contracts)",
                    ext.id
                ),
            };
            (ext.id, ext.detail, remediation)
        })
        .collect())
}

/// Findings of every check except L7 — the resume-tolerant subset the
/// orchestrator preflights with on an Active, partially-built SDD.
pub fn pre_run_findings(
    inst: &Instance,
    root: &Path,
    budget: i64,
) -> Result<Vec<Finding>, CoreError> {
    Ok(check_all(inst, root, budget)?
        .into_iter()
        .filter(|x| x.check != "L7")
        .collect())
}

fn find_cycle(units: &[model::Unit]) -> Option<Vec<String>> {
    // DFS with a path stack; returns the first cycle found.
    fn visit(
        uid: &str,
        units: &[model::Unit],
        path: &mut Vec<String>,
        done: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if done.iter().any(|d| d == uid) {
            return None;
        }
        if let Some(pos) = path.iter().position(|p| p == uid) {
            let mut cycle = path[pos..].to_vec();
            cycle.push(uid.to_string());
            return Some(cycle);
        }
        path.push(uid.to_string());
        if let Some(u) = units.iter().find(|u| u.unit == uid) {
            for d in u.meta.get_list("depends_on") {
                if let Some(c) = visit(&d, units, path, done) {
                    return Some(c);
                }
            }
        }
        path.pop();
        done.push(uid.to_string());
        None
    }
    let mut done = Vec::new();
    for u in units {
        if let Some(c) = visit(&u.unit, units, &mut Vec::new(), &mut done) {
            return Some(c);
        }
    }
    None
}

/// Conservative literal/prefix overlap between two owns globs.
fn globs_overlap(a: &str, b: &str) -> bool {
    let lit = |g: &str| -> (String, bool) {
        match g.find(['*', '?', '[']) {
            Some(i) => (g[..i].to_string(), true),
            None => (g.trim_end_matches('/').to_string(), false),
        }
    };
    let (pa, wild_a) = lit(a);
    let (pb, wild_b) = lit(b);
    if !wild_a && !wild_b {
        return pa == pb || pa.starts_with(&format!("{pb}/")) || pb.starts_with(&format!("{pa}/"));
    }
    pa.starts_with(&pb) || pb.starts_with(&pa)
}

fn unmanaged_globs(core_text: &str) -> Vec<String> {
    let sec = model::section(core_text, "System Layout & Ownership");
    let mut out = Vec::new();
    let mut in_block = false;
    for line in sec.lines() {
        if !in_block {
            if line.trim() == "unmanaged:" {
                in_block = true;
            }
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(item) = trimmed.strip_prefix("- ")
            && line.starts_with(|c: char| c.is_whitespace())
        {
            out.push(item.trim().to_string());
        } else {
            break;
        }
    }
    out
}

fn unit_text(inst: &Instance, u: &model::Unit) -> Result<String, CoreError> {
    match inst.shape {
        Shape::Bundle => model::read_text(&u.file),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            Ok(model::find_unit_span(&text, &u.unit)
                .map(|(s, e)| text[s..e].to_string())
                .unwrap_or_default())
        }
    }
}

/// Requirement numbers declared as `U-###.R<n>` in the unit's text.
fn declared_requirements(text: &str, unit_id: &str) -> Vec<u32> {
    let needle = format!("{unit_id}.R");
    let mut out: Vec<u32> = Vec::new();
    let mut search = text;
    while let Some(i) = search.find(&needle) {
        let rest = &search[i + needle.len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u32>()
            && !out.contains(&n)
        {
            out.push(n);
        }
        search = &search[i + needle.len()..];
    }
    out
}

/// First-column tokens of every Verification table row.
fn covers_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let is_heading = line.starts_with("##");
        if is_heading {
            in_section = line
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("Verification");
            continue;
        }
        if !in_section || !line.trim_start().starts_with('|') {
            continue;
        }
        let stripped = line.trim().trim_matches('|');
        let cells: Vec<&str> = stripped.split('|').map(str::trim).collect();
        if cells.is_empty() {
            continue;
        }
        let first = cells[0];
        if first.chars().all(|c| c == '-' || c == ' ' || c == ':')
            || first.eq_ignore_ascii_case("covers")
        {
            continue;
        }
        for tok in first.split([',', '·']) {
            let t = tok.trim().trim_matches('*').trim();
            if t.is_empty() {
                continue;
            }
            // expand ranges like R1–R4 / R1-R4
            if let Some((lo, hi)) = parse_range(t) {
                for n in lo..=hi {
                    tokens.push(format!("R{n}"));
                }
            } else {
                tokens.push(t.to_string());
            }
        }
    }
    tokens
}

/// Parse `R<a>–R<b>` / `R<a>-R<b>` range tokens.
fn parse_range(token: &str) -> Option<(u32, u32)> {
    for sep in ['\u{2013}', '-'] {
        if let Some((a, b)) = token.split_once(sep) {
            let lo = a.trim().strip_prefix('R')?.parse().ok()?;
            let hi = b.trim().strip_prefix('R')?.parse().ok()?;
            return Some((lo, hi));
        }
    }
    None
}
