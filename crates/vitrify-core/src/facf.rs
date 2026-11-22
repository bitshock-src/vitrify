//! Flake-aware culprit finding (FACF; Henderson, Dorward, Nickell, Johnston &
//! Kondareddy, "Flake Aware Culprit Finding", ICST 2023), as a pure algorithm.
//!
//! A Verification row that fails at a gate or re-check and also fails on the
//! base makes its unit rotten: something between the unit's anchor (the commit
//! of its last recorded check, where the row passed) and the base broke it, or
//! the row is flaky. FACF is a Bayesian noisy binary search over that range.
//! It keeps one probability per candidate commit being the culprit, plus one
//! for "no culprit" (the failures were flakes), probes the failing row at the
//! version that is expected to teach the most, updates on each pass or fail,
//! and stops when one conclusion is confident enough.
//!
//! This module does no I/O. The driver runs each probe in a pristine checkout
//! and feeds the outcome back:
//!
//! ```text
//! let mut search = Search::new(&weights, none_weight, &Config::default())?;
//! while let Some(version) = search.next_probe() {
//!     search.observe(version, run_row_at(version))?;
//! }
//! match search.verdict() { ... }
//! ```
//!
//! # Model
//!
//! Versions are candidate indices, oldest first: version `i` is the tree after
//! candidate `i`, and the last version is the base. The anchor is known to
//! pass and is not a candidate. Under "commit `k` is the culprit" the row
//! fails at every version `>= k`, and at versions `< k` it fails only with the
//! flake rate `f`. Under "no culprit" it fails with probability `f` at every
//! version. A pass at a version `>= k` therefore rules `k` out. The failure at
//! the base that started the search is folded in at construction.

/// Probability the row fails spuriously at a healthy version, when the caller
/// has no measurement. Judgment default: 0.2. Overestimating is the safe
/// side: a high rate only costs probes, while a low one makes a run of flaky
/// failures look like a real break (in simulation, a model rate of 0.1 against
/// a row that really failed 30% of the time blamed a commit in most runs).
pub const DEFAULT_FLAKE_RATE: f64 = 0.2;

/// Posterior a conclusion must reach to stop the search. Judgment default:
/// 0.95, the usual "confident" threshold; higher costs probes roughly linearly
/// in log(1/(1-c)).
pub const DEFAULT_CONFIDENCE: f64 = 0.95;

/// Share of the culprit prior given to commits that changed a path the rotten
/// unit is checked against. Judgment default: nine to one, so declared inputs
/// are searched first yet an undeclared dependency stays findable.
pub const TOUCHED_SHARE: f64 = 0.9;

/// Prior weight of "no culprit" relative to a total culprit weight of 1.
/// Judgment default: 1.0, even odds before the base failure is counted,
/// because a large fraction of the paper's search ranges are flaky; the
/// failure at the base then favors a real break only by 1 : f.
pub const DEFAULT_NO_CULPRIT_WEIGHT: f64 = 1.0;

/// Probes in the default budget: `BUDGET_PER_LOG2 * ceil(log2 n) + BUDGET_EXTRA`.
/// Judgment default: three times the bisection depth leaves room to ride out
/// noise on every step, plus a few probes of the base to confirm a flake.
const BUDGET_PER_LOG2: usize = 3;
const BUDGET_EXTRA: usize = 4;

/// Floor on the flake rate used inside the update, so a flake rate of 0 never
/// makes the evidence contradictory (a failure at a healthy version would
/// otherwise have probability 0 under every hypothesis).
const MIN_FLAKE_RATE: f64 = 1e-9;

/// Entropy gains closer than this are ties, which go to the earliest version.
const TIE_EPS: f64 = 1e-12;

/// Why a search could not be built or an outcome could not be recorded.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FacfError {
    #[error("culprit search needs at least one candidate commit")]
    EmptyRange,
    #[error("prior weight {0} is not a finite non-negative number")]
    BadWeight(f64),
    #[error("prior weights sum to zero")]
    ZeroPrior,
    #[error("flake rate {0} is outside [0, 1)")]
    BadFlakeRate(f64),
    #[error("confidence {0} is outside (0, 1]")]
    BadConfidence(f64),
    #[error("version {version} is outside the range of {candidates} candidates")]
    BadVersion { version: usize, candidates: usize },
}

/// Tunables of a search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Probability the row fails spuriously at a version where it is not
    /// broken, in `[0, 1)`.
    pub flake_rate: f64,
    /// Posterior at which a conclusion is accepted, in `(0, 1]`.
    pub confidence: f64,
    /// Most probes to spend; `None` takes [`default_budget`] for the range.
    pub budget: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            flake_rate: DEFAULT_FLAKE_RATE,
            confidence: DEFAULT_CONFIDENCE,
            budget: None,
        }
    }
}

/// The default probe budget for `n` candidates: a small multiple of the
/// bisection depth plus a few extra. A judgment default (see the constants).
pub fn default_budget(n: usize) -> usize {
    let depth = if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize // ceil(log2 n)
    };
    BUDGET_PER_LOG2 * depth + BUDGET_EXTRA
}

/// One hypothesis about the failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hypothesis {
    /// The candidate at this index (0 = oldest) broke the row.
    Commit(usize),
    /// No commit did: the row is flaky.
    NoCulprit,
}

/// Where the search stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Still searching: not confident, budget left.
    Searching,
    /// The candidate at `index` broke the row, with this posterior.
    Culprit { index: usize, probability: f64 },
    /// The row is flaky: no commit broke it.
    Flake { probability: f64 },
    /// The budget is spent without a confident conclusion; `best` is the most
    /// probable hypothesis.
    Inconclusive { best: Hypothesis, probability: f64 },
}

/// A flake-aware culprit search in progress. Plain data.
#[derive(Debug, Clone)]
pub struct Search {
    /// Posterior over `n` commit hypotheses then "no culprit"; sums to 1.
    posterior: Vec<f64>,
    /// Versions at which the row has been seen to pass.
    passed_at: Vec<bool>,
    flake_rate: f64,
    confidence: f64,
    budget: usize,
    probes: usize,
}

fn check_weight(w: f64) -> Result<(), FacfError> {
    if w.is_finite() && w >= 0.0 {
        Ok(())
    } else {
        Err(FacfError::BadWeight(w))
    }
}

/// `x ln x`, with the limit 0 at 0.
fn xlnx(x: f64) -> f64 {
    if x > 0.0 { x * x.ln() } else { 0.0 }
}

impl Search {
    /// Start a search over `weights.len()` candidates (oldest first) with a
    /// prior weight per candidate and one for "no culprit". Weights are
    /// relative; they need not sum to 1. The failure at the base is folded in.
    pub fn new(weights: &[f64], none_weight: f64, config: &Config) -> Result<Search, FacfError> {
        if weights.is_empty() {
            return Err(FacfError::EmptyRange);
        }
        for &w in weights.iter().chain(std::iter::once(&none_weight)) {
            check_weight(w)?;
        }
        let f = config.flake_rate;
        if !(f.is_finite() && (0.0..1.0).contains(&f)) {
            return Err(FacfError::BadFlakeRate(f));
        }
        let c = config.confidence;
        if !(c.is_finite() && c > 0.0 && c <= 1.0) {
            return Err(FacfError::BadConfidence(c));
        }
        let mut posterior: Vec<f64> = weights.iter().copied().chain([none_weight]).collect();
        let total: f64 = posterior.iter().sum();
        if !(total.is_finite() && total > 0.0) {
            return Err(FacfError::ZeroPrior);
        }
        for p in &mut posterior {
            *p /= total;
        }
        let mut search = Search {
            posterior,
            passed_at: vec![false; weights.len()],
            flake_rate: f,
            confidence: c,
            budget: config
                .budget
                .unwrap_or_else(|| default_budget(weights.len())),
            probes: 0,
        };
        // The failure at the base that started the search is evidence too.
        search.update(weights.len() - 1, false);
        Ok(search)
    }

    /// Start a search with the prior taken from the declared dependencies:
    /// `touched[i]` says whether candidate `i` changed a path the rotten unit
    /// is checked against. Touched commits share [`TOUCHED_SHARE`] of the
    /// culprit mass equally; every other commit shares the rest, so a culprit
    /// that reached the unit through an undeclared dependency is still found.
    /// When all or none are touched the mass spreads evenly. "No culprit"
    /// gets [`DEFAULT_NO_CULPRIT_WEIGHT`]. All are judgment defaults.
    pub fn from_touched(touched: &[bool], config: &Config) -> Result<Search, FacfError> {
        let hot = touched.iter().filter(|&&t| t).count();
        let cold = touched.len() - hot;
        let (hot_share, cold_share) = match (hot, cold) {
            (0, _) => (0.0, 1.0),
            (_, 0) => (1.0, 0.0),
            _ => (TOUCHED_SHARE, 1.0 - TOUCHED_SHARE),
        };
        let weights: Vec<f64> = touched
            .iter()
            .map(|&t| {
                if t {
                    hot_share / hot as f64
                } else {
                    cold_share / cold as f64
                }
            })
            .collect();
        Search::new(&weights, DEFAULT_NO_CULPRIT_WEIGHT, config)
    }

    /// Number of candidate commits.
    pub fn candidates(&self) -> usize {
        self.posterior.len() - 1
    }

    /// The posterior: one probability per candidate (oldest first) followed by
    /// the probability of "no culprit". Always sums to 1.
    pub fn posterior(&self) -> &[f64] {
        &self.posterior
    }

    /// Posterior that the candidate at `index` is the culprit (0 if out of range).
    pub fn commit_probability(&self, index: usize) -> f64 {
        if index < self.candidates() {
            self.posterior[index]
        } else {
            0.0
        }
    }

    /// Posterior that the row is flaky and no commit is the culprit.
    pub fn no_culprit_probability(&self) -> f64 {
        self.posterior[self.candidates()]
    }

    /// Probes observed so far (the base failure that started the search is not
    /// counted).
    pub fn probes_used(&self) -> usize {
        self.probes
    }

    /// The probe budget.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// The most probable hypothesis and its probability; ties go to the
    /// earliest commit, then "no culprit".
    fn best(&self) -> (Hypothesis, f64) {
        let mut at = 0;
        for (i, &p) in self.posterior.iter().enumerate() {
            if p > self.posterior[at] {
                at = i;
            }
        }
        let hyp = if at < self.candidates() {
            Hypothesis::Commit(at)
        } else {
            Hypothesis::NoCulprit
        };
        (hyp, self.posterior[at])
    }

    /// A culprit is bracketed when the version just before it has been seen to
    /// pass (the anchor, before candidate 0, is known to pass). Without that
    /// a prior that favors the candidate could carry it over the confidence
    /// threshold on its own, leaving an earlier, unfavored commit untested.
    fn bracketed(&self, index: usize) -> bool {
        index == 0 || self.passed_at.get(index - 1).copied().unwrap_or(false)
    }

    /// Where the search stands. A confident conclusion wins over a spent
    /// budget. A culprit counts as concluded only once it is bracketed (see
    /// [`Search::next_probe`]); a flake needs nothing more.
    pub fn verdict(&self) -> Verdict {
        let (best, probability) = self.best();
        if probability >= self.confidence {
            match best {
                Hypothesis::Commit(index) if self.bracketed(index) => {
                    return Verdict::Culprit { index, probability };
                }
                Hypothesis::NoCulprit => return Verdict::Flake { probability },
                Hypothesis::Commit(_) => {}
            }
        }
        if self.probes >= self.budget {
            return Verdict::Inconclusive { best, probability };
        }
        Verdict::Searching
    }

    /// The version to run the failing row at next: the one whose outcome is
    /// expected to reduce the entropy of the posterior the most. Ties go to
    /// the earliest version; the base itself is a legal probe (it separates a
    /// real culprit from a flake). When a commit already holds the confidence
    /// threshold but the version before it has not been seen to pass, that
    /// version is probed instead, so a culprit is never named on the prior
    /// alone. `None` once concluded or out of budget.
    pub fn next_probe(&self) -> Option<usize> {
        if self.verdict() != Verdict::Searching {
            return None;
        }
        if let (Hypothesis::Commit(k), p) = self.best()
            && p >= self.confidence
        {
            // invariant: unbracketed means k >= 1, so k - 1 is a candidate.
            return Some(k - 1);
        }
        let n = self.candidates();
        let f = self.flake_rate.max(MIN_FLAKE_RATE);
        let ln_f = f.ln();
        let ln_g = (1.0 - f).ln();
        let total_plnp: f64 = self.posterior.iter().map(|&p| xlnx(p)).sum();
        let current = -total_plnp;
        // Running mass and p ln p of the commits <= v (broken at v); the rest
        // (later commits and "no culprit") are healthy at v.
        let (mut low_mass, mut low_plnp) = (0.0, 0.0);
        let mut best: Option<(usize, f64)> = None;
        for v in 0..n {
            low_mass += self.posterior[v];
            low_plnp += xlnx(self.posterior[v]);
            let high_mass = (1.0 - low_mass).max(0.0);
            let high_plnp = total_plnp - low_plnp;
            // Expected entropy after the probe is sum over outcomes o of
            // P(o) ln P(o) - sum_h q_h ln q_h, with q_h = P(h) P(o | h).
            // Fail: commits <= v keep weight p, the rest weigh f * p.
            let p_fail = low_mass + f * high_mass;
            let fail_term = xlnx(p_fail) - (low_plnp + f * (high_plnp + ln_f * high_mass));
            // Pass: commits <= v are ruled out, the rest weigh (1-f) * p.
            let p_pass = (1.0 - f) * high_mass;
            let pass_term = xlnx(p_pass) - (1.0 - f) * (high_plnp + ln_g * high_mass);
            let gain = current - (fail_term + pass_term);
            match best {
                Some((_, g)) if gain <= g + TIE_EPS => {}
                _ => best = Some((v, gain)),
            }
        }
        best.map(|(v, _)| v)
    }

    /// Record the outcome of running the row at `version`. The probe counts
    /// against the budget. Errors only if `version` is not a candidate index.
    pub fn observe(&mut self, version: usize, passed: bool) -> Result<(), FacfError> {
        if version >= self.candidates() {
            return Err(FacfError::BadVersion {
                version,
                candidates: self.candidates(),
            });
        }
        self.probes += 1;
        if passed {
            self.passed_at[version] = true;
        }
        self.update(version, passed);
        Ok(())
    }

    /// Bayes update with the likelihood of the outcome under each hypothesis,
    /// then renormalize. Evidence no hypothesis can explain (total weight 0)
    /// is ignored rather than collapsing the posterior.
    fn update(&mut self, version: usize, passed: bool) {
        let f = self.flake_rate.max(MIN_FLAKE_RATE);
        let n = self.candidates();
        let mut next = self.posterior.clone();
        for (k, p) in next.iter_mut().enumerate() {
            let broken = k < n && k <= version;
            let like = match (broken, passed) {
                (true, true) => 0.0,
                (true, false) => 1.0,
                (false, true) => 1.0 - f,
                (false, false) => f,
            };
            *p *= like;
        }
        let total: f64 = next.iter().sum();
        if total.is_finite() && total > 0.0 {
            for p in &mut next {
                *p /= total;
            }
            self.posterior = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(n: usize) -> Vec<f64> {
        vec![1.0; n]
    }

    #[test]
    fn rejects_bad_input() {
        let c = Config::default();
        assert_eq!(
            Search::new(&[], 1.0, &c).unwrap_err(),
            FacfError::EmptyRange
        );
        assert!(matches!(
            Search::new(&[1.0, f64::NAN], 1.0, &c),
            Err(FacfError::BadWeight(_))
        ));
        assert!(matches!(
            Search::new(&[1.0, -1.0], 1.0, &c),
            Err(FacfError::BadWeight(_))
        ));
        assert!(matches!(
            Search::new(&[1.0], f64::INFINITY, &c),
            Err(FacfError::BadWeight(_))
        ));
        assert_eq!(
            Search::new(&[0.0, 0.0], 0.0, &c).unwrap_err(),
            FacfError::ZeroPrior
        );
        for f in [-0.1, 1.0, 2.0, f64::NAN] {
            let bad = Config { flake_rate: f, ..c };
            assert!(matches!(
                Search::new(&[1.0], 1.0, &bad),
                Err(FacfError::BadFlakeRate(_))
            ));
        }
        for conf in [0.0, 1.5, f64::NAN] {
            let bad = Config {
                confidence: conf,
                ..c
            };
            assert!(matches!(
                Search::new(&[1.0], 1.0, &bad),
                Err(FacfError::BadConfidence(_))
            ));
        }
    }

    #[test]
    fn observe_rejects_unknown_version() {
        let mut s = Search::new(&uniform(3), 1.0, &Config::default()).unwrap();
        assert!(matches!(
            s.observe(3, true),
            Err(FacfError::BadVersion { .. })
        ));
        assert_eq!(s.probes_used(), 0);
    }

    #[test]
    fn base_failure_is_folded_in() {
        let s = Search::new(&uniform(2), 2.0, &Config::default()).unwrap();
        // prior 1/4, 1/4, 1/2; the base failure has likelihood 1, 1, f.
        let f = DEFAULT_FLAKE_RATE;
        let z = 0.25 + 0.25 + 0.5 * f;
        assert!((s.posterior()[0] - 0.25 / z).abs() < 1e-12);
        assert!((s.no_culprit_probability() - 0.5 * f / z).abs() < 1e-12);
        assert_eq!(s.probes_used(), 0);
    }

    #[test]
    fn a_pass_rules_out_commits_at_or_before_it() {
        let c = Config {
            flake_rate: 0.0,
            ..Config::default()
        };
        let mut s = Search::new(&uniform(4), 0.0, &c).unwrap();
        s.observe(1, true).unwrap();
        assert!(s.commit_probability(0) < 1e-6 && s.commit_probability(1) < 1e-6);
        assert!((s.commit_probability(2) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn contradictory_evidence_keeps_the_posterior() {
        let c = Config {
            flake_rate: 0.0,
            ..Config::default()
        };
        let mut s = Search::new(&uniform(2), 0.0, &c).unwrap();
        // A pass at the base contradicts the failure there under every
        // hypothesis; the posterior stays a distribution.
        s.observe(1, true).unwrap();
        let sum: f64 = s.posterior().iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn first_probe_of_a_clean_range_is_the_midpoint() {
        let c = Config {
            flake_rate: 0.0,
            ..Config::default()
        };
        let s = Search::new(&uniform(8), 0.0, &c).unwrap();
        assert_eq!(s.next_probe(), Some(3));
    }

    #[test]
    fn default_budget_grows_with_log2() {
        assert_eq!(default_budget(1), 4);
        assert_eq!(default_budget(2), 7);
        assert_eq!(default_budget(8), 13);
        assert_eq!(default_budget(9), 16);
    }

    #[test]
    fn budget_spent_is_inconclusive() {
        let c = Config {
            budget: Some(0),
            ..Config::default()
        };
        let s = Search::new(&uniform(10), 1.0, &c).unwrap();
        assert_eq!(s.next_probe(), None);
        assert!(matches!(s.verdict(), Verdict::Inconclusive { .. }));
    }

    #[test]
    fn a_culprit_is_not_named_on_the_prior_alone() {
        // Candidate 3 carries nearly all the prior, but nothing has shown that
        // the version before it passes: that is probed before concluding.
        let c = Config {
            flake_rate: 0.0,
            ..Config::default()
        };
        let s = Search::new(&[0.01, 0.01, 0.01, 100.0], 0.0, &c).unwrap();
        assert!(s.commit_probability(3) > c.confidence);
        assert_eq!(s.verdict(), Verdict::Searching);
        assert_eq!(s.next_probe(), Some(2));
        let mut s = s;
        s.observe(2, true).unwrap();
        assert!(matches!(s.verdict(), Verdict::Culprit { index: 3, .. }));
        assert_eq!(s.next_probe(), None);
    }

    #[test]
    fn from_touched_splits_the_prior() {
        let s = Search::from_touched(&[true, false, false, true], &Config::default()).unwrap();
        let p = s.posterior();
        assert!((p[0] - p[3]).abs() < 1e-12);
        assert!((p[1] - p[2]).abs() < 1e-12);
        assert!((p[0] / p[1] - 9.0).abs() < 1e-9);
        let none = Search::from_touched(&[false, false], &Config::default()).unwrap();
        assert!((none.posterior()[0] - none.posterior()[1]).abs() < 1e-12);
    }
}
