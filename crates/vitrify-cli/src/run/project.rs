//! The project-run surface as pure building blocks: the project preflight
//! checks over the repository's SDDs, the plan types, every renderer (job
//! header, dry-run plan, event lines, final report), the `project.json`
//! record, and `--upstream` lane selection. Nothing here spawns a process
//! or runs git; the scheduler fills the plan in and calls these, and every
//! output is a function of its input alone.

use std::collections::BTreeSet;
use std::path::Path;

use vitrify_core::CoreError;
use vitrify_core::cascade::{Owner, Why};
use vitrify_core::graph::{self, Node, UnitGraph, render_path, slug_of};
use vitrify_core::model::Instance;

use crate::out::J;

// -------------------------------------------------------------- preflight

/// A refused project preflight: the exit code and every failure, each a
/// line with its fix, in today's preflight style (`<cause> → <fix>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// 4 for a 2.0 SDD that consumes pins; 2 for every other failure.
    pub code: u8,
    pub failures: Vec<String>,
}

impl Refused {
    fn from_error(e: CoreError) -> Refused {
        let message = e.to_string();
        Refused {
            code: e.exit_code(),
            failures: vec![
                message
                    .strip_prefix("error: ")
                    .unwrap_or(&message)
                    .to_string(),
            ],
        }
    }
}

fn fixed(cause: &str, fix: &str) -> String {
    format!("{cause} → {fix}")
}

/// The project checks that read the SDDs alone: refuse (exit 4) while any
/// SDD at schema 2.0 has units that consume pins, naming every one; then
/// (exit 2) slugs that are empty or shared, the unit graph's L1 and L12
/// problems, and every cycle with the units waiting behind them. Returns
/// the unit graph, built once, for the scheduler to resolve.
pub fn preflight(sdds: &[Instance]) -> Result<UnitGraph, Refused> {
    let old = graph::pins_on_2_0(sdds).map_err(Refused::from_error)?;
    if !old.is_empty() {
        let failures = old
            .iter()
            .map(|inst| {
                fixed(
                    &format!(
                        "SDD {} ({}) is at schema 2.0 and its units consume pins — under 2.0 nothing says which pins a unit needs built first",
                        slug_of(inst),
                        inst.file.display()
                    ),
                    "migrate it to schema 2.1 and list the pins its units need in depends_on",
                )
            })
            .collect();
        return Err(Refused { code: 4, failures });
    }

    let mut failures: Vec<String> = Vec::new();
    for inst in sdds {
        if slug_of(inst).is_empty() {
            failures.push(fixed(
                &format!("SDD {} has no `sdd:` slug", inst.file.display()),
                "set `sdd: <slug>` in its front matter",
            ));
        }
    }
    let graph = UnitGraph::build(sdds).map_err(Refused::from_error)?;
    for clash in graph.clashes() {
        let files: Vec<String> = clash
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        failures.push(fixed(
            &format!(
                "several SDDs have the slug `{}`: {}",
                clash.slug,
                files.join(", ")
            ),
            &format!(
                "give each SDD with the slug `{}` a slug of its own",
                clash.slug
            ),
        ));
    }
    for p in graph.problems() {
        failures.push(fixed(
            &format!("[{}] {} {}: {}", p.check(), p.slug, p.unit, p.cause()),
            &p.remediation(),
        ));
    }
    if let Err(cycles) = graph.resolve() {
        for path in &cycles.cycles {
            failures.push(fixed(
                &format!("[L12] dependency cycle: {}", render_path(path)),
                "break the cycle: remove one depends_on entry on that path",
            ));
        }
        if !cycles.waiting.is_empty() {
            let names: Vec<String> = cycles.waiting.iter().map(Node::to_string).collect();
            failures.push(fixed(
                &format!(
                    "{} {} behind a cycle: {}",
                    cycles.waiting.len(),
                    plural(cycles.waiting.len(), "unit waits", "units wait"),
                    names.join(", ")
                ),
                "waiting units build once the cycle is broken",
            ));
        }
    }
    if failures.is_empty() {
        Ok(graph)
    } else {
        Err(Refused { code: 2, failures })
    }
}

// ------------------------------------------------------------------- plan

/// Where a lane stands (the lane-state table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LaneState {
    /// Units remain, but none is claimable, in flight or due a re-check.
    Idle,
    /// Some unit is claimable, adoptable or in flight.
    Build,
    /// Dirty Verified units whose dependencies have no work pending.
    Recheck,
    /// Nothing to regenerate, re-check or build.
    UpToDate,
    /// The done-condition holds at the end of the run.
    Complete,
    /// Unfinished, set-aside or rotten units remain, or preflight failed.
    Blocked,
}

impl LaneState {
    /// `idle`, `build`, `recheck`, `up to date`, `complete`, `blocked`.
    pub fn label(self) -> &'static str {
        match self {
            LaneState::Idle => "idle",
            LaneState::Build => "build",
            LaneState::Recheck => "recheck",
            LaneState::UpToDate => "up to date",
            LaneState::Complete => "complete",
            LaneState::Blocked => "blocked",
        }
    }

    /// The label as the final report capitalizes it.
    pub fn title(self) -> &'static str {
        match self {
            LaneState::Idle => "Idle",
            LaneState::Build => "Build",
            LaneState::Recheck => "Recheck",
            LaneState::UpToDate => "Up to date",
            LaneState::Complete => "Complete",
            LaneState::Blocked => "Blocked",
        }
    }

    /// The header's phrase: `3 to build`, `2 to re-check`, `2 up to date`.
    fn count_phrase(self, n: usize) -> String {
        match self {
            LaneState::Idle => format!("{n} idle"),
            LaneState::Build => format!("{n} to build"),
            LaneState::Recheck => format!("{n} to re-check"),
            LaneState::UpToDate => format!("{n} up to date"),
            LaneState::Complete => format!("{n} complete"),
            LaneState::Blocked => format!("{n} blocked"),
        }
    }
}

/// One SDD's lane: its slug, its root (repository-relative) and its state,
/// with the counts the plan holds for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub slug: String,
    pub root: String,
    /// Where the lane's own records are: `.vitrify/logs/<job>/<dir>/`.
    pub records: String,
    pub state: LaneState,
    /// Non-Retired units.
    pub units: usize,
    pub verified: usize,
    pub regenerate: usize,
    pub adopt: usize,
    pub recheck: usize,
}

/// A built attempt a run adopts: the unit and the attempt's claim sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopt {
    pub node: Node,
    pub attempt: i64,
}

/// A Verified unit to re-check, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recheck {
    pub node: Node,
    pub why: Why,
}

impl Recheck {
    /// The units whose files made this one dirty, sorted.
    pub fn changed_units(&self) -> Vec<&Node> {
        let Why::Paths(reasons) = &self.why else {
            return Vec::new();
        };
        let set: BTreeSet<&Node> = reasons
            .iter()
            .filter_map(|r| match &r.owner {
                Owner::Unit(n) => Some(n),
                _ => None,
            })
            .collect();
        set.into_iter().collect()
    }
}

/// The pools a run draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub workers: usize,
    pub gate_jobs: usize,
    /// Lanes active at once; `0` is unlimited.
    pub lanes: usize,
}

/// What a project run will do, derived before anything runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub job_id: String,
    /// Sorted by slug.
    pub lanes: Vec<Lane>,
    /// Units to regenerate, sorted.
    pub regenerate: Vec<Node>,
    /// Built attempts to adopt, sorted by unit.
    pub adopt: Vec<Adopt>,
    /// Units to re-check, sorted.
    pub recheck: Vec<Recheck>,
    pub critical_path: Vec<Node>,
    pub capacity: Capacity,
}

impl Plan {
    /// Put every list in its fixed order, so the renderers and the record
    /// are the same however the scheduler collected them.
    pub fn sorted(mut self) -> Plan {
        self.lanes.sort_by(|a, b| a.slug.cmp(&b.slug));
        self.regenerate.sort();
        self.adopt.sort_by(|a, b| a.node.cmp(&b.node));
        self.recheck.sort_by(|a, b| a.node.cmp(&b.node));
        self
    }

    /// Every unit whose files made a re-check necessary, sorted.
    pub fn changed_units(&self) -> Vec<&Node> {
        let set: BTreeSet<&Node> = self
            .recheck
            .iter()
            .flat_map(|r| r.changed_units())
            .collect();
        set.into_iter().collect()
    }
}

// -------------------------------------------------------------- renderers

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

fn critical_line(path: &[Node]) -> String {
    if path.is_empty() {
        "Critical path: none".to_string()
    } else {
        format!(
            "Critical path: {} ({} {})",
            render_path(path),
            path.len(),
            plural(path.len(), "unit", "units")
        )
    }
}

fn capacity_line(c: &Capacity) -> String {
    let lanes = if c.lanes == 0 {
        "unlimited".to_string()
    } else {
        c.lanes.to_string()
    };
    format!(
        "Capacity: workers {} · gate jobs {} · lanes {lanes}",
        c.workers, c.gate_jobs
    )
}

/// `core U-002 changed`; `core U-002, api U-001 changed`; past three units,
/// `core U-002, core U-003, api U-001 and 2 more changed`.
fn changed_phrase(changed: &[&Node]) -> Option<String> {
    if changed.is_empty() {
        return None;
    }
    let shown: Vec<String> = changed.iter().take(3).map(|n| n.to_string()).collect();
    let more = changed.len().saturating_sub(3);
    Some(if more > 0 {
        format!("{} and {more} more changed", shown.join(", "))
    } else {
        format!("{} changed", shown.join(", "))
    })
}

/// The job header: the headline, the plan summary, the critical path, the
/// capacity, and — when the run is starting rather than being planned —
/// the command that watches it.
pub fn header(plan: &Plan, watch: bool) -> Vec<String> {
    let mut counts: Vec<String> = Vec::new();
    for state in [
        LaneState::Build,
        LaneState::Recheck,
        LaneState::UpToDate,
        LaneState::Idle,
        LaneState::Blocked,
        LaneState::Complete,
    ] {
        let n = plan.lanes.iter().filter(|l| l.state == state).count();
        if n > 0 {
            counts.push(state.count_phrase(n));
        }
    }
    let n = plan.lanes.len();
    let mut lines = vec![format!(
        "Project build job — {n} {}: {}  [{}]",
        plural(n, "SDD", "SDDs"),
        counts.join(" · "),
        plan.job_id
    )];
    let mut summary = format!(
        "Plan: regenerate {} {} · adopt {} {} · re-check {} {}",
        plan.regenerate.len(),
        plural(plan.regenerate.len(), "unit", "units"),
        plan.adopt.len(),
        plural(plan.adopt.len(), "built attempt", "built attempts"),
        plan.recheck.len(),
        plural(plan.recheck.len(), "unit", "units"),
    );
    if let Some(why) = changed_phrase(&plan.changed_units()) {
        summary.push_str(&format!(" ({why})"));
    }
    lines.push(summary);
    lines.push(critical_line(&plan.critical_path));
    lines.push(capacity_line(&plan.capacity));
    if watch {
        lines.push(format!(
            "Watch: vitrify run status --job {} --follow",
            plan.job_id
        ));
    }
    lines
}

/// The full `--dry-run` plan: the header (nothing to watch), the lane
/// states, what regenerates, what is adopted, and what is re-checked and
/// why.
pub fn dry_run(plan: &Plan) -> Vec<String> {
    let mut lines = header(plan, false);
    lines.push("Lanes:".to_string());
    let slug_w = plan.lanes.iter().map(|l| l.slug.len()).max().unwrap_or(0);
    let state_w = plan
        .lanes
        .iter()
        .map(|l| l.state.label().len())
        .max()
        .unwrap_or(0);
    for l in &plan.lanes {
        lines.push(format!(
            "  {:slug_w$}  {:state_w$}  {}/{} Verified · regenerate {} · adopt {} · re-check {}  ({})",
            l.slug,
            l.state.label(),
            l.verified,
            l.units,
            l.regenerate,
            l.adopt,
            l.recheck,
            l.root
        ));
    }
    lines.push(format!("Regenerate ({}):", plan.regenerate.len()));
    for n in &plan.regenerate {
        lines.push(format!("  {n}"));
    }
    lines.push(format!("Adopt ({}):", plan.adopt.len()));
    for a in &plan.adopt {
        lines.push(format!("  {}  built attempt c{}", a.node, a.attempt));
    }
    lines.push(format!("Re-check ({}):", plan.recheck.len()));
    for r in &plan.recheck {
        lines.push(format!("  {}  {}", r.node, r.why.describe()));
        if let Some(why) = changed_phrase(&r.changed_units()) {
            lines.push(format!("    {why}"));
        }
    }
    lines
}

/// An event line carrying its lane: `[api] U-002 adopted (built attempt c3)
/// → gate`.
pub fn event_line(slug: &str, message: &str) -> String {
    format!("[{slug}] {message}")
}

/// `[<slug>] <unit> adopted (built attempt c<seq>) → gate`.
pub fn adopted_line(slug: &str, unit: &str, attempt: i64) -> String {
    event_line(
        slug,
        &format!("{unit} adopted (built attempt c{attempt}) → gate"),
    )
}

/// How one lane ended, as the final report words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneOutcome {
    Complete {
        verified: usize,
        total: usize,
        /// Units built by workers this run.
        built: usize,
        /// Units re-checked instead of rebuilt.
        rechecked: usize,
        /// What made them dirty, as phrased for the report: `core`.
        changed: Vec<String>,
        /// The §5 Test command: `Some(true)` passing, `Some(false)` failing;
        /// `None` when the SDD has none.
        suite: Option<bool>,
        /// Worker, escalation and authoring sessions the lane used.
        model_calls: usize,
    },
    /// Why the lane is blocked, one line.
    Blocked {
        detail: String,
    },
    UpToDate {
        since: String,
    },
}

/// One lane's line of the final report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneReport {
    pub slug: String,
    pub outcome: LaneOutcome,
}

impl LaneReport {
    pub fn state(&self) -> LaneState {
        match self.outcome {
            LaneOutcome::Complete { .. } => LaneState::Complete,
            LaneOutcome::Blocked { .. } => LaneState::Blocked,
            LaneOutcome::UpToDate { .. } => LaneState::UpToDate,
        }
    }

    fn detail(&self) -> String {
        match &self.outcome {
            LaneOutcome::Complete {
                verified,
                total,
                built,
                rechecked,
                changed,
                suite,
                model_calls,
            } => {
                let mut parts = vec![format!("{verified}/{total} Verified")];
                if *built > 0 {
                    parts.push(format!("built {built}"));
                }
                if *rechecked > 0 {
                    parts.push(if changed.is_empty() {
                        format!("re-checked {rechecked}")
                    } else {
                        format!("re-checked {rechecked} ({} changed)", changed.join(", "))
                    });
                }
                if let Some(ok) = suite {
                    parts.push(format!(
                        "§5 Test {}",
                        if *ok { "passing" } else { "failing" }
                    ));
                }
                let mut text = parts.join(" · ");
                if *model_calls == 0 && *rechecked > 0 {
                    text.push_str(", no model calls");
                }
                text
            }
            LaneOutcome::Blocked { detail } => detail.clone(),
            LaneOutcome::UpToDate { since } => format!("nothing changed since {since}"),
        }
    }
}

/// The final report: `Complete.` when no lane is blocked, else `Blocked.`,
/// then one aligned line per SDD, sorted by slug.
pub fn report(lanes: &[LaneReport]) -> Vec<String> {
    let blocked = lanes.iter().any(|l| l.state() == LaneState::Blocked);
    let mut lines = vec![if blocked { "Blocked." } else { "Complete." }.to_string()];
    let mut sorted: Vec<&LaneReport> = lanes.iter().collect();
    sorted.sort_by(|a, b| a.slug.cmp(&b.slug));
    let slug_w = sorted
        .iter()
        .map(|l| l.slug.len())
        .max()
        .unwrap_or(0)
        .max(8);
    for l in sorted {
        lines.push(format!(
            "  {:slug_w$} {:12} {}",
            l.slug,
            l.state().title(),
            l.detail()
        ));
    }
    lines
}

// ----------------------------------------------------------- project.json

/// `.vitrify/logs/<job>/project.json`: the plan, the lane states as the run
/// ended, and the critical path as it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub started_at: String,
    /// `None` while the run is alive.
    pub ended_at: Option<String>,
    /// The plan, its lanes carrying their latest states.
    pub plan: Plan,
    /// The units of the critical path as they ran, in order.
    pub critical_path_ran: Vec<Node>,
}

fn node_pairs(n: &Node) -> Vec<(String, J)> {
    vec![
        ("sdd".into(), J::s(n.slug.clone())),
        ("unit".into(), J::s(n.unit.clone())),
    ]
}

fn node_json(n: &Node) -> J {
    J::Obj(node_pairs(n))
}

fn int(n: usize) -> J {
    J::Int(n as i64)
}

impl ProjectRecord {
    pub fn to_json(&self) -> J {
        let p = &self.plan;
        let lanes = p
            .lanes
            .iter()
            .map(|l| {
                J::Obj(vec![
                    ("slug".into(), J::s(l.slug.clone())),
                    ("root".into(), J::s(l.root.clone())),
                    ("records".into(), J::s(l.records.clone())),
                    ("state".into(), J::s(l.state.label())),
                    ("units".into(), int(l.units)),
                    ("verified".into(), int(l.verified)),
                    ("regenerate".into(), int(l.regenerate)),
                    ("adopt".into(), int(l.adopt)),
                    ("recheck".into(), int(l.recheck)),
                ])
            })
            .collect();
        let adopt = p
            .adopt
            .iter()
            .map(|a| {
                let mut pairs = node_pairs(&a.node);
                pairs.push(("attempt".into(), J::Int(a.attempt)));
                J::Obj(pairs)
            })
            .collect();
        let recheck = p
            .recheck
            .iter()
            .map(|r| {
                let mut pairs = node_pairs(&r.node);
                pairs.push(("why".into(), J::s(r.why.describe())));
                pairs.push((
                    "changed_units".into(),
                    J::Arr(r.changed_units().into_iter().map(node_json).collect()),
                ));
                J::Obj(pairs)
            })
            .collect();
        J::Obj(vec![
            ("job_id".into(), J::s(p.job_id.clone())),
            ("started_at".into(), J::s(self.started_at.clone())),
            ("ended_at".into(), J::opt_str(self.ended_at.clone())),
            (
                "capacity".into(),
                J::Obj(vec![
                    ("workers".into(), int(p.capacity.workers)),
                    ("gate_jobs".into(), int(p.capacity.gate_jobs)),
                    ("lanes".into(), int(p.capacity.lanes)),
                ]),
            ),
            ("lanes".into(), J::Arr(lanes)),
            (
                "regenerate".into(),
                J::Arr(p.regenerate.iter().map(node_json).collect()),
            ),
            ("adopt".into(), J::Arr(adopt)),
            ("recheck".into(), J::Arr(recheck)),
            (
                "critical_path".into(),
                J::Arr(p.critical_path.iter().map(node_json).collect()),
            ),
            (
                "critical_path_ran".into(),
                J::Arr(self.critical_path_ran.iter().map(node_json).collect()),
            ),
        ])
    }

    /// Write `<logs_dir>/project.json`, creating the directory.
    pub fn write(&self, logs_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(logs_dir)?;
        let mut text = self.to_json().dumps_indent2();
        text.push('\n');
        std::fs::write(logs_dir.join("project.json"), text)
    }
}

// ------------------------------------------------------------ --upstream

/// The lanes a run takes for `--sdd <slug>`: the slug alone, or with
/// `upstream`, the SDDs it depends on, transitively. Sorted. `None` when
/// the graph has no SDD with that slug.
pub fn select_lanes(graph: &UnitGraph, slug: &str, upstream: bool) -> Option<Vec<String>> {
    if !graph.slugs().iter().any(|s| s == slug) {
        return None;
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    out.insert(slug.to_string());
    if upstream {
        out.extend(graph.upstream(slug));
    }
    Some(out.into_iter().collect())
}

/// What a unit of a selected lane waits on outside the selection, as its
/// blocker line: `U-004 waits on api U-007 and core U-002, outside this run
/// (SDDs api, core not selected; --upstream builds them)`. Only the
/// dependencies outside the selection that are not built yet are named —
/// `status` gives each unit's status now (Built and Verified count as
/// built; Retired, or none, as never built). `None` when no unit it waits
/// on is outside the selection.
pub fn outside_blocker(
    graph: &UnitGraph,
    selected: &[String],
    node: &Node,
    status: &dyn Fn(&Node) -> Option<String>,
) -> Option<String> {
    let outside: Vec<&Node> = graph
        .dependencies(node)
        .into_iter()
        .filter(|d| !selected.contains(&d.slug))
        .filter(|d| !matches!(status(d).as_deref(), Some("Built" | "Verified")))
        .collect();
    if outside.is_empty() {
        return None;
    }
    let units: Vec<String> = outside.iter().map(|n| n.to_string()).collect();
    let sdds: BTreeSet<&str> = outside.iter().map(|n| n.slug.as_str()).collect();
    let one = sdds.len() == 1;
    let sdds: Vec<&str> = sdds.into_iter().collect();
    Some(format!(
        "{} waits on {}, outside this run (SDD{} {} not selected; --upstream builds {})",
        node.unit,
        join_and(&units),
        if one { "" } else { "s" },
        sdds.join(", "),
        if one { "it" } else { "them" },
    ))
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vitrify_core::cascade::Reason;
    use vitrify_core::model;

    fn n(slug: &str, unit: &str) -> Node {
        Node::new(slug, unit)
    }

    fn lane(slug: &str, state: LaneState) -> Lane {
        Lane {
            slug: slug.into(),
            root: format!("specs/{slug}"),
            records: format!(".vitrify/logs/run-x/{slug}/"),
            state,
            units: 12,
            verified: 8,
            regenerate: 2,
            adopt: 1,
            recheck: 0,
        }
    }

    fn sample_plan() -> Plan {
        Plan {
            job_id: "run-20261121T101500Z-p4242".into(),
            lanes: vec![
                lane("web", LaneState::Idle),
                lane("api", LaneState::Build),
                lane("core", LaneState::Build),
                lane("cli", LaneState::Recheck),
                lane("docs", LaneState::UpToDate),
            ],
            regenerate: vec![n("core", "U-003"), n("api", "U-004")],
            adopt: vec![
                Adopt {
                    node: n("core", "U-002"),
                    attempt: 3,
                },
                Adopt {
                    node: n("api", "U-002"),
                    attempt: 1,
                },
            ],
            recheck: vec![
                Recheck {
                    node: n("cli", "U-002"),
                    why: Why::Paths(vec![Reason {
                        path: "core/src/lib.rs".into(),
                        owner: Owner::Unit(n("core", "U-002")),
                    }]),
                },
                Recheck {
                    node: n("cli", "U-001"),
                    why: Why::NoAnchor,
                },
            ],
            critical_path: vec![
                n("core", "U-001"),
                n("core", "U-002"),
                n("api", "U-002"),
                n("api", "U-003"),
            ],
            capacity: Capacity {
                workers: 8,
                gate_jobs: 10,
                lanes: 0,
            },
        }
        .sorted()
    }

    #[test]
    fn header_matches_the_documented_shape() {
        let lines = header(&sample_plan(), true);
        assert_eq!(
            lines.join("\n"),
            "Project build job — 5 SDDs: 2 to build · 1 to re-check · 1 up to date · 1 idle  [run-20261121T101500Z-p4242]\n\
             Plan: regenerate 2 units · adopt 2 built attempts · re-check 2 units (core U-002 changed)\n\
             Critical path: core U-001 → core U-002 → api U-002 → api U-003 (4 units)\n\
             Capacity: workers 8 · gate jobs 10 · lanes unlimited\n\
             Watch: vitrify run status --job run-20261121T101500Z-p4242 --follow"
        );
    }

    #[test]
    fn header_singulars_and_lane_cap() {
        let mut plan = sample_plan();
        plan.lanes.truncate(1);
        plan.regenerate.truncate(1);
        plan.adopt.truncate(1);
        plan.recheck.clear();
        plan.critical_path.truncate(1);
        plan.capacity.lanes = 3;
        let lines = header(&plan, false);
        assert_eq!(
            lines.join("\n"),
            "Project build job — 1 SDD: 1 to build  [run-20261121T101500Z-p4242]\n\
             Plan: regenerate 1 unit · adopt 1 built attempt · re-check 0 units\n\
             Critical path: core U-001 (1 unit)\n\
             Capacity: workers 8 · gate jobs 10 · lanes 3"
        );
    }

    #[test]
    fn dry_run_lists_what_regenerates_is_adopted_and_rechecked() {
        let text = dry_run(&sample_plan()).join("\n");
        assert_eq!(
            text,
            "Project build job — 5 SDDs: 2 to build · 1 to re-check · 1 up to date · 1 idle  [run-20261121T101500Z-p4242]\n\
             Plan: regenerate 2 units · adopt 2 built attempts · re-check 2 units (core U-002 changed)\n\
             Critical path: core U-001 → core U-002 → api U-002 → api U-003 (4 units)\n\
             Capacity: workers 8 · gate jobs 10 · lanes unlimited\n\
             Lanes:\n\
             \x20 api   build       8/12 Verified · regenerate 2 · adopt 1 · re-check 0  (specs/api)\n\
             \x20 cli   recheck     8/12 Verified · regenerate 2 · adopt 1 · re-check 0  (specs/cli)\n\
             \x20 core  build       8/12 Verified · regenerate 2 · adopt 1 · re-check 0  (specs/core)\n\
             \x20 docs  up to date  8/12 Verified · regenerate 2 · adopt 1 · re-check 0  (specs/docs)\n\
             \x20 web   idle        8/12 Verified · regenerate 2 · adopt 1 · re-check 0  (specs/web)\n\
             Regenerate (2):\n\
             \x20 api U-004\n\
             \x20 core U-003\n\
             Adopt (2):\n\
             \x20 api U-002  built attempt c1\n\
             \x20 core U-002  built attempt c3\n\
             Re-check (2):\n\
             \x20 cli U-001  no recorded check\n\
             \x20 cli U-002  core/src/lib.rs (unit core U-002)\n\
             \x20   core U-002 changed"
        );
    }

    #[test]
    fn many_changed_units_are_abbreviated() {
        let nodes: Vec<Node> = (1..=5).map(|i| n("core", &format!("U-00{i}"))).collect();
        let refs: Vec<&Node> = nodes.iter().collect();
        assert_eq!(
            changed_phrase(&refs).unwrap(),
            "core U-001, core U-002, core U-003 and 2 more changed"
        );
        assert_eq!(changed_phrase(&[]), None);
    }

    #[test]
    fn event_lines_carry_the_lane() {
        assert_eq!(
            adopted_line("api", "U-002", 3),
            "[api] U-002 adopted (built attempt c3) → gate"
        );
        assert_eq!(
            event_line("core", "U-001 verified"),
            "[core] U-001 verified"
        );
    }

    fn sample_report() -> Vec<LaneReport> {
        let r = |slug: &str, outcome| LaneReport {
            slug: slug.into(),
            outcome,
        };
        vec![
            r(
                "docs",
                LaneOutcome::UpToDate {
                    since: "9f3c2e1".into(),
                },
            ),
            r(
                "web",
                LaneOutcome::Blocked {
                    detail: "U-004 waits on api U-007 (rotten)".into(),
                },
            ),
            r(
                "core",
                LaneOutcome::Complete {
                    verified: 12,
                    total: 12,
                    built: 3,
                    rechecked: 0,
                    changed: vec![],
                    suite: Some(true),
                    model_calls: 5,
                },
            ),
            r(
                "api",
                LaneOutcome::Blocked {
                    detail: "U-007 rotten: row 2 broke at a1b2c3d (core U-004 integrated) — culprit found".into(),
                },
            ),
            r(
                "cli",
                LaneOutcome::Complete {
                    verified: 4,
                    total: 4,
                    built: 0,
                    rechecked: 2,
                    changed: vec!["core".into()],
                    suite: None,
                    model_calls: 0,
                },
            ),
        ]
    }

    #[test]
    fn final_report_aligns_one_line_per_sdd() {
        assert_eq!(
            report(&sample_report()).join("\n"),
            "Blocked.\n\
             \x20 api      Blocked      U-007 rotten: row 2 broke at a1b2c3d (core U-004 integrated) — culprit found\n\
             \x20 cli      Complete     4/4 Verified · re-checked 2 (core changed), no model calls\n\
             \x20 core     Complete     12/12 Verified · built 3 · §5 Test passing\n\
             \x20 docs     Up to date   nothing changed since 9f3c2e1\n\
             \x20 web      Blocked      U-004 waits on api U-007 (rotten)"
        );
    }

    #[test]
    fn final_report_is_complete_when_nothing_is_blocked() {
        let lanes: Vec<LaneReport> = sample_report()
            .into_iter()
            .filter(|l| l.state() != LaneState::Blocked)
            .collect();
        let lines = report(&lanes);
        assert_eq!(lines[0], "Complete.");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn project_json_has_a_fixed_key_order_and_is_deterministic() {
        let rec = ProjectRecord {
            started_at: "2026-11-21T10:15:00Z".into(),
            ended_at: None,
            plan: sample_plan(),
            critical_path_ran: vec![n("core", "U-001"), n("core", "U-002")],
        };
        let text = rec.to_json().dumps_indent2();
        assert_eq!(text, rec.clone().to_json().dumps_indent2());
        let keys: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("  \""))
            .map(|l| l.trim().split('"').nth(1).unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "job_id",
                "started_at",
                "ended_at",
                "capacity",
                "lanes",
                "regenerate",
                "adopt",
                "recheck",
                "critical_path",
                "critical_path_ran"
            ]
        );
        assert!(text.contains("\"state\": \"up to date\""));
        assert!(text.contains("\"why\": \"core/src/lib.rs (unit core U-002)\""));

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("logs").join("job");
        rec.write(&dir).unwrap();
        let on_disk = std::fs::read_to_string(dir.join("project.json")).unwrap();
        assert_eq!(on_disk, format!("{text}\n"));
    }

    // ---------------------------------------------------- temp repositories

    struct Repo {
        _tmp: tempfile::TempDir,
        root: std::path::PathBuf,
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

        fn instances(&self) -> Vec<Instance> {
            model::discover(&self.root)
                .into_iter()
                .map(|h| Instance::load(h).unwrap())
                .collect()
        }

        /// A bundle SDD at `specs/<slug>/`: contracts as
        /// `(id, header lines)`, units as `(id, depends_on, consumes)`.
        fn bundle(
            &self,
            slug: &str,
            schema: &str,
            contracts: &[(&str, &str)],
            units: &[(&str, &[&str], &[&str])],
        ) {
            self.write(
                &format!("specs/{slug}/sdd.md"),
                &format!(
                    "---\nsdd: {slug}\ntitle: {slug}\nschema_version: {schema}\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - x/\n---\n\n# {slug}\n\n## Ground Rules\n\nOne implementer per unit.\n"
                ),
            );
            for (id, header) in contracts {
                self.write(
                    &format!("specs/{slug}/contracts/{id}-c.md"),
                    &format!(
                        "# {id} — contract\n\n{header}\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\nsurface of {id}\n"
                    ),
                );
            }
            for (id, deps, consumes) in units {
                self.write(
                    &format!("specs/{slug}/units/{id}-u.md"),
                    &format!(
                        "---\nunit: {id}\ntitle: {id} t\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [{}]\nparallel_group: —\nowns: [x/{slug}/{id}]\nconsumes: [{}]\n---\n\n# {id} — unit\n",
                        deps.join(", "),
                        consumes.join(", ")
                    ),
                );
            }
        }
    }

    const PRODUCED_BY_U1: &str = "- Producer: U-001";
    const PRODUCED_BY_U2: &str = "- Producer: U-002";
    const PIN_CORE_C1: &str =
        "- Producer: external — core, C-001\n- Pin: core C-001 @ 000000000000";
    const PIN_API_C1: &str = "- Producer: external — api, C-001\n- Pin: api C-001 @ 000000000000";

    /// core (two units, C-001 produced by U-001) and api consuming it.
    fn two_sdds(repo: &Repo) {
        repo.bundle(
            "core",
            "2.1",
            &[("C-001", PRODUCED_BY_U1)],
            &[("U-001", &[], &[]), ("U-002", &["U-001"], &[])],
        );
        repo.bundle(
            "api",
            "2.1",
            &[("C-001", PIN_CORE_C1)],
            &[
                ("U-001", &["C-001"], &["C-001"]),
                ("U-002", &["U-001"], &[]),
            ],
        );
    }

    #[test]
    fn a_sound_repository_passes_preflight() {
        let repo = Repo::new();
        two_sdds(&repo);
        let graph = preflight(&repo.instances()).unwrap();
        assert_eq!(graph.slugs(), ["api", "core"]);
    }

    #[test]
    fn a_2_0_sdd_consuming_a_pin_is_refused_with_exit_4_naming_every_sdd() {
        let repo = Repo::new();
        repo.bundle(
            "core",
            "2.1",
            &[("C-001", PRODUCED_BY_U1)],
            &[("U-001", &[], &[])],
        );
        repo.bundle(
            "api",
            "2.0",
            &[("C-001", PIN_CORE_C1)],
            &[("U-001", &[], &["C-001"])],
        );
        repo.bundle(
            "web",
            "2.0",
            &[("C-001", PIN_CORE_C1)],
            &[("U-001", &[], &["C-001"])],
        );
        let refused = preflight(&repo.instances()).unwrap_err();
        assert_eq!(refused.code, 4);
        assert_eq!(refused.failures.len(), 2);
        assert!(refused.failures[0].starts_with("SDD api ("));
        assert!(refused.failures[1].starts_with("SDD web ("));
        assert!(
            refused
                .failures
                .iter()
                .all(|f| f.contains("schema 2.0") && f.contains(" → migrate it to schema 2.1"))
        );
    }

    #[test]
    fn unresolved_depends_on_and_authorities_without_producers_exit_2() {
        let repo = Repo::new();
        repo.bundle(
            "core",
            "2.1",
            &[("C-001", "- Producer: external — a vendor")],
            &[("U-001", &[], &[])],
        );
        repo.bundle(
            "api",
            "2.1",
            &[("C-001", PIN_CORE_C1)],
            &[("U-001", &["C-001", "U-009"], &["C-001"])],
        );
        let refused = preflight(&repo.instances()).unwrap_err();
        assert_eq!(refused.code, 2);
        assert_eq!(refused.failures.len(), 2, "{:?}", refused.failures);
        assert!(
            refused
                .failures
                .iter()
                .any(|f| f.starts_with("[L12] api U-001:"))
        );
        assert!(
            refused
                .failures
                .iter()
                .any(|f| f.starts_with("[L1] api U-001:"))
        );
        assert!(refused.failures.iter().all(|f| f.contains(" → ")));
    }

    #[test]
    fn a_cycle_through_pins_is_reported_as_its_path_with_the_units_behind_it() {
        let repo = Repo::new();
        repo.bundle(
            "core",
            "2.1",
            &[("C-001", PRODUCED_BY_U1), ("C-002", PIN_API_C1)],
            &[("U-001", &["C-002"], &["C-002"])],
        );
        repo.bundle(
            "api",
            "2.1",
            &[("C-001", PIN_CORE_C1)],
            &[
                ("U-001", &["C-001"], &["C-001"]),
                ("U-002", &["U-001"], &[]),
            ],
        );
        // core U-001 waits on api C-001 (produced by api U-001), which waits
        // on core C-001 (produced by core U-001)
        repo.write(
            "specs/api/contracts/C-002-c.md",
            "# C-002 — contract\n\n- Producer: U-001\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\ns\n",
        );
        repo.write(
            "specs/core/contracts/C-002-c.md",
            "# C-002 — contract\n\n- Producer: external — api, C-002\n- Consumers: —\n- Stability: Frozen by its producer\n- Pin: api C-002 @ 000000000000\n",
        );
        let refused = preflight(&repo.instances()).unwrap_err();
        assert_eq!(refused.code, 2);
        assert_eq!(
            refused.failures,
            vec![
                "[L12] dependency cycle: api U-001 → core U-001 → api U-001 → break the cycle: remove one depends_on entry on that path".to_string(),
                "1 unit waits behind a cycle: api U-002 → waiting units build once the cycle is broken"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn shared_slugs_are_refused() {
        let repo = Repo::new();
        repo.bundle("core", "2.1", &[], &[("U-001", &[], &[])]);
        // a second SDD with the same slug, in another directory
        repo.write(
            "specs/other/sdd.md",
            "---\nsdd: core\ntitle: x\nschema_version: 2.1\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - y/\n---\n\n# x\n",
        );
        let refused = preflight(&repo.instances()).unwrap_err();
        assert_eq!(refused.code, 2);
        assert!(refused.failures[0].starts_with("several SDDs have the slug `core`: "));
        assert!(refused.failures[0].contains(" → give each SDD"));
    }

    // ------------------------------------------------------- lane selection

    #[test]
    fn lane_selection_takes_the_lane_and_what_it_depends_on() {
        let repo = Repo::new();
        two_sdds(&repo);
        repo.bundle(
            "web",
            "2.1",
            &[(
                "C-001",
                "- Producer: external — api, C-002\n- Pin: api C-002 @ 000000000000",
            )],
            &[("U-001", &["C-001"], &["C-001"])],
        );
        repo.bundle(
            "api",
            "2.1",
            &[("C-001", PIN_CORE_C1), ("C-002", PRODUCED_BY_U2)],
            &[
                ("U-001", &["C-001"], &["C-001"]),
                ("U-002", &["U-001"], &[]),
            ],
        );
        let graph = preflight(&repo.instances()).unwrap();
        assert_eq!(select_lanes(&graph, "ghost", true), None);
        assert_eq!(select_lanes(&graph, "web", false).unwrap(), ["web"]);
        assert_eq!(
            select_lanes(&graph, "web", true).unwrap(),
            ["api", "core", "web"]
        );
        assert_eq!(select_lanes(&graph, "core", true).unwrap(), ["core"]);
    }

    #[test]
    fn units_waiting_outside_the_selection_end_as_named_blockers() {
        let repo = Repo::new();
        two_sdds(&repo);
        let graph = preflight(&repo.instances()).unwrap();
        let selected = select_lanes(&graph, "api", false).unwrap();
        let ready = |_: &Node| Some("Ready".to_string());
        assert_eq!(
            outside_blocker(&graph, &selected, &n("api", "U-001"), &ready).unwrap(),
            "U-001 waits on core U-001, outside this run (SDD core not selected; --upstream builds it)"
        );
        // inside the lane: nothing outside
        assert_eq!(
            outside_blocker(&graph, &selected, &n("api", "U-002"), &ready),
            None
        );
        let all = select_lanes(&graph, "api", true).unwrap();
        assert_eq!(
            outside_blocker(&graph, &all, &n("api", "U-001"), &ready),
            None
        );
        // a dependency outside that is built already is not waited on
        let verified = |_: &Node| Some("Verified".to_string());
        assert_eq!(
            outside_blocker(&graph, &selected, &n("api", "U-001"), &verified),
            None
        );
        let retired = |_: &Node| Some("Retired".to_string());
        assert!(outside_blocker(&graph, &selected, &n("api", "U-001"), &retired).is_some());
    }
}
