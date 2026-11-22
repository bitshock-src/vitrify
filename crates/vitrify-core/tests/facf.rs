//! Flake-aware culprit finding: deterministic culprits at every position,
//! flaky rows with and without a real culprit (seeded, so the measured rates
//! are reproducible), the prior from touched commits, and determinism.

use vitrify_core::facf::{
    Config, DEFAULT_NO_CULPRIT_WEIGHT, Hypothesis, Search, Verdict, default_budget,
};

/// xorshift64*: a seeded PRNG for simulating flaky rows.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // Mix the seed so small seeds do not start in a low-entropy state.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How the simulated row behaves: broken from `culprit` on (always fails
/// there), and fails with probability `flake` anywhere it is not broken.
#[derive(Clone, Copy)]
struct Row {
    culprit: Option<usize>,
    flake: f64,
}

impl Row {
    fn passes(&self, version: usize, rng: &mut Rng) -> bool {
        if self.culprit.is_some_and(|k| version >= k) {
            return false;
        }
        rng.unit() >= self.flake
    }
}

struct Run {
    verdict: Verdict,
    probes: usize,
    sequence: Vec<usize>,
}

fn drive(mut search: Search, row: Row, rng: &mut Rng) -> Run {
    let mut sequence = Vec::new();
    while let Some(v) = search.next_probe() {
        sequence.push(v);
        let passed = row.passes(v, rng);
        search.observe(v, passed).unwrap();
        let sum: f64 = search.posterior().iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "posterior sums to {sum}");
    }
    Run {
        verdict: search.verdict(),
        probes: search.probes_used(),
        sequence,
    }
}

fn ceil_log2(n: usize) -> usize {
    (usize::BITS - n.saturating_sub(1).leading_zeros()) as usize
}

fn uniform(n: usize) -> Vec<f64> {
    vec![1.0; n]
}

/// Seeds run per measured rate.
const SEEDS: u64 = 400;

#[test]
fn clean_row_names_the_culprit_everywhere_in_about_log2_n_probes() {
    // With nothing flaky and a model that knows it, bisection depth plus a
    // confirming probe or two suffices.
    let config = Config {
        flake_rate: 0.0,
        ..Config::default()
    };
    let mut rng = Rng::new(1);
    for n in 1..=20usize {
        for k in 0..n {
            let search = Search::new(&uniform(n), DEFAULT_NO_CULPRIT_WEIGHT, &config).unwrap();
            let row = Row {
                culprit: Some(k),
                flake: 0.0,
            };
            let run = drive(search, row, &mut rng);
            match run.verdict {
                Verdict::Culprit { index, .. } => assert_eq!(index, k, "n={n} k={k}"),
                other => panic!("n={n} k={k}: {other:?}"),
            }
            assert!(
                run.probes <= ceil_log2(n) + 1,
                "n={n} k={k}: {} probes",
                run.probes
            );
        }
    }
}

#[test]
fn clean_row_with_default_flake_rate_still_names_the_culprit() {
    // The model allows flakes the row does not have, so it asks for a few
    // more probes, but never names the wrong commit.
    let mut rng = Rng::new(2);
    for n in 1..=20usize {
        for k in 0..n {
            let search =
                Search::new(&uniform(n), DEFAULT_NO_CULPRIT_WEIGHT, &Config::default()).unwrap();
            let row = Row {
                culprit: Some(k),
                flake: 0.0,
            };
            let run = drive(search, row, &mut rng);
            match run.verdict {
                Verdict::Culprit { index, .. } => assert_eq!(index, k, "n={n} k={k}"),
                other => panic!("n={n} k={k}: {other:?}"),
            }
            assert!(
                run.probes <= ceil_log2(n) + 6,
                "n={n} k={k}: {} probes",
                run.probes
            );
        }
    }
}

#[derive(Default, Debug)]
struct Tally {
    culprit_right: usize,
    culprit_wrong: usize,
    flake: usize,
    inconclusive: usize,
    probes: usize,
}

fn tally(build: impl Fn() -> Search, row_for: impl Fn(u64) -> Row) -> Tally {
    let mut t = Tally::default();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed + 1000);
        let row = row_for(seed);
        let run = drive(build(), row, &mut rng);
        t.probes += run.probes;
        match run.verdict {
            Verdict::Culprit { index, .. } if Some(index) == row.culprit => t.culprit_right += 1,
            Verdict::Culprit { .. } => t.culprit_wrong += 1,
            Verdict::Flake { .. } => t.flake += 1,
            Verdict::Inconclusive { .. } | Verdict::Searching => t.inconclusive += 1,
        }
    }
    t
}

fn rate(count: usize) -> f64 {
    count as f64 / SEEDS as f64
}

fn flake_only(n: usize, model_f: f64, true_f: f64, confidence: f64) -> Tally {
    let config = Config {
        flake_rate: model_f,
        confidence,
        ..Config::default()
    };
    tally(
        || Search::new(&uniform(n), DEFAULT_NO_CULPRIT_WEIGHT, &config).unwrap(),
        |_| Row {
            culprit: None,
            flake: true_f,
        },
    )
}

fn flaky_with_culprit(n: usize, model_f: f64, true_f: f64) -> Tally {
    let config = Config {
        flake_rate: model_f,
        ..Config::default()
    };
    tally(
        || Search::new(&uniform(n), DEFAULT_NO_CULPRIT_WEIGHT, &config).unwrap(),
        |seed| Row {
            culprit: Some((seed as usize * 7 + 3) % n),
            flake: true_f,
        },
    )
}

#[test]
fn flaky_row_without_culprit_concludes_flake() {
    // The row fails 30% of the time everywhere; no commit broke it. With the
    // flake rate known, most searches conclude Flake and few blame a commit
    // (measured over 400 seeds: flake 88%/83%, blamed 11%/15% for n = 8/16).
    for n in [8usize, 16] {
        let t = flake_only(n, 0.3, 0.3, 0.95);
        println!("flake-only n={n}: {t:?}");
        assert!(rate(t.flake) >= 0.8, "n={n}: flake {}", rate(t.flake));
        assert!(
            rate(t.culprit_wrong) <= 0.2,
            "n={n}: blamed a commit {}",
            rate(t.culprit_wrong)
        );
    }
}

#[test]
fn flaky_row_without_culprit_almost_never_blames_a_commit_at_high_confidence() {
    // Asking for 99% makes a wrong name rare (measured: 2-3%).
    for n in [8usize, 16] {
        let t = flake_only(n, 0.3, 0.3, 0.99);
        println!("flake-only, 0.99, n={n}: {t:?}");
        assert!(rate(t.flake) >= 0.9, "n={n}: flake {}", rate(t.flake));
        assert!(
            rate(t.culprit_wrong) <= 0.05,
            "n={n}: blamed a commit {}",
            rate(t.culprit_wrong)
        );
    }
}

#[test]
fn a_mildly_flaky_row_with_the_default_flake_rate_is_a_flake() {
    // The default rate (0.2) overestimates a row that fails 10% of the time:
    // still a flake in most searches, a wrong blame only now and then.
    for n in [8usize, 16] {
        let t = flake_only(n, Config::default().flake_rate, 0.1, 0.95);
        println!("default model, true 0.1, n={n}: {t:?}");
        assert!(rate(t.flake) >= 0.85, "n={n}: flake {}", rate(t.flake));
        assert!(
            rate(t.culprit_wrong) <= 0.15,
            "n={n}: {}",
            rate(t.culprit_wrong)
        );
    }
}

#[test]
fn flaky_row_with_a_real_culprit_still_names_it() {
    // Broken from the culprit on, and also failing 30% of the time before it.
    for n in [8usize, 16] {
        let t = flaky_with_culprit(n, 0.3, 0.3);
        println!("flaky+culprit n={n}: {t:?}");
        assert!(
            rate(t.culprit_right) >= 0.9,
            "n={n}: right {}",
            rate(t.culprit_right)
        );
        assert!(
            rate(t.culprit_wrong) <= 0.05,
            "n={n}: wrong {}",
            rate(t.culprit_wrong)
        );
    }
    // And with the default rate, an underestimate here.
    for n in [8usize, 16] {
        let t = flaky_with_culprit(n, Config::default().flake_rate, 0.3);
        println!("flaky+culprit, default model, n={n}: {t:?}");
        assert!(
            rate(t.culprit_right) >= 0.85,
            "n={n}: right {}",
            rate(t.culprit_right)
        );
        assert!(
            rate(t.culprit_wrong) <= 0.1,
            "n={n}: wrong {}",
            rate(t.culprit_wrong)
        );
    }
}

#[test]
fn prior_on_touched_commits_finds_a_touched_culprit_in_fewer_probes() {
    let n = 16;
    let touched: Vec<bool> = (0..n).map(|i| i == 5 || i == 6 || i == 11).collect();
    let config = Config {
        flake_rate: 0.0,
        ..Config::default()
    };
    let mut with_prior = 0;
    let mut without_prior = 0;
    for &k in &[5usize, 6, 11] {
        let mut rng = Rng::new(7);
        let row = Row {
            culprit: Some(k),
            flake: 0.0,
        };
        let a = drive(
            Search::from_touched(&touched, &config).unwrap(),
            row,
            &mut rng,
        );
        let b = drive(
            Search::new(&uniform(n), DEFAULT_NO_CULPRIT_WEIGHT, &config).unwrap(),
            row,
            &mut rng,
        );
        for run in [&a, &b] {
            assert!(
                matches!(run.verdict, Verdict::Culprit { index, .. } if index == k),
                "k={k}: {:?}",
                run.verdict
            );
        }
        with_prior += a.probes;
        without_prior += b.probes;
    }
    println!("touched probes {with_prior} vs uniform {without_prior}");
    assert!(
        with_prior < without_prior,
        "{with_prior} probes with the prior, {without_prior} without"
    );
}

#[test]
fn culprit_outside_the_touched_commits_is_still_found() {
    // An undeclared dependency: the culprit is a commit the declarations did
    // not point at. The small share of the prior still lets the search reach
    // it, in every position, with and without flakes.
    let n = 12;
    let touched: Vec<bool> = (0..n).map(|i| i == 2 || i == 3).collect();
    let config = Config {
        flake_rate: 0.0,
        ..Config::default()
    };
    for k in (0..n).filter(|&k| !touched[k]) {
        let mut rng = Rng::new(11);
        let row = Row {
            culprit: Some(k),
            flake: 0.0,
        };
        let run = drive(
            Search::from_touched(&touched, &config).unwrap(),
            row,
            &mut rng,
        );
        assert!(
            matches!(run.verdict, Verdict::Culprit { index, .. } if index == k),
            "k={k}: {:?}",
            run.verdict
        );
    }
    let flaky = Config::default();
    let t = tally(
        || Search::from_touched(&touched, &flaky).unwrap(),
        |seed| Row {
            culprit: Some([0usize, 7, 9, 11][seed as usize % 4]),
            flake: 0.1,
        },
    );
    println!("undeclared, flaky: {t:?}");
    assert!(
        rate(t.culprit_right) >= 0.85,
        "right in {}",
        rate(t.culprit_right)
    );
    assert!(
        rate(t.culprit_wrong) <= 0.05,
        "wrong in {}",
        rate(t.culprit_wrong)
    );
}

#[test]
fn same_inputs_and_outcomes_give_the_same_probe_sequence() {
    let touched: Vec<bool> = (0..13).map(|i| i % 4 == 1).collect();
    let row = Row {
        culprit: Some(9),
        flake: 0.2,
    };
    let go = || {
        let mut rng = Rng::new(99);
        drive(
            Search::from_touched(&touched, &Config::default()).unwrap(),
            row,
            &mut rng,
        )
    };
    let (a, b) = (go(), go());
    assert!(!a.sequence.is_empty());
    assert_eq!(a.sequence, b.sequence);
    assert_eq!(a.verdict, b.verdict);
}

#[test]
fn posterior_sums_to_one_under_arbitrary_outcomes() {
    let config = Config {
        flake_rate: 0.05,
        budget: Some(10_000),
        ..Config::default()
    };
    let mut rng = Rng::new(5);
    let mut search = Search::new(&uniform(9), 0.5, &config).unwrap();
    for _ in 0..300 {
        let v = (rng.next_u64() % 9) as usize;
        search.observe(v, rng.next_u64().is_multiple_of(2)).unwrap();
        let sum: f64 = search.posterior().iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "sum {sum}");
        assert!(
            search
                .posterior()
                .iter()
                .all(|p| p.is_finite() && *p >= 0.0)
        );
    }
}

#[test]
fn a_budget_that_runs_out_reports_the_best_guess() {
    let config = Config {
        budget: Some(1),
        ..Config::default()
    };
    let mut search = Search::new(&uniform(16), DEFAULT_NO_CULPRIT_WEIGHT, &config).unwrap();
    let v = search.next_probe().unwrap();
    search.observe(v, true).unwrap();
    assert_eq!(search.next_probe(), None);
    match search.verdict() {
        Verdict::Inconclusive { best, probability } => {
            assert!(probability < 0.95);
            assert!(matches!(
                best,
                Hypothesis::Commit(_) | Hypothesis::NoCulprit
            ));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(default_budget(16), 16);
}

#[test]
fn probing_the_base_again_is_possible() {
    // A single candidate: the only probe is the base, which separates a real
    // culprit (fails again) from a flake (passes).
    let config = Config::default();
    let mut search = Search::new(&[1.0], 1.0, &config).unwrap();
    assert_eq!(search.next_probe(), Some(0));
    search.observe(0, true).unwrap();
    assert!(matches!(search.verdict(), Verdict::Flake { .. }));
}
