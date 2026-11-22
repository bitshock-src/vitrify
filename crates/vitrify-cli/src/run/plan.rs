//! What a run will do, read from the repository before anything runs — the
//! plan the job header, `--dry-run` and `project.json` show — and the
//! dispatch priorities the scheduler orders ready units by.
//!
//! A unit's priority is its critical-path label over the unfinished units
//! of the whole repository ([`Resolution::priorities`]); with
//! `run.priority = "history"` each unit weighs what its past worker
//! sessions took ([`history::weights`]).

use std::collections::BTreeMap;

use vitrify_core::graph::{self, Node, Priorities, Resolution, UnitGraph};
use vitrify_core::model::{self, UtcTime};

use super::history;
use super::project::{Adopt, Capacity, Lane, LaneState, Plan, Recheck};
use super::{Ctx, LaneSeed};
use crate::git::Git;
use crate::incremental::adopt;
use crate::incremental::dirty::{Cascade, DirtySet, Mode, Snapshot};

/// Whether `n` counts as finished for priorities: Verified or Retired.
pub(super) fn finished(graph: &UnitGraph, n: &Node) -> bool {
    matches!(graph.status(n), Some("Verified" | "Retired"))
}

/// The dispatch priorities over the unfinished units of `graph`: by depth,
/// or weighted by `weights` (estimated seconds per unit) when given.
pub(super) fn priorities(
    graph: &UnitGraph,
    resolution: &Resolution,
    weights: Option<&BTreeMap<Node, u64>>,
) -> Priorities {
    match weights {
        Some(known) => {
            let w = history::weights_for(known, graph.nodes());
            resolution
                .priorities_weighted(|n| finished(graph, n), |n| w.get(n).copied().unwrap_or(1))
        }
        None => resolution.priorities(|n| finished(graph, n)),
    }
}

/// Whether `commit` is a run's note setting an attempt aside: only such an
/// attempt is known to have no worker left — its run ended, or the next one
/// recovered it — and so may be adopted.
pub(super) fn set_aside(git: &Git, commit: &str) -> bool {
    git.commit_message(commit)
        .is_ok_and(|m| m.starts_with("sdd: abandoned attempt "))
}

/// The plan for `lanes`, read from the primary checkout at its `HEAD`.
/// `blocked` lanes (their preflight failed) are planned as blocked.
pub(super) fn build(ctx: &Ctx, lanes: &[LaneSeed], job_id: &str) -> Plan {
    let git = &ctx.primary;
    let snapshot = Snapshot::load_resolvable(git, &git.dir, "HEAD")
        .ok()
        .map(|(s, _)| s);
    let mode = if ctx.recheck {
        Mode::Everything
    } else {
        Mode::Changes
    };
    let dirty: Option<DirtySet> = snapshot.as_ref().and_then(|s| match s.dirty(git, mode) {
        Ok(Cascade::Resolved(set)) => Some(set),
        _ => None,
    });
    let weights = ctx
        .cfg
        .run
        .priority_by_history()
        .then(|| history::weights(&ctx.layout.scratch));
    let critical_path: Vec<Node> = snapshot
        .as_ref()
        .and_then(|s| {
            let r = s.resolved.as_ref().ok()?;
            Some(
                priorities(&s.graph, r, weights.as_ref())
                    .critical_path()
                    .into_iter()
                    .cloned()
                    .collect(),
            )
        })
        .unwrap_or_default();
    let now = UtcTime::now();

    let mut plan = Plan {
        job_id: job_id.to_string(),
        lanes: Vec::new(),
        regenerate: Vec::new(),
        adopt: Vec::new(),
        recheck: Vec::new(),
        critical_path,
        capacity: Capacity {
            workers: ctx.cap,
            gate_jobs: ctx.gate_pool.size(),
            lanes: ctx.cfg.run.lane_limit().unwrap_or(0),
        },
    };
    for seed in lanes {
        let inst = &seed.inst;
        let slug = graph::slug_of(inst).to_string();
        let open: Vec<&model::Unit> = inst
            .units
            .iter()
            .filter(|u| !matches!(u.meta.get_str_or("status", "Draft"), "Verified" | "Retired"))
            .collect();
        let units = inst
            .units
            .iter()
            .filter(|u| u.meta.get_str_or("status", "Draft") != "Retired")
            .count();
        let mut adopted: Vec<Adopt> = Vec::new();
        for u in &open {
            if let Ok(Ok(a)) = adopt::adoptable(git, inst, &git.dir, &u.unit)
                && set_aside(git, &a.commit)
            {
                adopted.push(Adopt {
                    node: Node::new(slug.clone(), u.unit.clone()),
                    attempt: a.seq,
                });
            }
        }
        let regenerate: Vec<Node> = open
            .iter()
            .map(|u| Node::new(slug.clone(), u.unit.clone()))
            .filter(|n| !adopted.iter().any(|a| &a.node == n))
            .collect();
        let recheck: Vec<Recheck> = dirty
            .as_ref()
            .map(|set| {
                set.of_sdd(&slug)
                    .map(|d| Recheck {
                        node: d.node.clone(),
                        why: d.why.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let suite_dirty = dirty
            .as_ref()
            .and_then(|set| set.suite(&slug))
            .is_some_and(|s| s.dirty.is_some());
        let claimable = snapshot.as_ref().is_some_and(|s| {
            open.iter().any(|u| {
                let node = Node::new(slug.clone(), u.unit.clone());
                model::unit_view_with(u, &inst.units, now, &s.graph.resolver(&node)).claimable
            })
        });
        let state = if !seed.blocked.is_empty() {
            LaneState::Blocked
        } else if !open.is_empty() && (claimable || !adopted.is_empty()) {
            LaneState::Build
        } else if !open.is_empty() {
            LaneState::Idle
        } else if !recheck.is_empty() || suite_dirty {
            LaneState::Recheck
        } else {
            LaneState::UpToDate
        };
        plan.lanes.push(Lane {
            slug,
            root: crate::cmd::sdd_arg(&seed.ml.sdd_rel),
            records: format!(".vitrify/logs/{job_id}/{}/", seed.ml.slug),
            state,
            units,
            verified: units - open.len(),
            regenerate: regenerate.len(),
            adopt: adopted.len(),
            recheck: recheck.len(),
        });
        plan.regenerate.extend(regenerate);
        plan.adopt.extend(adopted);
        plan.recheck.extend(recheck);
    }
    plan.sorted()
}
