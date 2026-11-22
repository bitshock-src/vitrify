//! List scheduling over the unit graph, simulated: a discrete-event run of
//! m workers that each take the highest-priority ready unit (Hu's labels,
//! ties by slug then unit id) the moment they are free. The properties the
//! scheduling guarantee rests on are asserted on seeded random graphs of
//! several shapes — no worker idles while a unit is ready, Graham's bound,
//! Hu's optimum on in-trees of unit work — and what the bound does not say
//! is measured: how close to the lower bound the schedule lands.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use vitrify_core::graph::{Node, UnitGraph};
use vitrify_core::model::{self, Instance};

// ------------------------------------------------------------ scaffolding

/// A tiny seeded PRNG (xorshift64), so every random case reproduces.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1)
    }

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

/// An abstract unit graph: unit `i` belongs to SDD `sdd[i]` and depends on
/// the units `deps[i]`. Acyclic by construction (callers only depend on a
/// unit earlier in some hidden order). Units of one SDD must be added in
/// the order they are numbered.
struct Dag {
    sdds: usize,
    sdd: Vec<usize>,
    deps: Vec<Vec<usize>>,
}

impl Dag {
    fn new(sdds: usize) -> Dag {
        Dag {
            sdds,
            sdd: Vec::new(),
            deps: Vec::new(),
        }
    }

    fn add(&mut self, sdd: usize) -> usize {
        self.sdd.push(sdd);
        self.deps.push(Vec::new());
        self.sdd.len() - 1
    }

    fn depend(&mut self, from: usize, on: usize) {
        if from != on && !self.deps[from].contains(&on) {
            self.deps[from].push(on);
        }
    }

    fn slug(s: usize) -> String {
        format!("s{s:02}")
    }

    /// The unit id of `i`: its place among the units of its SDD.
    fn unit_id(&self, i: usize) -> String {
        let k = self.sdd[..i].iter().filter(|&&s| s == self.sdd[i]).count() + 1;
        format!("U-{k:03}")
    }

    fn node(&self, i: usize) -> Node {
        Node::new(Self::slug(self.sdd[i]), self.unit_id(i))
    }

    fn cross_edges(&self) -> usize {
        self.deps
            .iter()
            .enumerate()
            .flat_map(|(i, ds)| ds.iter().map(move |&d| (i, d)))
            .filter(|&(i, d)| self.sdd[i] != self.sdd[d])
            .count()
    }

    /// Write the repository: one bundle per SDD, every unit producing a
    /// contract of its own, and a pin of the target's contract for every
    /// dependency across SDDs.
    fn write(&self, root: &Path) {
        let put = |rel: String, text: String| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        for s in 0..self.sdds {
            let slug = Self::slug(s);
            put(
                format!("specs/{slug}/sdd.md"),
                format!(
                    "---\nsdd: {slug}\ntitle: {slug}\nschema_version: 2.1\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - x/\n---\n\n# {slug}\n\n## Ground Rules\n\nOne implementer per unit.\n"
                ),
            );
            let mut pins = 0;
            for i in (0..self.sdd.len()).filter(|&i| self.sdd[i] == s) {
                let id = self.unit_id(i);
                let num = &id[2..];
                put(
                    format!("specs/{slug}/contracts/C-1{num}-c.md"),
                    format!(
                        "# C-1{num} — contract\n\n- Producer: {id}\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\nsurface of {id}\n"
                    ),
                );
                let mut depends_on: Vec<String> = Vec::new();
                let mut consumes: Vec<String> = Vec::new();
                for &d in &self.deps[i] {
                    if self.sdd[d] == s {
                        depends_on.push(self.unit_id(d));
                    } else {
                        pins += 1;
                        let pin = format!("C-5{pins:03}");
                        let up = self.node(d);
                        put(
                            format!("specs/{slug}/contracts/{pin}-p.md"),
                            format!(
                                "# {pin} — pin\n\n- Producer: external — {0}, C-1{1}\n- Consumers: —\n- Stability: Frozen by its producer\n- Pin: {0} C-1{1} @ 000000000000\n",
                                up.slug,
                                &up.unit[2..]
                            ),
                        );
                        consumes.push(pin.clone());
                        depends_on.push(pin);
                    }
                }
                put(
                    format!("specs/{slug}/units/{id}-u.md"),
                    format!(
                        "---\nunit: {id}\ntitle: {id} t\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [{}]\nparallel_group: —\nowns: [x/{id}]\nconsumes: [{}]\n---\n\n# {id} — unit\n",
                        depends_on.join(", "),
                        consumes.join(", ")
                    ),
                );
            }
        }
    }
}

/// A scheduling problem: the nodes of a resolved graph, what each waits for,
/// how long each takes, and the dispatch rank.
struct Problem {
    nodes: Vec<Node>,
    deps: Vec<Vec<usize>>,
    /// What must be finished before a unit may start: its dependencies, and
    /// in the barrier variant whole upstream SDDs.
    gate: Vec<Vec<usize>>,
    dur: Vec<u64>,
    /// Position in dispatch order: Hu's label descending, slug, unit id.
    rank: Vec<usize>,
}

struct Schedule {
    start: Vec<u64>,
    end: Vec<u64>,
    makespan: u64,
}

/// Build the repository for `dag`, resolve it, and read the problem off the
/// library: nodes, dependencies and priorities all come from `UnitGraph`.
/// `dur(i)` is the duration of the dag's unit `i`.
fn problem(dag: &Dag, dur: &dyn Fn(usize) -> u64) -> Problem {
    let tmp = tempfile::tempdir().unwrap();
    let root: PathBuf = tmp.path().canonicalize().unwrap();
    dag.write(&root);
    let instances: Vec<Instance> = model::discover(&root)
        .into_iter()
        .map(|h| Instance::load(h).unwrap())
        .collect();
    let g = UnitGraph::build(&instances).unwrap();
    assert!(g.problems().is_empty(), "{:?}", g.problems());
    let r = g.resolve().unwrap();
    let nodes: Vec<Node> = g.nodes().to_vec();
    let at = |x: &Node| nodes.iter().position(|y| y == x).unwrap();
    // the graph holds exactly the dag's edges
    let by_dag: Vec<usize> = (0..dag.sdd.len()).map(|i| at(&dag.node(i))).collect();
    let mut want: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (i, ds) in dag.deps.iter().enumerate() {
        for &d in ds {
            want.insert((by_dag[i], by_dag[d]));
        }
    }
    let have: BTreeSet<(usize, usize)> =
        g.edges().iter().map(|e| (at(&e.from), at(&e.to))).collect();
    assert_eq!(have, want, "the written repository has the dag's edges");
    let deps: Vec<Vec<usize>> = nodes
        .iter()
        .map(|x| r.dependencies(x).into_iter().map(at).collect())
        .collect();
    let p = r.priorities(|_| false);
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| p.cmp(&nodes[a], &nodes[b]));
    let mut rank = vec![0; nodes.len()];
    for (pos, &i) in order.iter().enumerate() {
        rank[i] = pos;
    }
    let mut d = vec![0; nodes.len()];
    for i in 0..dag.sdd.len() {
        d[by_dag[i]] = dur(i);
    }
    Problem {
        gate: deps.clone(),
        deps,
        dur: d,
        rank,
        nodes,
    }
}

impl Problem {
    fn total_work(&self) -> u64 {
        self.dur.iter().sum()
    }

    /// The longest chain, weighted by duration.
    fn critical_path(&self) -> u64 {
        fn go(p: &Problem, i: usize, memo: &mut Vec<Option<u64>>) -> u64 {
            if let Some(v) = memo[i] {
                return v;
            }
            let v = p.dur[i] + p.deps[i].iter().map(|&d| go(p, d, memo)).max().unwrap_or(0);
            memo[i] = Some(v);
            v
        }
        let mut memo = vec![None; self.nodes.len()];
        (0..self.nodes.len())
            .map(|i| go(self, i, &mut memo))
            .max()
            .unwrap_or(0)
    }

    /// The variant that waits for whole SDDs: a unit also waits for every
    /// unit of every SDD its own SDD depends on (the derived SDD graph).
    /// Meaningful only when the derived graph is acyclic.
    fn barrier(mut self) -> Problem {
        let n = self.nodes.len();
        let mut upstream: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for i in 0..n {
            for &d in &self.deps[i] {
                if self.nodes[d].slug != self.nodes[i].slug {
                    upstream
                        .entry(self.nodes[i].slug.clone())
                        .or_default()
                        .insert(self.nodes[d].slug.clone());
                }
            }
        }
        for i in 0..n {
            let Some(ups) = upstream.get(&self.nodes[i].slug) else {
                continue;
            };
            let extra: Vec<usize> = (0..n)
                .filter(|&j| ups.contains(&self.nodes[j].slug))
                .collect();
            self.gate[i].extend(extra);
        }
        self
    }

    /// List scheduling on `m` workers: whenever a worker is free it takes
    /// the best-ranked unit whose gate is finished.
    fn simulate(&self, m: usize) -> Schedule {
        let n = self.nodes.len();
        let mut started = vec![false; n];
        let mut finished = vec![false; n];
        let mut start = vec![0; n];
        let mut end = vec![0; n];
        let mut running: Vec<(u64, usize)> = Vec::new();
        let mut t = 0;
        loop {
            let mut ready: Vec<usize> = (0..n)
                .filter(|&i| !started[i] && self.gate[i].iter().all(|&d| finished[d]))
                .collect();
            ready.sort_by_key(|&i| self.rank[i]);
            for i in ready.into_iter().take(m - running.len()) {
                started[i] = true;
                start[i] = t;
                end[i] = t + self.dur[i];
                running.push((end[i], i));
            }
            let Some(next) = running.iter().map(|r| r.0).min() else {
                break;
            };
            t = next;
            running.retain(|&(e, i)| {
                if e == t {
                    finished[i] = true;
                }
                e != t
            });
        }
        assert!(started.iter().all(|&s| s), "every unit ran");
        Schedule {
            makespan: end.iter().copied().max().unwrap_or(0),
            start,
            end,
        }
    }

    /// Check a schedule from its trace alone: gates finished before a
    /// start, never more than `m` running, and at every event time either
    /// all `m` workers busy or no unit ready — no worker idles while a unit
    /// is claimable.
    fn assert_work_conserving(&self, s: &Schedule, m: usize, what: &str) {
        let n = self.nodes.len();
        for i in 0..n {
            assert_eq!(s.end[i] - s.start[i], self.dur[i], "{what}");
            for &d in &self.gate[i] {
                assert!(s.end[d] <= s.start[i], "{what}: {} early", self.nodes[i]);
            }
        }
        let times: BTreeSet<u64> = s.start.iter().chain(&s.end).copied().collect();
        for &t in &times {
            let busy = (0..n).filter(|&i| s.start[i] <= t && t < s.end[i]).count();
            assert!(busy <= m, "{what}: {busy} running on {m} workers at {t}");
            let ready: Vec<&Node> = (0..n)
                .filter(|&i| s.start[i] > t && self.gate[i].iter().all(|&d| s.end[d] <= t))
                .map(|i| &self.nodes[i])
                .collect();
            assert!(
                busy == m || ready.is_empty(),
                "{what}: a worker idles at {t} while {ready:?} are ready"
            );
        }
    }

    /// Graham's bound, in integers: m * makespan <= W + (m - 1) * CP.
    fn within_graham(&self, makespan: u64, m: usize) -> bool {
        let m = m as u64;
        m * makespan <= self.total_work() + (m - 1) * self.critical_path()
    }
}

// ------------------------------------------------------------ generators

fn durations(rng: &mut Rng, n: usize) -> Vec<u64> {
    (0..n).map(|_| 1 + rng.below(9) as u64).collect()
}

/// Every SDD a chain, with a few edges to units of earlier SDDs.
fn chains(rng: &mut Rng) -> Dag {
    let sdds = 2 + rng.below(5);
    let mut d = Dag::new(sdds);
    let mut all: Vec<Vec<usize>> = vec![Vec::new(); sdds];
    for s in 0..sdds {
        for k in 0..2 + rng.below(8) {
            let i = d.add(s);
            if k > 0 {
                let prev = *all[s].last().unwrap();
                d.depend(i, prev);
            }
            if s > 0 && rng.chance(30) {
                let up = rng.below(s);
                let t = all[up][rng.below(all[up].len())];
                d.depend(i, t);
            }
            all[s].push(i);
        }
    }
    d
}

/// A root unit, a wide fan behind it spread over SDDs, a join, and a tail.
fn fans(rng: &mut Rng) -> Dag {
    let sdds = 2 + rng.below(4);
    let mut d = Dag::new(sdds);
    let width = 4 + rng.below(20);
    // numbered SDD by SDD: the root and the join sit in the first and last
    let root = d.add(0);
    let mut mid_sdds: Vec<usize> = (0..width).map(|_| rng.below(sdds)).collect();
    mid_sdds.sort();
    let mid: Vec<usize> = mid_sdds.iter().map(|&s| d.add(s)).collect();
    for &i in &mid {
        d.depend(i, root);
    }
    let join = d.add(sdds - 1);
    for &i in &mid {
        if rng.chance(80) {
            d.depend(join, i);
        }
    }
    let mut at = join;
    for _ in 0..rng.below(5) {
        let i = d.add(sdds - 1);
        d.depend(i, at);
        at = i;
    }
    d
}

/// Many SDDs; a unit depends on earlier units of its own SDD and of
/// earlier SDDs, so the derived SDD graph is acyclic.
fn layered(rng: &mut Rng, sdds: usize, per: usize, density: u64) -> Dag {
    let mut d = Dag::new(sdds);
    for s in 0..sdds {
        for _ in 0..1 + rng.below(per) {
            let i = d.add(s);
            for j in 0..i {
                if rng.chance(density * if d.sdd[j] == s { 3 } else { 1 }) {
                    d.depend(i, j);
                }
            }
        }
    }
    d
}

/// Units spread over SDDs in no relation to the dependency order, so edges
/// run across SDDs both ways (the derived SDD graph may have mutual pairs).
fn interleaved(rng: &mut Rng) -> Dag {
    let sdds = 2 + rng.below(6);
    let n = 8 + rng.below(40);
    let (d, at) = scattered(rng, n, sdds);
    let mut d = d;
    for a in 0..n {
        for b in 0..a {
            if rng.chance(8) {
                d.depend(at[a], at[b]);
            }
        }
    }
    d
}

/// `n` units over `sdds` SDDs at random; `at[k]` is the dag index of the
/// k-th unit of the hidden order.
fn scattered(rng: &mut Rng, n: usize, sdds: usize) -> (Dag, Vec<usize>) {
    let mut d = Dag::new(sdds);
    let sdd_of: Vec<usize> = (0..n).map(|_| rng.below(sdds)).collect();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by_key(|&i| sdd_of[i]);
    let mut at = vec![0; n];
    for &i in &idx {
        at[i] = d.add(sdd_of[i]);
    }
    (d, at)
}

/// The shape of a generated benchmark repository: `sdds` SDDs of `per`
/// units, local chains and short fans, and a few dependencies on earlier
/// SDDs.
fn benchmark_shaped(rng: &mut Rng, sdds: usize, per: usize) -> Dag {
    let mut d = Dag::new(sdds);
    let mut units: Vec<Vec<usize>> = vec![Vec::new(); sdds];
    for s in 0..sdds {
        for k in 0..per {
            let i = d.add(s);
            if k > 0 {
                // a chain, or a fan out of a recent unit
                let back = if rng.chance(60) {
                    1
                } else {
                    1 + rng.below(k.min(3))
                };
                d.depend(i, units[s][k - back]);
            }
            if s > 0 && rng.chance(25) {
                let up = rng.below(s);
                let t = units[up][rng.below(per)];
                d.depend(i, t);
            }
            units[s].push(i);
        }
    }
    d
}

/// A forest in which every unit has at most one dependent: each unit but
/// the first is, most of the time, depended on by an earlier one.
fn in_tree(rng: &mut Rng, n: usize, sdds: usize) -> Dag {
    let (mut d, at) = scattered(rng, n, sdds);
    for i in 1..n {
        if !rng.chance(10) {
            let parent = rng.below(i);
            d.depend(at[parent], at[i]);
        }
    }
    d
}

const WORKERS: [usize; 4] = [1, 2, 4, 8];

// ------------------------------------------------------------------ tests

#[test]
fn no_worker_idles_while_a_unit_is_ready_and_graham_bounds_the_makespan() {
    let mut cases = 0;
    let mut across = 0;
    for seed in 1..=40u64 {
        let mut rng = Rng::new(seed);
        let dag = match seed % 5 {
            0 => chains(&mut rng),
            1 => fans(&mut rng),
            2 => {
                let sdds = 3 + rng.below(8);
                layered(&mut rng, sdds, 6, 12)
            }
            3 => interleaved(&mut rng),
            _ => {
                let (sdds, per) = (3 + rng.below(6), 4 + rng.below(8));
                benchmark_shaped(&mut rng, sdds, per)
            }
        };
        across += dag.cross_edges();
        let durs = durations(&mut rng, dag.sdd.len());
        let p = problem(&dag, &|i| durs[i]);
        let (w, cp) = (p.total_work(), p.critical_path());
        for m in WORKERS {
            let s = p.simulate(m);
            let what = format!("seed {seed}, m = {m}");
            p.assert_work_conserving(&s, m, &what);
            assert!(
                p.within_graham(s.makespan, m),
                "{what}: makespan {} over the bound (W {w}, CP {cp})",
                s.makespan
            );
            // and never below the two lower bounds
            assert!(s.makespan >= cp && m as u64 * s.makespan >= w, "{what}");
            cases += 1;
        }
    }
    assert_eq!(cases, 160);
    assert!(across >= 100, "too few edges through pins: {across}");
}

/// The least number of unit-time steps to run `deps` on `m` workers, by
/// dynamic programming over the sets of finished units.
fn optimum(deps: &[Vec<usize>], m: usize) -> usize {
    let n = deps.len();
    let full = (1usize << n) - 1;
    let mut best = vec![usize::MAX; full + 1];
    best[full] = 0;
    for done in (0..full).rev() {
        let ready: Vec<usize> = (0..n)
            .filter(|&i| done >> i & 1 == 0 && deps[i].iter().all(|&d| done >> d & 1 == 1))
            .collect();
        // starting as many units as there are workers is never worse
        let k = ready.len().min(m);
        let mut b = usize::MAX;
        for pick in 0usize..1 << ready.len() {
            if pick.count_ones() as usize != k {
                continue;
            }
            let next = ready
                .iter()
                .enumerate()
                .filter(|(j, _)| pick >> j & 1 == 1)
                .fold(done, |acc, (_, &i)| acc | 1 << i);
            if best[next] != usize::MAX {
                b = b.min(best[next] + 1);
            }
        }
        best[done] = b;
    }
    best[0]
}

#[test]
fn unit_work_on_in_trees_finishes_at_hus_optimum() {
    let mut not_trivial = 0;
    let mut cases = 0;
    for seed in 1..=60u64 {
        let mut rng = Rng::new(0x1000 + seed);
        let n = 4 + rng.below(9);
        let sdds = 1 + rng.below(4);
        let dag = in_tree(&mut rng, n, sdds);
        let p = problem(&dag, &|_| 1);
        // every unit has at most one dependent
        let mut dependents = vec![0; n];
        for ds in &p.deps {
            for &d in ds {
                dependents[d] += 1;
            }
        }
        assert!(dependents.iter().all(|&c| c <= 1), "seed {seed}");
        for m in WORKERS {
            let s = p.simulate(m);
            p.assert_work_conserving(&s, m, &format!("seed {seed}, m = {m}"));
            let opt = optimum(&p.deps, m);
            assert_eq!(
                s.makespan as usize, opt,
                "seed {seed}, m = {m}: Hu's optimum"
            );
            // the optimum is not always the plain lower bound
            let lower = (p.critical_path() as usize).max(n.div_ceil(m));
            assert!(opt >= lower);
            not_trivial += usize::from(opt > lower);
            cases += 1;
        }
    }
    assert_eq!(cases, 240);
    assert!(
        not_trivial >= 10,
        "too few trees where the lower bound is not tight: {not_trivial}"
    );
}

#[test]
fn the_worked_example_takes_four_unit_times_and_whole_sdds_in_turn_take_six() {
    // two SDDs of three chained units; core U-001 -> api U-001 and
    // core U-002 -> api U-002 across them
    let mut d = Dag::new(2);
    let core: Vec<usize> = (0..3).map(|_| d.add(0)).collect();
    let api: Vec<usize> = (0..3).map(|_| d.add(1)).collect();
    for s in [&core, &api] {
        d.depend(s[1], s[0]);
        d.depend(s[2], s[1]);
    }
    d.depend(api[0], core[0]);
    d.depend(api[1], core[1]);
    let p = problem(&d, &|_| 1);
    assert_eq!(p.critical_path(), 4);
    let s = p.simulate(2);
    p.assert_work_conserving(&s, 2, "worked example");
    assert_eq!(s.makespan, 4);
    // one worker needs all six; no worker count beats the critical path
    assert_eq!(p.simulate(1).makespan, 6);
    assert_eq!(p.simulate(8).makespan, 4);
    // the critical chain is claimed first: core U-001 starts at 0
    let at = |x: &Node| p.nodes.iter().position(|y| y == x).unwrap();
    assert_eq!(s.start[at(&Node::new("s00", "U-001"))], 0);
    assert_eq!(s.start[at(&Node::new("s01", "U-001"))], 1);
    assert_eq!(s.start[at(&Node::new("s00", "U-002"))], 1);
    // the second SDD waits for the first as a whole: they run in turn
    let barrier = problem(&d, &|_| 1).barrier();
    let s = barrier.simulate(2);
    barrier.assert_work_conserving(&s, 2, "worked example, barrier");
    assert_eq!(s.makespan, 6);
}

const ANOMALIES: [(u64, usize); 2] = [(13, 2), (33, 2)];

#[test]
fn waiting_for_whole_sdds_is_never_faster_and_usually_slower() {
    let (mut slower, mut cases) = (0, 0);
    let mut faster: Vec<(u64, usize)> = Vec::new();
    for seed in 1..=40u64 {
        let mut rng = Rng::new(0x2000 + seed);
        let dag = if seed % 2 == 0 {
            let sdds = 3 + rng.below(6);
            layered(&mut rng, sdds, 6, 10)
        } else {
            let (sdds, per) = (3 + rng.below(6), 4 + rng.below(6));
            benchmark_shaped(&mut rng, sdds, per)
        };
        let durs = durations(&mut rng, dag.sdd.len());
        let list = problem(&dag, &|i| durs[i]);
        let barrier = problem(&dag, &|i| durs[i]).barrier();
        for m in [2, 4, 8] {
            let (a, b) = (list.simulate(m), barrier.simulate(m));
            barrier.assert_work_conserving(&b, m, &format!("barrier, seed {seed}, m = {m}"));
            assert!(list.within_graham(a.makespan, m), "seed {seed}, m = {m}");
            if a.makespan > b.makespan {
                eprintln!(
                    "anomaly: seed {seed}, m = {m}: {} > {}",
                    a.makespan, b.makespan
                );
                faster.push((seed, m));
            }
            slower += usize::from(b.makespan > a.makespan);
            cases += 1;
        }
    }
    eprintln!("barrier slower than list scheduling in {slower} of {cases} cases");
    // A Graham anomaly, recorded and not hidden: list scheduling with Hu's
    // labels is within the theorem's bound but is not optimal on graphs of
    // unequal durations (Hu's labels count units, not time), and a schedule
    // with extra waits can by luck order the work better. These (seed, m)
    // cases finish sooner under the barrier; the bound is asserted on both.
    assert_eq!(faster, ANOMALIES, "the documented anomalies");
    assert!(slower * 2 > cases, "usually slower: {slower} of {cases}");
}

#[test]
fn makespan_against_the_lower_bound_on_benchmark_shaped_graphs() {
    let mut ratios: Vec<(f64, usize)> = Vec::new();
    for seed in 1..=24u64 {
        let mut rng = Rng::new(0x3000 + seed);
        let sdds = 8 + rng.below(8);
        let per = 8 + rng.below(8);
        let dag = benchmark_shaped(&mut rng, sdds, per);
        let durs = durations(&mut rng, dag.sdd.len());
        let p = problem(&dag, &|i| durs[i]);
        let (w, cp) = (p.total_work(), p.critical_path());
        for m in WORKERS {
            let s = p.simulate(m);
            assert!(p.within_graham(s.makespan, m), "seed {seed}, m = {m}");
            let bound = (cp as f64).max(w as f64 / m as f64);
            ratios.push((s.makespan as f64 / bound, m));
        }
    }
    for m in WORKERS {
        let mut r: Vec<f64> = ratios.iter().filter(|x| x.1 == m).map(|x| x.0).collect();
        r.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |f: f64| r[((r.len() - 1) as f64 * f).round() as usize];
        eprintln!(
            "m = {m}: makespan / max(CP, W/m) over {} graphs: min {:.3}, median {:.3}, p90 {:.3}, max {:.3}",
            r.len(),
            r[0],
            q(0.5),
            q(0.9),
            r[r.len() - 1]
        );
    }
    let worst = ratios
        .iter()
        .filter(|x| x.1 == 4)
        .map(|x| x.0)
        .fold(0.0, f64::max);
    assert!(worst <= 1.10, "m = 4: worst ratio {worst:.3}");
}
