//! Culprit finding for rotten units: probes in a pristine checkout.
//!
//! A unit is rotten when one of its Verification rows fails on the base too
//! ([`super::blame`]): the batch at hand did not break it, so something
//! already on mainline did — or the row is flaky. [`find`] runs
//! flake-aware culprit finding ([`vitrify_core::facf`]) over the commits in
//! between and reports a culprit commit, a flake, or no conclusion.
//!
//! # Candidates
//!
//! The candidates are the mainline commits after the unit's **anchor** — the
//! commit of its latest `verified` journal event, where the row last passed
//! — up to and including the base, oldest first. They are taken from the
//! **first-parent** history of the base. Mainline moves only by integration
//! (`--no-ff` merges of attempt branches, then the integration commit),
//! re-check commits and claim transactions, so its first-parent chain is
//! exactly the sequence of trees mainline has held. The commits inside a
//! merged attempt branch never were mainline: they are a worker's partial
//! states, often unbuildable, and probing them would blame a step of a
//! worker's session rather than the integration that landed it. A merge is
//! judged by its diff against its first parent — everything the integration
//! brought in.
//!
//! # Prior
//!
//! Each candidate's own diff against its first parent goes through the
//! re-check cascade ([`cascade::dirty_set`]) with the unit's closure and the
//! repository's [`Ownership`]: a candidate is **touched** when it changed a
//! path the unit is checked against. [`facf::Search::from_touched`] gives
//! touched candidates almost all of the prior and every other candidate a
//! small share, so a culprit that reached the unit through an undeclared
//! dependency is still found, only later.
//!
//! # Probes
//!
//! Each probe prepares the pristine probe checkout at the candidate (reset,
//! no untracked or ignored file), runs the SDD's §5 Bootstrap and Build
//! there, then the row, all through the injected [`Runner`], and feeds the
//! row's pass or fail to the search, until it concludes or its budget is
//! spent. A probe whose Bootstrap or Build fails cannot run the row; it is
//! **counted as a failure of the row**, with a note. A tree that does not
//! build fails every row run on it, so that is what the row would have
//! done; and the search's flake model absorbs a setup that failed only by
//! chance, as it absorbs any spurious failure. Skipping the version instead
//! would leave the search choosing it again. When the search ends the
//! checkout is prepared once more at the base, so it is left pristine.
//!
//! # Running it
//!
//! [`find`] blocks for as long as its probes take. It is meant for a thread
//! of its own, off the critical path: the rotten unit and its dependents
//! wait either way. Give each lane its own probe checkout
//! (`<worktrees>/probe-<slug>` by default, [`probe_checkout`]), never the
//! lane's integration checkout, and count each probe command against the
//! gate pool as the caller does for gate commands.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::cascade::{self, Candidate, Closures, Owner, Ownership};
use vitrify_core::facf::{self, Hypothesis, Search, Verdict};
use vitrify_core::graph::Node;

use super::blame::{Prepare, Runner};
use crate::git::Git;

/// The default probe checkout of the lane `slug`, under the worktree root.
pub fn probe_checkout(worktrees: &Path, slug: &str) -> PathBuf {
    worktrees.join(format!("probe-{slug}"))
}

/// A rotten unit's failing row, and where to search.
#[derive(Debug, Clone)]
pub struct Rotten<'a> {
    pub node: &'a Node,
    /// The row's 1-based position in the unit's Verification table, as the
    /// report names it.
    pub row_number: usize,
    /// The row's command.
    pub row: &'a str,
    /// The unit's anchor: the commit of its latest `verified` event.
    pub anchor: &'a str,
    /// The base the row failed on.
    pub base: &'a str,
}

/// How to probe.
pub struct Probe<'a> {
    /// Any checkout of the repository, to read history from.
    pub git: &'a Git,
    pub ownership: &'a Ownership,
    /// The dependency closures; the rotten unit's entry is the one used.
    pub closures: &'a Closures,
    /// The SDD's §5 Bootstrap and Build, those it declares, in order.
    pub setup: &'a [String],
    pub prepare: &'a dyn Prepare,
    pub runner: &'a dyn Runner,
    pub config: facf::Config,
}

/// A commit named as the culprit (or the most likely one).
#[derive(Debug, Clone, PartialEq)]
pub struct Culprit {
    pub sha: String,
    pub short: String,
    pub subject: String,
    /// The units it integrated (from an `sdd: integrate …` message), or else
    /// the units owning paths it changed; sorted.
    pub upstream: Vec<Node>,
    /// Whether `upstream` comes from an integration message.
    pub integrated: bool,
    pub probability: f64,
}

impl Culprit {
    /// `core U-004 integrated`, `changed core U-004`, or the subject.
    pub fn describe(&self) -> String {
        let units = self
            .upstream
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        match (self.upstream.is_empty(), self.integrated) {
            (true, _) => format!("\"{}\"", self.subject),
            (false, true) => format!("{units} integrated"),
            (false, false) => format!("changed {units}"),
        }
    }
}

/// What the search concluded.
#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// This commit broke the row.
    Culprit(Culprit),
    /// No commit broke the row: it is flaky. The unit is re-checked again
    /// later; the project should fix the row.
    Flake { probability: f64 },
    /// No confident conclusion within the budget (or no candidate at all);
    /// `best` is the most likely culprit, when a commit is the most likely
    /// hypothesis.
    Inconclusive {
        best: Option<Culprit>,
        probability: f64,
    },
    /// The run was interrupted mid-search.
    Interrupted,
}

/// One culprit search's report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub node: Node,
    pub row_number: usize,
    pub row: String,
    pub finding: Finding,
    /// Candidate commits searched, and how many of them were touched.
    pub candidates: usize,
    pub touched: usize,
    pub probes: usize,
    /// Probes that could not run the row (setup failed), one line each.
    pub notes: Vec<String>,
}

impl Report {
    /// One line naming the unit, e.g. `api U-002 row 2 broke at a1b2c3d
    /// (core U-004 integrated) — culprit found`.
    pub fn line(&self) -> String {
        format!("{} {}", self.node, self.finding_line())
    }

    /// The finding without the unit, for where the unit is already given,
    /// e.g. `row 2 broke at a1b2c3d (core U-004 integrated) — culprit found`.
    pub fn finding_line(&self) -> String {
        let who = format!("row {}", self.row_number);
        match &self.finding {
            Finding::Culprit(c) => {
                format!(
                    "{who} broke at {} ({}) — culprit found",
                    c.short,
                    c.describe()
                )
            }
            Finding::Flake { .. } => format!(
                "{who} is flaky: no commit since its last check broke it — fix the row (`{}`); the unit is re-checked again",
                self.row
            ),
            Finding::Inconclusive { best, .. } => match best {
                Some(c) => format!(
                    "{who} broke, culprit not found after {} probes (most likely {}, {}) — inconclusive",
                    self.probes,
                    c.short,
                    c.describe()
                ),
                None if self.candidates == 0 && !self.notes.is_empty() => format!(
                    "{who} broke, culprit not searched ({}) — inconclusive",
                    self.notes.join("; ")
                ),
                None if self.candidates == 0 => format!(
                    "{who} fails at its own last check — no commit to search — inconclusive"
                ),
                None => format!(
                    "{who} broke, culprit not found after {} probes (most likely flaky) — inconclusive",
                    self.probes
                ),
            },
            Finding::Interrupted => format!("{who}: culprit search interrupted"),
        }
    }
}

/// Search for the commit that broke `rotten.row`. See the module doc for the
/// candidates, the prior and the probes. Errors on git failures and a probe
/// checkout that cannot be prepared.
pub fn find(rotten: &Rotten<'_>, probe: &Probe<'_>) -> Result<Report, CoreError> {
    let git = probe.git;
    let mut report = Report {
        node: rotten.node.clone(),
        row_number: rotten.row_number,
        row: rotten.row.to_string(),
        finding: Finding::Inconclusive {
            best: None,
            probability: 0.0,
        },
        candidates: 0,
        touched: 0,
        probes: 0,
        notes: Vec::new(),
    };
    // A recorded anchor is an abbreviated sha: peeled to a commit, so a
    // prefix that a blob or tree shares is not ambiguous.
    let Some(anchor) = git.resolve_commit(rotten.anchor).map_err(CoreError::from)? else {
        report.notes.push(format!(
            "its recorded anchor {} names no commit here",
            rotten.anchor
        ));
        return Ok(report);
    };
    let base = git.rev_parse(rotten.base).map_err(CoreError::from)?;
    let commits = git
        .first_parent_range(&anchor, &base)
        .map_err(CoreError::from)?;
    report.candidates = commits.len();
    if commits.is_empty() {
        return Ok(report);
    }

    // the prior: which candidates changed a path the unit is checked against
    let closure: BTreeSet<Node> = probe.closures.get(rotten.node).cloned().unwrap_or_default();
    let closures: Closures = [(rotten.node.clone(), closure)].into();
    let mut diffs: Vec<Vec<String>> = Vec::with_capacity(commits.len());
    let mut touched: Vec<bool> = Vec::with_capacity(commits.len());
    for c in &commits {
        let parent = format!("{c}^1");
        let paths = git.changed_paths(&parent, c).map_err(CoreError::from)?;
        let changes: cascade::Changes = [(parent.clone(), paths.clone())].into();
        let cand = [Candidate {
            node: rotten.node.clone(),
            anchor: Some(parent),
        }];
        touched.push(!cascade::dirty_set(&cand, &changes, &closures, probe.ownership).is_empty());
        diffs.push(paths);
    }
    report.touched = touched.iter().filter(|&&t| t).count();
    let mut search = Search::from_touched(&touched, &probe.config)
        .map_err(|e| CoreError::refusal(1, format!("error: culprit search: {e}")))?;

    let mut interrupted = false;
    while let Some(version) = search.next_probe() {
        let commit = &commits[version];
        let dir = probe.prepare.prepare(commit)?;
        let Some(passed) = run_probe(probe, &dir, rotten.row, commit, &mut report.notes) else {
            interrupted = true;
            break;
        };
        search
            .observe(version, passed)
            .map_err(|e| CoreError::refusal(1, format!("error: culprit search: {e}")))?;
    }
    report.probes = search.probes_used();
    // leave the probe checkout pristine, at the base
    probe.prepare.prepare(&base)?;

    let culprit = |index: usize, probability: f64| -> Result<Culprit, CoreError> {
        describe(
            git,
            probe.ownership,
            &commits[index],
            &diffs[index],
            probability,
        )
    };
    report.finding = if interrupted {
        Finding::Interrupted
    } else {
        match search.verdict() {
            Verdict::Culprit { index, probability } => {
                Finding::Culprit(culprit(index, probability)?)
            }
            Verdict::Flake { probability } => Finding::Flake { probability },
            Verdict::Inconclusive { best, probability } => Finding::Inconclusive {
                best: match best {
                    Hypothesis::Commit(index) => Some(culprit(index, probability)?),
                    Hypothesis::NoCulprit => None,
                },
                probability,
            },
            // invariant: the loop ends only once the search is not Searching
            Verdict::Searching => Finding::Inconclusive {
                best: None,
                probability: 0.0,
            },
        }
    };
    Ok(report)
}

/// One probe in the prepared checkout `dir`: setup, then the row. `None`
/// when interrupted; a failed setup counts as a failure of the row.
fn run_probe(
    probe: &Probe<'_>,
    dir: &Path,
    row: &str,
    commit: &str,
    notes: &mut Vec<String>,
) -> Option<bool> {
    for cmd in probe.setup {
        let ran = probe.runner.run(cmd, dir);
        if ran.interrupted {
            return None;
        }
        if !ran.ok() {
            notes.push(format!(
                "`{cmd}` failed at {} (exit {}): counted as a failure of the row",
                &commit[..7.min(commit.len())],
                ran.code
            ));
            return Some(false);
        }
    }
    let ran = probe.runner.run(row, dir);
    if ran.interrupted {
        return None;
    }
    Some(ran.ok())
}

/// Name a commit: short sha, subject, and the upstream units it changed.
fn describe(
    git: &Git,
    ownership: &Ownership,
    sha: &str,
    paths: &[String],
    probability: f64,
) -> Result<Culprit, CoreError> {
    let subject = git.subject(sha).map_err(CoreError::from)?;
    let short = git.short_sha(sha).map_err(CoreError::from)?;
    let mut owners: BTreeSet<Node> = BTreeSet::new();
    let mut spec_slugs: BTreeSet<String> = BTreeSet::new();
    for p in paths {
        match ownership.classify(p) {
            Owner::Unit(n) | Owner::Test { unit: Some(n), .. } => {
                owners.insert(n);
            }
            Owner::Spec(slug) => {
                spec_slugs.insert(slug);
            }
            _ => {}
        }
    }
    let integrated = integrated_units(&subject);
    let (upstream, from_message) = if integrated.is_empty() {
        (owners.iter().cloned().collect::<Vec<_>>(), false)
    } else {
        // the message names unit ids; their SDD is the one whose units — or
        // whose spec — the merge changed
        let slugs: BTreeSet<String> = owners
            .iter()
            .map(|n| n.slug.clone())
            .chain(spec_slugs)
            .collect();
        let mut nodes: Vec<Node> = Vec::new();
        for unit in &integrated {
            let mut found = false;
            for n in owners.iter().filter(|n| &n.unit == unit) {
                nodes.push(n.clone());
                found = true;
            }
            if !found
                && slugs.len() == 1
                && let Some(slug) = slugs.first()
            {
                nodes.push(Node::new(slug.as_str(), unit.as_str()));
            }
        }
        nodes.sort();
        nodes.dedup();
        if nodes.is_empty() {
            (owners.into_iter().collect(), false)
        } else {
            (nodes, true)
        }
    };
    Ok(Culprit {
        sha: sha.to_string(),
        short,
        subject,
        upstream,
        integrated: from_message,
        probability,
    })
}

/// The unit ids an integration's subject names: `sdd: integrate U-004 (c3)`
/// names `U-004`; an integration commit `sdd: verify U-001 (c1), U-002 (c4)`
/// names both. Empty for any other subject.
fn integrated_units(subject: &str) -> Vec<String> {
    let Some(rest) = subject
        .strip_prefix("sdd: integrate ")
        .or_else(|| subject.strip_prefix("sdd: verify "))
    else {
        return Vec::new();
    };
    rest.split(", ")
        .filter_map(|part| part.split_whitespace().next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_subjects_name_their_units() {
        assert_eq!(integrated_units("sdd: integrate U-004 (c3)"), vec!["U-004"]);
        assert_eq!(
            integrated_units("sdd: verify U-001 (c1), U-002 (c4)"),
            vec!["U-001", "U-002"]
        );
        assert!(integrated_units("fix the parser").is_empty());
    }

    #[test]
    fn a_culprit_is_described_by_what_it_integrated_or_changed() {
        let mut c = Culprit {
            sha: "a1b2c3d4".into(),
            short: "a1b2c3d".into(),
            subject: "sdd: integrate U-004 (c3)".into(),
            upstream: vec![Node::new("core", "U-004")],
            integrated: true,
            probability: 0.97,
        };
        assert_eq!(c.describe(), "core U-004 integrated");
        c.integrated = false;
        assert_eq!(c.describe(), "changed core U-004");
        c.upstream.clear();
        assert_eq!(c.describe(), "\"sdd: integrate U-004 (c3)\"");
        let report = Report {
            node: Node::new("api", "U-002"),
            row_number: 2,
            row: "make check".into(),
            finding: Finding::Culprit(Culprit {
                upstream: vec![Node::new("core", "U-004")],
                integrated: true,
                ..c
            }),
            candidates: 3,
            touched: 1,
            probes: 2,
            notes: Vec::new(),
        };
        assert_eq!(
            report.line(),
            "api U-002 row 2 broke at a1b2c3d (core U-004 integrated) — culprit found"
        );
    }
}
