//! The benchmark: `vitrify run` on a synthetic repository of dozens of SDDs
//! and hundreds of units (24 × 12 by default), with stub workers of fixed,
//! seeded durations, at `--cap 4` — and, printed for comparison, at caps 2
//! and 8.
//!
//! What is measured, per cap: the makespan from `STUB_LOG` (first worker
//! start to last worker end), the worker sessions, and the run's own
//! per-unit overhead (below). What is asserted, at cap 4:
//!
//! - one worker session per unit: nothing is built twice;
//! - the no-idle invariant, from the run's `pass` events: a worker slot is
//!   idle only when no unit is claimable;
//! - the makespan is within 10% of the lower bound max(critical path,
//!   total work / cap), counted fairly (below);
//! - the makespan is measurably shorter (by 10% at least) than building
//!   whole SDDs one after another, simulated with the same durations and
//!   the same overheads.
//!
//! # A fair comparison
//!
//! A stub session is only part of what a unit costs. Around it the run
//! claims the unit (a mainline commit), adds its worktree, renders its
//! prompt and spawns the worker, then commits the handoff; that holds the
//! worker slot. After it, the unit's gate and publish stand between it and
//! its dependents. That overhead is real and no schedule avoids it, so the
//! bound charges it — but only the run's *floor* overhead, the 10th
//! percentile over its units, never a median or a mean: time a unit spent
//! stuck behind something else (a lock, another lane's publish, a lost
//! race) is above the floor, and stays the scheduler's to answer for.
//!
//! - **slot overhead** `s`: per unit, the time from its `dispatched` event
//!   to its `integrating` event (the worker's report taken in) minus its
//!   stub duration; the floor over the units.
//! - **link latency** `g`: per unit with dependencies, the time from the
//!   last of its dependencies' stub sessions ending to its own starting;
//!   the floor over the units.
//!
//! The fair lower bound is then max(CP', W'/cap) with each unit costing
//! `d + s` in W' and `d + max(s, g)` along a chain in CP'. The durations
//! (1–3 s) are chosen large against the floor overheads a healthy run has,
//! so the bound is still mostly the work itself, and the raw bound (stub
//! durations alone) is printed beside it. The list-scheduling simulator
//! (the one `crates/vitrify-core/tests/schedule.rs` checks, here with the
//! same overhead model and the run's own priority order) is printed too:
//! it is what the same graph takes when dispatch is exactly list
//! scheduling.
//!
//! It is a measurement, not a gate: minutes long and dependent on the
//! machine's speed and load, so it runs only when `VITRIFY_BENCH` is set,
//! and neither test tier runs it on its own:
//! `VITRIFY_BENCH=1 cargo test -p vitrify-cli --test bench -- --ignored
//! --nocapture`. The repository size and the caps can be changed for a
//! quicker look with `VITRIFY_BENCH_SIZE=8x8 VITRIFY_BENCH_CAPS=4`.
//!
//! The no-idle check reads `pass` events, which the scheduler writes only
//! when it runs; a scheduler stuck elsewhere writes none. So the timeline
//! is read too: the longest stretch in which a slot was free and a unit
//! claimable, and nothing was dispatched, must stay under 3 s (event times
//! are to the second).
//!
//! # Today
//!
//! On 6 SDDs × 8 units of 1–3 s (`VITRIFY_BENCH_SIZE=6x8`, 96 s of work, a
//! 27 s critical path), on a machine shared with other work (load 5 on 8
//! cores), several runs each:
//!
//! | cap | makespan | longest stall | slot / link floor | fair bound | whole SDDs |
//! | --- | --- | --- | --- | --- | --- |
//! | 4 | 62–84 s | 0.9–2.8 s | 0.1–0.3 s / 2.3–3.2 s | 56–68 s (1.07–1.35×) | 156–189 s |
//! | 2 | 81–84 s | 2.7–4.3 s | 0.2–0.3 s / 3.1–3.6 s | 67–73 s (1.12–1.26×) | 186–203 s |
//! | 8 | 66–81 s | 1.8–2.9 s | 0.1–0.2 s / 2.4–3.1 s | 57–67 s (1.11–1.29×) | 159–187 s |
//!
//! Before, slots were refilled in waves of the cap (cap 4: 239–281 s, a
//! 11–15 s link floor): every lane read the whole repository and reset a
//! head checkout of its own at every new head on the scheduler's thread,
//! the dirty set diffed every anchor afresh, and a publish held the
//! process's mainline turn through its merges, stamps and audit, which
//! claims and derives waited on. Now the turn covers only the move of
//! mainline, the lanes share one head read per head, and the diffs carry
//! over from head to head; a lost publish race replays its merges without
//! gating again (no re-gate in 48 units at cap 4).
//!
//! What is left above the floor is the per-unit protocol: each unit is
//! claimed, gated and published on its own, a lane's integrator gates one
//! batch at a time, and every publish is a new head every lane derives
//! from — about 0.3 s of scheduler time per head at 6 SDDs, which grows
//! with the SDDs. The 10% bound holds on some runs and not others on a
//! loaded machine. On a GitHub Actions runner at the default 24 × 12, the
//! fair-bound ratio was 1.58× at cap 4, 1.40× at cap 2 and 1.39× at cap 8,
//! with stalls up to 6.8 s, which is why the benchmark runs only on request.

mod project_rig;
mod synth;

use std::collections::BTreeMap;

use project_rig::{Rig, empty_rig, sessions, stub_log};
use synth::{Spec, Synth};
use vitrify_core::graph::{Node, UnitGraph};
use vitrify_core::model::{self, Instance};

/// The graph as the simulator needs it: units in synth order.
struct Graph {
    nodes: Vec<(String, String)>,
    sdd: Vec<usize>,
    deps: Vec<Vec<usize>>,
    dur: Vec<u64>,
    /// Dispatch rank: the run's priority order (Hu's labels, ties by slug
    /// then unit), read off the library on the written repository.
    rank: Vec<usize>,
}

impl Graph {
    fn of(s: &Synth, root: &std::path::Path) -> Graph {
        let nodes: Vec<(String, String)> = s
            .units
            .iter()
            .map(|u| (u.slug.clone(), u.id.clone()))
            .collect();
        let at = |x: &(String, String)| nodes.iter().position(|y| y == x).unwrap();
        let mut deps = vec![Vec::new(); nodes.len()];
        for (from, to) in s.edges() {
            deps[at(&from)].push(at(&to));
        }
        let sdd = s
            .units
            .iter()
            .map(|u| u.slug[1..].parse().unwrap())
            .collect();
        let instances: Vec<Instance> = model::discover(root)
            .into_iter()
            .map(|h| Instance::load(h).unwrap())
            .collect();
        let g = UnitGraph::build(&instances).unwrap();
        let p = g.resolve().unwrap().priorities(|_| false);
        let node = |i: usize| Node::new(nodes[i].0.clone(), nodes[i].1.clone());
        let mut order: Vec<usize> = (0..nodes.len()).collect();
        order.sort_by(|&a, &b| p.cmp(&node(a), &node(b)));
        let mut rank = vec![0; nodes.len()];
        for (pos, &i) in order.iter().enumerate() {
            rank[i] = pos;
        }
        Graph {
            dur: s.units.iter().map(|u| u.millis).collect(),
            nodes,
            sdd,
            deps,
            rank,
        }
    }

    fn total_work(&self, s: u64) -> u64 {
        self.dur.iter().map(|d| d + s).sum()
    }

    /// The longest chain, each unit costing its duration plus `extra`.
    fn critical_path(&self, extra: u64) -> u64 {
        let mut memo: Vec<Option<u64>> = vec![None; self.nodes.len()];
        fn go(g: &Graph, i: usize, extra: u64, memo: &mut Vec<Option<u64>>) -> u64 {
            if let Some(v) = memo[i] {
                return v;
            }
            let v = g.dur[i]
                + extra
                + g.deps[i]
                    .iter()
                    .map(|&d| go(g, d, extra, memo))
                    .max()
                    .unwrap_or(0);
            memo[i] = Some(v);
            v
        }
        (0..self.nodes.len())
            .map(|i| go(self, i, extra, &mut memo))
            .max()
            .unwrap_or(0)
    }

    /// List scheduling on `m` slots: a free slot takes the best-ranked
    /// ready unit at once. A unit holds its slot for its duration plus `s`;
    /// its dependents are ready `latency` after it leaves the slot. With
    /// `whole_sdds`, a unit also waits for every unit of every earlier SDD
    /// — the SDDs built one after another (the synthetic graph only points
    /// at earlier SDDs, so their order is a topological one).
    fn simulate(&self, m: usize, s: u64, latency: u64, whole_sdds: bool) -> u64 {
        let n = self.nodes.len();
        let gate: Vec<Vec<usize>> = (0..n)
            .map(|i| {
                let mut g = self.deps[i].clone();
                if whole_sdds {
                    g.extend((0..n).filter(|&j| self.sdd[j] < self.sdd[i]));
                }
                g
            })
            .collect();
        let mut end: Vec<Option<u64>> = vec![None; n];
        let mut started = vec![false; n];
        let mut running: Vec<u64> = Vec::new();
        let mut t = 0u64;
        let mut left = n;
        // when unit `i` is ready, once everything it waits for has ended
        let ready_at = |end: &[Option<u64>], i: usize| -> Option<u64> {
            gate[i]
                .iter()
                .map(|&d| end[d].map(|e| e + latency))
                .try_fold(0, |a, x| x.map(|x| a.max(x)))
        };
        while left > 0 {
            running.retain(|&e| e > t);
            let mut ready: Vec<usize> = (0..n)
                .filter(|&i| !started[i] && ready_at(&end, i).is_some_and(|r| r <= t))
                .collect();
            ready.sort_by_key(|&i| self.rank[i]);
            for i in ready.into_iter().take(m.saturating_sub(running.len())) {
                started[i] = true;
                left -= 1;
                let e = t + self.dur[i] + s;
                end[i] = Some(e);
                running.push(e);
            }
            // the next moment anything changes: a slot frees, a unit
            // becomes ready
            let next_end = running.iter().copied().filter(|&e| e > t).min();
            let next_ready = (0..n)
                .filter(|&i| !started[i])
                .filter_map(|i| ready_at(&end, i))
                .filter(|&r| r > t)
                .min();
            match (next_end, next_ready) {
                (Some(a), Some(b)) => t = a.min(b),
                (Some(a), None) | (None, Some(a)) => t = a,
                (None, None) => break,
            }
        }
        end.into_iter().flatten().max().unwrap_or(0)
    }
}

fn percentile(mut v: Vec<u64>, f: f64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * f).floor() as usize]
}

fn seconds(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// What one run measured.
struct Measured {
    cap: usize,
    makespan: u64,
    sessions: usize,
    slot_overhead: u64,
    link_latency: u64,
    passes: Vec<serde_json::Value>,
    /// The longest stretch in which a worker slot was free and a unit was
    /// claimable — every unit it depends on Verified, itself not yet
    /// dispatched — and nothing was dispatched (ms). The scheduler writes
    /// no `pass` event while it is stuck, so this is read off the timeline
    /// instead: dispatches and verdicts from the event log, the workers'
    /// ends from the stub log.
    stall: u64,
}

/// [`Measured::stall`], sampled every 100 ms. `dispatched`, `verified` and
/// `ended` map units to times; a unit not in `ended` holds its slot to the
/// end.
fn longest_stall(
    g: &Graph,
    cap: usize,
    dispatched: &BTreeMap<usize, u64>,
    verified: &BTreeMap<usize, u64>,
    ended: &BTreeMap<usize, u64>,
    (from, to): (u64, u64),
) -> u64 {
    let claimable_at = |i: usize| -> Option<u64> {
        g.deps[i]
            .iter()
            .try_fold(from, |a, d| verified.get(d).map(|t| a.max(*t)))
    };
    let (mut longest, mut since) = (0, None::<u64>);
    let mut t = from;
    while t <= to {
        let busy = dispatched
            .iter()
            .filter(|(i, d)| **d <= t && ended.get(i).is_none_or(|e| *e > t))
            .count();
        let claimable = (0..g.nodes.len()).any(|i| {
            dispatched.get(&i).is_none_or(|d| *d > t) && claimable_at(i).is_some_and(|c| c <= t)
        });
        let dispatching = dispatched
            .values()
            .any(|d| *d > t.saturating_sub(100) && *d <= t);
        if busy < cap && claimable && !dispatching {
            let start = *since.get_or_insert(t);
            longest = longest.max(t - start);
        } else {
            since = None;
        }
        t += 100;
    }
    longest
}

fn epoch_ms(at: &str) -> u64 {
    // `YYYY-MM-DDTHH:MM:SSZ`, to milliseconds since the epoch
    let d = |a: usize, b: usize| -> i64 { at[a..b].parse().unwrap() };
    let (y, mo, da) = (d(0, 4), d(5, 7), d(8, 10));
    let (h, mi, s) = (d(11, 13), d(14, 16), d(17, 19));
    // days from civil (Howard Hinnant)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + h * 3600 + mi * 60 + s) * 1000) as u64
}

fn run_at(s: &Synth, g: &Graph, cap: usize) -> Measured {
    let r: Rig = empty_rig();
    s.write(&r.root, &r.support);
    synth::commit_all(&r.root);
    let log = r.scratch().join("stub.log");
    let (log_s, durations) = (
        log.display().to_string(),
        r.support.join("durations.txt").display().to_string(),
    );
    let cap_s = cap.to_string();
    let started = std::time::Instant::now();
    let stdout = r.run_expect(
        0,
        &["run", "--cap", &cap_s],
        &[("STUB_LOG", &log_s), ("STUB_SLEEP_MAP", &durations)],
    );
    eprintln!("cap {cap}: the run took {:?}", started.elapsed());
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");

    let ran = sessions(&stub_log(&log));
    let at = |sdd: &str, unit: &str| {
        g.nodes
            .iter()
            .position(|(a, b)| a == sdd && b == unit)
            .unwrap()
    };
    let mut span: BTreeMap<usize, (u64, u64)> = BTreeMap::new();
    for (sdd, unit, _, start, end) in &ran {
        let end = end.expect("every session ended");
        span.insert(at(sdd, unit), (*start as u64, end as u64));
    }
    let first = span.values().map(|x| x.0).min().unwrap();
    let last = span.values().map(|x| x.1).max().unwrap();

    let events = r.events();
    let when = |kind: &str| -> BTreeMap<usize, u64> {
        events
            .iter()
            .filter(|e| e["event"] == kind)
            .filter_map(|e| {
                let i = at(e["lane"].as_str()?, e["unit"].as_str()?);
                Some((i, epoch_ms(e["at"].as_str()?)))
            })
            .collect()
    };
    let (dispatched, integrating) = (when("dispatched"), when("integrating"));
    let verified: BTreeMap<usize, u64> = events
        .iter()
        .filter(|e| e["event"] == "outcome" && e["outcome"] == "verified")
        .filter_map(|e| {
            let i = at(e["lane"].as_str()?, e["unit"].as_str()?);
            Some((i, epoch_ms(e["at"].as_str()?)))
        })
        .collect();
    let ended: BTreeMap<usize, u64> = span.iter().map(|(i, x)| (*i, x.1)).collect();
    let stall = longest_stall(
        g,
        cap,
        &dispatched,
        &verified,
        &ended,
        (dispatched.values().min().copied().unwrap_or(first), last),
    );
    let slot: Vec<u64> = dispatched
        .iter()
        .filter_map(|(i, d)| {
            let held = integrating.get(i)?.checked_sub(*d)?;
            Some(held.saturating_sub(g.dur[*i]))
        })
        .collect();
    let link: Vec<u64> = span
        .iter()
        .filter(|(i, _)| !g.deps[**i].is_empty())
        .filter_map(|(i, (start, _))| {
            let ready = g.deps[*i]
                .iter()
                .map(|d| span.get(d).map(|x| x.1))
                .try_fold(0, |a, x| x.map(|x| a.max(x)))?;
            Some(start.saturating_sub(ready))
        })
        .collect();
    Measured {
        cap,
        makespan: last - first,
        sessions: ran.len(),
        slot_overhead: percentile(slot, 0.1),
        link_latency: percentile(link, 0.1),
        passes: events
            .into_iter()
            .filter(|e| e["event"] == "pass")
            .collect(),
        stall,
    }
}

/// The longest a free slot may wait while a unit is claimable.
const STALL_MS: u64 = 3000;

fn size() -> (usize, usize) {
    let text = std::env::var("VITRIFY_BENCH_SIZE").unwrap_or_else(|_| "24x12".to_string());
    let (a, b) = text
        .split_once('x')
        .expect("VITRIFY_BENCH_SIZE is <sdds>x<units>");
    (a.parse().unwrap(), b.parse().unwrap())
}

fn caps() -> Vec<usize> {
    std::env::var("VITRIFY_BENCH_CAPS")
        .unwrap_or_else(|_| "4,2,8".to_string())
        .split(',')
        .map(|c| c.trim().parse().unwrap())
        .collect()
}

#[test]
#[ignore = "benchmark: runs only with VITRIFY_BENCH=1 (minutes long, timing-dependent)"]
fn the_makespan_is_within_ten_percent_of_the_lower_bound_at_cap_4() {
    if std::env::var_os("VITRIFY_BENCH").is_none() {
        eprintln!("benchmark skipped: set VITRIFY_BENCH=1 to run it");
        return;
    }
    let (sdds, units) = size();
    let mut spec = Spec::new(sdds, units, 2026);
    spec.millis = (1000, 3000);
    let s = synth::generate(&spec);
    let g = {
        let r = empty_rig();
        s.write(&r.root, &r.support);
        Graph::of(&s, &r.root)
    };
    let n = g.nodes.len();
    eprintln!(
        "benchmark: {sdds} SDDs × {units} units = {n} units, {} edges across SDDs; work {}, critical path {}",
        s.cross_edges().len(),
        seconds(g.total_work(0)),
        seconds(g.critical_path(0))
    );

    let mut results = Vec::new();
    for cap in caps() {
        let m = run_at(&s, &g, cap);
        let (o, l) = (m.slot_overhead, m.link_latency);
        let raw = g.critical_path(0).max(g.total_work(0) / cap as u64);
        let fair = g.critical_path(o.max(l)).max(g.total_work(o) / cap as u64);
        let list = g.simulate(cap, o, l.saturating_sub(o), false);
        let whole = g.simulate(cap, o, l.saturating_sub(o), true);
        eprintln!(
            "cap {cap}: makespan {} · {} worker sessions · longest stall {} · overhead per unit: slot {}, link {} · bound {} raw, {} fair ({:.2}×) · list schedule {} · whole SDDs in turn {} ({:.2}×)",
            seconds(m.makespan),
            m.sessions,
            seconds(m.stall),
            seconds(o),
            seconds(l),
            seconds(raw),
            seconds(fair),
            m.makespan as f64 / fair as f64,
            seconds(list),
            seconds(whole),
            m.makespan as f64 / whole as f64,
        );
        results.push((m, fair, whole));
    }

    for (m, fair, whole) in &results {
        assert_eq!(m.sessions, n, "cap {}: one worker session per unit", m.cap);
        assert!(!m.passes.is_empty());
        // event times are to the second: a stall is one longer than that
        assert!(
            m.stall <= STALL_MS,
            "cap {}: a slot was free and a unit claimable, and nothing was dispatched, for {}",
            m.cap,
            seconds(m.stall)
        );
        for p in &m.passes {
            assert!(
                p["working"].as_i64().unwrap() <= m.cap as i64,
                "cap {}: {p}",
                m.cap
            );
            assert!(
                p["idle"] == 0 || p["waiting"] == 0,
                "cap {}: a slot idles while units are claimable: {p}",
                m.cap
            );
        }
        if m.cap == 4 {
            assert!(
                m.makespan * 10 <= fair * 11,
                "cap 4: makespan {} against the fair bound {}",
                seconds(m.makespan),
                seconds(*fair)
            );
            assert!(
                m.makespan * 10 <= whole * 9,
                "cap 4: makespan {} against whole SDDs in turn {}",
                seconds(m.makespan),
                seconds(*whole)
            );
        }
    }
}

/// The simulator itself, on a graph small enough to check by hand.
#[test]
fn the_simulator_schedules_a_small_graph_as_list_scheduling_does() {
    // two SDDs: s00 U-001 → U-002, s01 U-001 depends on s00 U-001
    let g = Graph {
        nodes: vec![
            ("s00".into(), "U-001".into()),
            ("s00".into(), "U-002".into()),
            ("s01".into(), "U-001".into()),
        ],
        sdd: vec![0, 0, 1],
        deps: vec![vec![], vec![0], vec![0]],
        dur: vec![10, 5, 7],
        rank: vec![0, 2, 1],
    };
    assert_eq!(g.critical_path(0), 17);
    assert_eq!(g.total_work(1), 25);
    // two slots: U-001 at 0–10, then both dependents side by side
    assert_eq!(g.simulate(2, 0, 0, false), 17);
    // one slot, ranked: 0, s01 U-001, s00 U-002
    assert_eq!(g.simulate(1, 0, 0, false), 22);
    // overheads: slot +1 each, dependents ready 2 after
    assert_eq!(g.simulate(2, 1, 2, false), 11 + 2 + 8);
    // whole SDDs in turn: s01 waits for s00 U-002 too
    assert_eq!(g.simulate(2, 0, 0, true), 22);
}
