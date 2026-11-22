//! Incremental builds against real repositories: dirty units from git (a
//! protocol-state-only commit, a foundation change reaching its SDD and a
//! dependent SDD through a pin, byte-identical regeneration, a spec-only
//! edit, a change outside every scope, an anchor git cannot resolve, the
//! distrust-everything mode, a cycle), and re-check batches (passing units
//! recorded as `verified`/`recheck` with fresh stamps, the suite recorded,
//! replay and audit clean, nothing dirty afterwards; a failing row records
//! nothing for its unit).

mod common;

use std::path::{Path, PathBuf};

use common::{Project, git};
use vitrify_cli::git::Git;
use vitrify_cli::incremental::dirty::{self, Cascade, DirtySet, Mode, Snapshot};
use vitrify_cli::incremental::recheck::{self, ShRunner};
use vitrify_core::cascade::{Owner, Why};
use vitrify_core::graph::Node;
use vitrify_core::model::{self, Instance};

/// The cascade at `checkout`'s `HEAD`.
fn cascade_at(checkout: &Path, mode: Mode) -> Cascade {
    let git = Git::new(checkout);
    let snap = Snapshot::load(&git, checkout, "HEAD").unwrap();
    snap.dirty(&git, mode).unwrap()
}

fn dirty_at(checkout: &Path) -> DirtySet {
    match cascade_at(checkout, Mode::Changes) {
        Cascade::Resolved(set) => set,
        Cascade::Cycles(c) => panic!("unexpected cycles: {c:?}"),
    }
}

/// `slug unit` of every dirty unit.
fn names(set: &DirtySet) -> Vec<String> {
    set.units.iter().map(|d| d.node.to_string()).collect()
}

/// Give the unit file at `rel` another `verified:` stamp: a managed field.
fn restamp(p: &Project, rel: &str) {
    let unit = p.read(rel);
    let stamped: Vec<String> = unit
        .lines()
        .map(|l| {
            if l.starts_with("verified:") {
                "verified: 2030-01-01 @ SDD 1.0.0, commit 1234567".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    p.write(rel, &format!("{}\n", stamped.join("\n")));
}

#[test]
fn a_freshly_verified_repository_has_nothing_dirty() {
    let p = Project::new();
    let set = dirty_at(&p.root);
    assert!(set.units.is_empty(), "{:?}", set.units);
    assert_eq!(set.head, p.head());
    // no suite pass recorded yet: both suites must run
    let suites: Vec<(&str, Option<&Why>)> = set
        .suites
        .iter()
        .map(|s| (s.slug.as_str(), s.dirty.as_ref()))
        .collect();
    assert_eq!(
        suites,
        [
            ("alpha", Some(&Why::NoAnchor)),
            ("beta", Some(&Why::NoAnchor))
        ]
    );
    // and the seeded state is consistent
    for sdd in ["specs/alpha", "specs/beta"] {
        let (ok, out, err) = p.vitrify(&["journal", "replay", "--sdd", sdd]);
        assert!(ok, "replay {sdd}: {out}{err}");
        let (ok, out, err) = p.vitrify(&["audit", "--sdd", sdd]);
        assert!(ok, "audit {sdd}: {out}{err}");
    }
}

#[test]
fn a_protocol_state_only_commit_dirties_nothing() {
    let p = Project::new();
    // a journal event, a regenerated ledger, a decision recorded, and a unit
    // entry that differs only in managed fields
    p.event("specs/alpha", "audit", None, "audit", &p.anchor);
    p.write(
        "specs/alpha/decisions.md",
        "# Decisions\n\n- **Q-001** (2026-11-01, open) \u{2014} which?\n",
    );
    restamp(&p, "specs/alpha/units/U-002-two.md");
    p.commit("state");
    let git = Git::new(&p.root);
    let snap = Snapshot::load(&git, &p.root, "HEAD").unwrap();
    let anchor = git.resolve_commit(&p.anchor).unwrap().unwrap();
    let classified = dirty::classify_changes(&git, &anchor, &p.head(), &snap.spec).unwrap();
    assert!(
        classified
            .iter()
            .any(|(path, state)| path == "specs/alpha/units/U-002-two.md" && *state),
        "{classified:?}"
    );
    assert!(
        dirty::benign(&git, &anchor, &p.head(), &snap.spec).unwrap(),
        "{classified:?}"
    );
    assert!(dirty_at(&p.root).units.is_empty());
}

#[test]
fn a_foundation_change_dirties_its_sdd_and_its_dependents_across_a_pin() {
    let p = Project::new();
    p.write("a/one.txt", "one, again\n");
    p.commit("foundation");
    let set = dirty_at(&p.root);
    assert_eq!(names(&set), ["alpha U-001", "alpha U-002", "beta U-001"]);
    for d in &set.units {
        let Why::Paths(reasons) = &d.why else {
            panic!("{d:?}");
        };
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].path, "a/one.txt");
        assert_eq!(reasons[0].owner, Owner::Unit(Node::new("alpha", "U-001")));
        assert_eq!(dirty::describe(&d.why), "a/one.txt (unit alpha U-001)");
    }
    assert!(!set.is_dirty(&Node::new("beta", "U-002")));
    assert_eq!(set.of_sdd("beta").count(), 1);
    // a change to a leaf reaches only the leaf
    p.write("b/two.txt", "b two, again\n");
    p.commit("leaf");
    let set = dirty_at(&p.root);
    assert_eq!(
        names(&set),
        ["alpha U-001", "alpha U-002", "beta U-001", "beta U-002"]
    );
}

#[test]
fn byte_identical_regeneration_dirties_nothing() {
    let p = Project::new();
    // the unit is regenerated: its file rewritten, its entry restamped…
    p.write("a/one.txt", "something else\n");
    p.commit("attempt");
    p.write("a/one.txt", "one\n");
    restamp(&p, "specs/alpha/units/U-001-one.md");
    p.commit("regenerated");
    // …into the same bytes: no path differs from the anchor
    assert!(dirty_at(&p.root).units.is_empty());
}

#[test]
fn a_spec_only_edit_dirties_nothing() {
    let p = Project::new();
    let unit = p.read("specs/alpha/units/U-001-one.md");
    p.write(
        "specs/alpha/units/U-001-one.md",
        &unit.replace("THE SYSTEM SHALL ship", "THE SYSTEM SHALL always ship"),
    );
    let sdd = p.read("specs/beta/sdd.md");
    p.write("specs/beta/sdd.md", &sdd.replace("G.", "G, reworded."));
    p.commit("spec");
    // the regeneration cascade's business, never the re-check cascade's
    assert!(dirty_at(&p.root).units.is_empty());
}

#[test]
fn a_change_outside_every_scope_dirties_everything() {
    let p = Project::new();
    p.write("tools/build.sh", "true\n");
    p.commit("tool");
    let set = dirty_at(&p.root);
    assert_eq!(set.units.len(), 4);
    assert_eq!(
        dirty::describe(&set.units[0].why),
        "tools/build.sh (outside every scope)"
    );
}

#[test]
fn an_anchor_git_cannot_resolve_dirties_its_units() {
    let p = Project::new();
    // history rewritten under the record: beta U-002's anchor is gone
    p.event(
        "specs/beta",
        "verified",
        Some("U-002"),
        "integrator",
        "deadbee",
    );
    let set = dirty_at(&p.root);
    assert_eq!(names(&set), ["beta U-002"]);
    assert_eq!(set.units[0].why, Why::NoDiff("deadbee".into()));
    assert_eq!(set.unresolved, ["deadbee"]);
    assert_eq!(
        dirty::describe(&set.units[0].why),
        "anchor deadbee cannot be resolved (rewritten history, or a shallow clone)"
    );
}

#[test]
fn everything_mode_distrusts_every_recorded_check() {
    let p = Project::new();
    let Cascade::Resolved(set) = cascade_at(&p.root, Mode::Everything) else {
        panic!("cycles");
    };
    assert_eq!(
        names(&set),
        ["alpha U-001", "alpha U-002", "beta U-001", "beta U-002"]
    );
    assert!(set.units.iter().all(|d| d.why == Why::Everything));
    assert!(set.suites.iter().all(|s| s.dirty == Some(Why::Everything)));
}

#[test]
fn a_cycle_is_reported_instead_of_a_guess() {
    let p = Project::new();
    let unit = p.read("specs/alpha/units/U-001-one.md");
    p.write(
        "specs/alpha/units/U-001-one.md",
        &unit.replace("depends_on: []", "depends_on: [U-002]"),
    );
    p.commit("cycle");
    let Cascade::Cycles(cycles) = cascade_at(&p.root, Mode::Changes) else {
        panic!("expected cycles");
    };
    assert_eq!(
        cycles.cycles,
        [vec![
            Node::new("alpha", "U-001"),
            Node::new("alpha", "U-002"),
            Node::new("alpha", "U-001")
        ]]
    );
    assert_eq!(cycles.waiting, [Node::new("beta", "U-001")]);
    let (_, _, err) = p.vitrify(&["check", "--all", "--skip-rotten"]);
    assert!(
        err.contains("note: dirty units not computed: the unit graph has a cycle: alpha U-001 → alpha U-002 → alpha U-001"),
        "{err}"
    );
}

// ------------------------------------------------------------ re-checks

/// A lane checkout of the repository, detached at `HEAD`.
struct Lane {
    path: PathBuf,
}

impl Lane {
    fn new(p: &Project) -> Lane {
        let path = p.tmp.path().join("lane");
        git(
            &p.root,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                path.to_str().unwrap(),
                "HEAD",
            ],
        );
        Lane {
            path: path.canonicalize().unwrap(),
        }
    }

    fn git(&self) -> Git {
        Git::new(&self.path)
    }

    fn sdd(&self, rel: &str) -> Instance {
        Instance::load(
            model::resolve_sdd(
                &self.path,
                Some(&self.path.join(rel).to_string_lossy()),
                &self.path,
            )
            .unwrap(),
        )
        .unwrap()
    }

    /// Re-check `units` of `sdd` at `commit`, apply what it earned and
    /// commit it; returns the report and the new commit.
    fn recheck(&self, commit: &str, sdd: &str, units: &[&str]) -> (recheck::Report, String) {
        let units: Vec<String> = units.iter().map(|u| u.to_string()).collect();
        let report = recheck::run(&self.git(), commit, Path::new(sdd), &units, &ShRunner).unwrap();
        // what the commands left behind is not recorded
        self.git().reset_hard("HEAD").unwrap();
        self.git().clean_all().unwrap();
        let findings = recheck::apply(&self.sdd(sdd), &self.path, &report.edits()).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
        git(&self.path, &["add", "-A"]);
        git(
            &self.path,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("sdd: re-check {sdd}"),
            ],
        );
        (report, git(&self.path, &["rev-parse", "HEAD"]))
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(vitrify_cli::gate::slots_root(&self.path));
    }
}

#[test]
fn a_passing_recheck_is_recorded_and_leaves_nothing_dirty() {
    let p = Project::new();
    p.write("a/one.txt", "one, again\n");
    let changed = p.commit("foundation");
    let set = dirty_at(&p.root);
    assert_eq!(names(&set), ["alpha U-001", "alpha U-002", "beta U-001"]);
    let lane = Lane::new(&p);

    // alpha's dirty units, then its suite
    let (report, after_alpha) = lane.recheck(&changed, "specs/alpha", &["U-001", "U-002"]);
    assert!(!report.interrupted && report.broken.is_none(), "{report:?}");
    assert!(report.units.iter().all(|u| u.passed), "{report:?}");
    let suite = report.suite.as_ref().unwrap();
    assert_eq!(suite.command, "test -f a/one.txt");
    assert!(suite.passed);
    let edits = report.edits();
    assert_eq!(edits.verified, ["U-001", "U-002"]);
    assert!(edits.suite_pass);
    let short = &changed[..7];
    assert_eq!(edits.commit, short);

    // beta's, on top: a descendant whose changes since are protocol state
    let (report, after_beta) = lane.recheck(&after_alpha, "specs/beta", &["U-001"]);
    assert!(report.units.iter().all(|u| u.passed), "{report:?}");

    // the journal holds `verified` events by `recheck` at the checked
    // commits, and an audit event with the suite's pass
    let journal = std::fs::read_to_string(lane.path.join("specs/alpha/journal.ndjson")).unwrap();
    for unit in ["U-001", "U-002"] {
        assert!(
            journal.contains(&format!(
                r#""event":"verified","unit":"{unit}","actor":"recheck","sdd":"1.0.0","commit":"{short}""#
            )),
            "{journal}"
        );
    }
    assert!(
        journal.contains(&format!(
            r#""event":"audit","actor":"audit","findings":0,"suite":"pass","sdd":"1.0.0","commit":"{short}""#
        )),
        "{journal}"
    );
    // fresh stamps, naming the checked commit
    let unit = std::fs::read_to_string(lane.path.join("specs/alpha/units/U-002-two.md")).unwrap();
    assert!(
        unit.lines()
            .any(|l| l.starts_with("verified:") && l.ends_with(&format!("commit {short}"))),
        "{unit}"
    );
    let unit = std::fs::read_to_string(lane.path.join("specs/beta/units/U-001-one.md")).unwrap();
    assert!(
        unit.contains(&format!("commit {}", &after_alpha[..7])),
        "{unit}"
    );

    // replay and audit stay clean
    for sdd in ["specs/alpha", "specs/beta"] {
        let (ok, out, err) = p.vitrify_in(&lane.path, &["journal", "replay", "--sdd", sdd]);
        assert!(ok, "replay {sdd}: {out}{err}");
        let (ok, out, err) = p.vitrify_in(&lane.path, &["audit", "--sdd", sdd]);
        assert!(ok, "audit {sdd}: {out}{err}");
    }

    // kept progress: at the new head nothing is dirty, and the suites hold
    let set = dirty_at(&lane.path);
    assert_eq!(set.head, after_beta);
    assert!(set.units.is_empty(), "{:?}", set.units);
    assert!(
        set.suites.iter().all(|s| s.dirty.is_none()),
        "{:?}",
        set.suites
    );
}

#[test]
fn a_failing_row_records_nothing_for_its_unit() {
    let p = Project::new();
    p.write("a/two.txt", "broken\n");
    let changed = p.commit("break U-002");
    assert_eq!(names(&dirty_at(&p.root)), ["alpha U-002"]);
    let lane = Lane::new(&p);
    let journal_before =
        std::fs::read_to_string(lane.path.join("specs/alpha/journal.ndjson")).unwrap();
    let unit_before =
        std::fs::read_to_string(lane.path.join("specs/alpha/units/U-002-two.md")).unwrap();

    let (report, after) = lane.recheck(&changed, "specs/alpha", &["U-002", "U-009"]);
    assert_eq!(report.skipped, ["U-009"]);
    assert_eq!(report.units.len(), 1);
    assert!(!report.units[0].passed);
    assert_eq!(report.units[0].failures.len(), 1);
    assert_eq!(report.units[0].failures[0].command, "grep -q two a/two.txt");
    // the suite is judged on its own, and passed
    assert!(report.suite.as_ref().unwrap().passed);
    let edits = report.edits();
    assert!(edits.verified.is_empty());
    assert!(edits.suite_pass);

    let journal = std::fs::read_to_string(lane.path.join("specs/alpha/journal.ndjson")).unwrap();
    assert!(!journal.contains(r#""actor":"recheck""#), "{journal}");
    assert!(journal.starts_with(&journal_before));
    assert_eq!(
        std::fs::read_to_string(lane.path.join("specs/alpha/units/U-002-two.md")).unwrap(),
        unit_before
    );
    // still dirty at the new head
    let set = dirty_at(&lane.path);
    assert_eq!(set.head, after);
    assert_eq!(names(&set), ["alpha U-002"]);
}

#[test]
fn a_broken_build_runs_no_row_and_records_nothing() {
    let p = Project::new();
    let sdd = p.read("specs/alpha/sdd.md");
    p.write(
        "specs/alpha/sdd.md",
        &sdd.replace("- Build:     `true`", "- Build:     `false`"),
    );
    let head = p.commit("break the build");
    let lane = Lane::new(&p);
    let ran = std::sync::Mutex::new(Vec::<String>::new());
    let runner = |cmd: &str, cwd: &Path| {
        ran.lock().unwrap().push(cmd.to_string());
        let r = recheck::ShRunner;
        recheck::Runner::run(&r, cmd, cwd)
    };
    let units = vec!["U-001".to_string()];
    let report = recheck::run(
        &lane.git(),
        &head,
        Path::new("specs/alpha"),
        &units,
        &runner,
    )
    .unwrap();
    assert_eq!(*ran.lock().unwrap(), ["true", "false"]);
    assert_eq!(report.broken.as_ref().unwrap().command, "false");
    assert!(!report.units[0].passed);
    assert!(report.edits().is_empty());
}

#[test]
fn the_lane_is_made_pristine_first() {
    let p = Project::new();
    let lane = Lane::new(&p);
    // leftovers: an ignored file, an untracked one, a tracked edit
    std::fs::write(lane.path.join(".gitignore"), "*.log\n").unwrap();
    git(&lane.path, &["add", ".gitignore"]);
    git(&lane.path, &["commit", "-q", "-m", "ignore logs"]);
    let head = git(&lane.path, &["rev-parse", "HEAD"]);
    std::fs::write(lane.path.join("a/build.log"), "stale\n").unwrap();
    std::fs::write(lane.path.join("b/extra.txt"), "extra\n").unwrap();
    std::fs::write(lane.path.join("a/one.txt"), "edited\n").unwrap();
    let report = recheck::run(
        &lane.git(),
        &head,
        Path::new("specs/alpha"),
        &["U-001".to_string()],
        &ShRunner,
    )
    .unwrap();
    assert!(report.units[0].passed, "{report:?}");
    assert!(!lane.path.join("a/build.log").exists());
    assert!(!lane.path.join("b/extra.txt").exists());
    assert_eq!(
        std::fs::read_to_string(lane.path.join("a/one.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn a_diff_cache_carried_across_heads_finds_what_fresh_diffs_find() {
    let p = Project::new();
    let git = Git::new(&p.root);
    let mut cache = dirty::DiffCache::default();
    let both = |cache: &mut dirty::DiffCache| {
        let snap = Snapshot::load(&git, &p.root, "HEAD").unwrap();
        let fresh = snap.dirty(&git, Mode::Changes).unwrap();
        let kept = snap.dirty_with(&git, Mode::Changes, cache).unwrap();
        let (Cascade::Resolved(fresh), Cascade::Resolved(kept)) = (fresh, kept) else {
            panic!("cycles");
        };
        let suites = |s: &DirtySet| format!("{:?}", s.suites);
        assert_eq!(names(&kept), names(&fresh));
        assert_eq!(suites(&kept), suites(&fresh));
        names(&fresh)
    };
    assert!(both(&mut cache).is_empty());
    // a change, then the same bytes back: dirty, then clean again
    let one = p.read("a/one.txt");
    p.write("a/one.txt", "one, again\n");
    p.commit("foundation");
    assert_eq!(
        both(&mut cache),
        ["alpha U-001", "alpha U-002", "beta U-001"]
    );
    p.event("specs/alpha", "audit", None, "audit", &p.anchor);
    p.commit("state");
    assert_eq!(
        both(&mut cache),
        ["alpha U-001", "alpha U-002", "beta U-001"]
    );
    p.write("a/one.txt", &one);
    p.commit("foundation, as it was");
    assert!(both(&mut cache).is_empty());
    p.write("b/two.txt", "b two, again\n");
    p.commit("leaf");
    assert_eq!(both(&mut cache), ["beta U-002"]);
}
