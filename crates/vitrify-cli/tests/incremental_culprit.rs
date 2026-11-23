//! Culprit finding against temp repositories with a real git history: a
//! seeded culprit among three upstream commits is named — also when it
//! reached the unit through a path outside its declared inputs; a test edit
//! in another SDD is not a likely culprit, and a generated test's owner is
//! named as the unit it changed; a flaky row
//! is concluded a flake; the probe checkout is left pristine; a probe whose
//! Build fails counts as a failure of the row; the base check of the no new
//! failures rule tells a rotten row from one the batch broke.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_cli::gate::Ran;
use vitrify_cli::git::Git;
use vitrify_cli::incremental::blame::{self, Judged, PristineCheckout, ShellRunner};
use vitrify_cli::incremental::culprit::{self, Finding, Probe, Rotten};
use vitrify_core::cascade::{Closures, Ownership, SddOwnership};
use vitrify_core::facf;
use vitrify_core::graph::Node;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// `sh -c` in `dir`, as a test runner: no temp slots to clean up after.
fn sh(cmd: &str, dir: &Path) -> Ran {
    let out = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(dir)
        .output()
        .unwrap();
    Ran {
        code: out.status.code().unwrap_or(-1),
        output: String::from_utf8_lossy(&out.stdout).to_string(),
        interrupted: false,
    }
}

/// Two SDDs: `core` (U-001 owns `src/core/a.txt`, U-002 owns
/// `src/core/b.txt`) and `api` (U-001 owns `src/api/x.txt` and depends on
/// core U-001). `api` declares no outside path an input.
fn ownership() -> Ownership {
    Ownership::new(vec![
        SddOwnership {
            slug: "core".into(),
            spec_paths: vec!["specs/core".into()],
            scope_roots: vec!["src/core/".into()],
            unmanaged: vec![],
            tests: Vec::new(),
            units: vec![
                ("U-001".into(), vec!["src/core/a.txt".into()]),
                ("U-002".into(), vec!["src/core/b.txt".into()]),
            ],
            shared_inputs: None,
        },
        SddOwnership {
            slug: "api".into(),
            spec_paths: vec!["specs/api".into()],
            scope_roots: vec!["src/api/".into()],
            unmanaged: vec![],
            tests: Vec::new(),
            units: vec![("U-001".into(), vec!["src/api/x.txt".into()])],
            shared_inputs: Some(vec![]),
        },
    ])
}

/// [`ownership`], with core listing `src/core/t/` under `tests:` and
/// core U-001 owning the generated test `src/core/t/a_test.txt`; without
/// `tests`, the same file is plainly U-001's.
fn ownership_with_test(listed: bool) -> Ownership {
    let mut core = SddOwnership {
        slug: "core".into(),
        spec_paths: vec!["specs/core".into()],
        scope_roots: vec!["src/core/".into()],
        unmanaged: vec![],
        tests: Vec::new(),
        units: vec![
            (
                "U-001".into(),
                vec!["src/core/a.txt".into(), "src/core/t/a_test.txt".into()],
            ),
            ("U-002".into(), vec!["src/core/b.txt".into()]),
        ],
        shared_inputs: None,
    };
    if listed {
        core.tests = vec!["src/core/t/".into()];
    }
    let api = SddOwnership {
        slug: "api".into(),
        spec_paths: vec!["specs/api".into()],
        scope_roots: vec!["src/api/".into()],
        unmanaged: vec![],
        tests: Vec::new(),
        units: vec![("U-001".into(), vec!["src/api/x.txt".into()])],
        shared_inputs: Some(vec![]),
    };
    Ownership::new(vec![core, api])
}

fn api() -> Node {
    Node::new("api", "U-001")
}

fn closures() -> Closures {
    [(api(), BTreeSet::from([Node::new("core", "U-001")]))].into()
}

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    probe: PathBuf,
    /// A directory outside every checkout, for counters.
    outside: PathBuf,
}

/// A repository with `a.txt` and `b.txt` both `good`, `.gitignore` ignoring
/// `out/`; returns it with the seed commit (the anchor).
fn repo() -> (Repo, String) {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let root = base.join("repo");
    write(&root, "src/core/a.txt", "good\n");
    write(&root, "src/core/b.txt", "good\n");
    write(&root, "src/api/x.txt", "x\n");
    write(&root, "specs/core/sdd.md", "core\n");
    write(&root, ".gitignore", "out/\n");
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    // the seed commit is the same in every repository: its sha is known
    // ([`SEED`])
    let seeded = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed"])
        .envs([
            ("GIT_AUTHOR_NAME", "t"),
            ("GIT_AUTHOR_EMAIL", "t@test"),
            ("GIT_AUTHOR_DATE", "1700000000 +0000"),
            ("GIT_COMMITTER_NAME", "t"),
            ("GIT_COMMITTER_EMAIL", "t@test"),
            ("GIT_COMMITTER_DATE", "1700000000 +0000"),
        ])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(seeded.success());
    let anchor = git(&root, &["rev-parse", "HEAD"]);
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    (
        Repo {
            probe: base.join("wt").join("probe-api"),
            root,
            outside,
            _tmp: tmp,
        },
        anchor,
    )
}

impl Repo {
    /// Commit `files` directly on main.
    fn commit(&self, files: &[(&str, &str)], msg: &str) -> String {
        for (rel, content) in files {
            write(&self.root, rel, content);
        }
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", msg]);
        git(&self.root, &["rev-parse", "HEAD"])
    }

    /// Integrate `files` the way the integrator does: a side branch with two
    /// commits (a worker's partial state, then its finish), merged `--no-ff`
    /// under `sdd: integrate <unit> (c1)`, then an integration commit that
    /// touches the SDD's spec. Returns the merge commit.
    fn integrate(&self, unit: &str, files: &[(&str, &str)]) -> String {
        let branch = format!("sdd/core/{unit}/c1");
        git(&self.root, &["checkout", "-q", "-b", &branch]);
        write(&self.root, files[0].0, "half-written\n");
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", "wip"]);
        self.commit(files, &format!("sdd: build {unit} (c1)"));
        git(&self.root, &["checkout", "-q", "main"]);
        git(
            &self.root,
            &[
                "merge",
                "--no-ff",
                "-q",
                "-m",
                &format!("sdd: integrate {unit} (c1)"),
                &branch,
            ],
        );
        let merge = git(&self.root, &["rev-parse", "HEAD"]);
        self.commit(
            &[("specs/core/sdd.md", &format!("core, {unit} verified\n"))],
            &format!("sdd: verify {unit} (c1)"),
        );
        merge
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"])
    }

    fn short(&self, sha: &str) -> String {
        git(&self.root, &["rev-parse", "--short", sha])
    }

    fn find(
        &self,
        anchor: &str,
        row: &str,
        setup: &[String],
        runner: &dyn blame::Runner,
    ) -> culprit::Report {
        self.find_with(&ownership(), anchor, row, setup, runner)
    }

    /// [`Repo::find`] under the ownership `own`.
    fn find_with(
        &self,
        own: &Ownership,
        anchor: &str,
        row: &str,
        setup: &[String],
        runner: &dyn blame::Runner,
    ) -> culprit::Report {
        let primary = Git::new(&self.root);
        let prepare = PristineCheckout::new(primary.clone(), &self.probe);
        let cl = closures();
        let node = api();
        let base = self.head();
        culprit::find(
            &Rotten {
                node: &node,
                row_number: 2,
                row,
                anchor,
                base: &base,
            },
            &Probe {
                git: &primary,
                ownership: own,
                closures: &cl,
                setup,
                prepare: &prepare,
                runner,
                config: facf::Config::default(),
            },
        )
        .unwrap()
    }

    /// Whether the probe checkout is at `commit` with nothing untracked or
    /// ignored in it.
    fn probe_is_pristine_at(&self, commit: &str) -> bool {
        git(&self.probe, &["rev-parse", "HEAD"]) == commit
            && git(
                &self.probe,
                &[
                    "status",
                    "--porcelain",
                    "--ignored",
                    "--untracked-files=all",
                ],
            )
            .is_empty()
    }
}

/// Every row leaves an ignored build output and an untracked file behind.
const LITTER: &str = "mkdir -p out && echo built > out/cache && echo x > junk.txt";

#[test]
fn a_seeded_culprit_among_three_upstream_commits_is_named() {
    let (r, anchor) = repo();
    // three upstream integrations: the second breaks a.txt, which the unit
    // depends on through core U-001
    r.integrate("U-002", &[("src/core/b.txt", "good\nmore\n")]);
    let culprit = r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    r.commit(&[("docs/notes.md", "notes\n")], "docs: notes");
    let row = format!("{LITTER} && grep -qx good src/core/a.txt");
    let setup = vec!["true".to_string(), LITTER.to_string()];
    let report = r.find(&anchor, &row, &setup, &ShellRunner);
    let _ = std::fs::remove_dir_all(vitrify_cli::gate::slots_root(&r.probe));

    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
    assert_eq!(c.short, r.short(&culprit));
    assert_eq!(c.subject, "sdd: integrate U-001 (c1)");
    assert_eq!(c.upstream, vec![Node::new("core", "U-001")]);
    // first-parent history: the merges, the integration commits and the
    // docs commit, never the side branches' own commits
    assert_eq!(report.candidates, 5);
    assert_eq!(report.touched, 1);
    assert_eq!(
        report.line(),
        format!(
            "api U-001 row 2 broke at {} (core U-001 integrated) — culprit found",
            r.short(&culprit)
        )
    );
    assert!(r.probe_is_pristine_at(&r.head()), "probe left dirty");
}

#[test]
fn a_test_edit_in_another_sdd_is_not_a_likely_culprit() {
    let (r, anchor) = repo();
    // three upstream commits: one changes only core U-001's generated test,
    // the next breaks its code, the last is docs
    r.commit(
        &[("src/core/t/a_test.txt", "asserts more\n")],
        "test: core U-001 asserts more",
    );
    let culprit = r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    r.commit(&[("docs/notes.md", "notes\n")], "docs: notes");
    let row = "grep -qx good src/core/a.txt";
    let report = r.find_with(&ownership_with_test(true), &anchor, row, &[], &sh);
    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
    assert_eq!(c.upstream, vec![Node::new("core", "U-001")]);
    assert_eq!(report.candidates, 4);
    // the test edit does not reach api U-001: only the code change counts
    assert_eq!(report.touched, 1);
    // unlisted, the same file is plain U-001 code, and both commits count
    let report = r.find_with(&ownership_with_test(false), &anchor, row, &[], &sh);
    assert_eq!(report.touched, 2);
    assert!(r.probe_is_pristine_at(&r.head()));

    // a test edit that does break the row is named, its owner the unit it
    // changed
    let (r, anchor) = repo();
    let culprit = r.integrate("U-001", &[("src/core/t/a_test.txt", "bad\n")]);
    r.commit(&[("docs/notes.md", "notes\n")], "docs: notes");
    let row = "! grep -qx bad src/core/t/a_test.txt";
    let report = r.find_with(&ownership_with_test(true), &anchor, row, &[], &sh);
    assert_eq!(report.touched, 0);
    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
    assert_eq!(c.upstream, vec![Node::new("core", "U-001")]);
}

#[test]
fn a_culprit_outside_the_declared_inputs_is_still_named() {
    let (r, anchor) = repo();
    // the unit's row reads b.txt, which it never declared: core U-002 is not
    // in its closure. A touched commit changes a.txt harmlessly.
    r.integrate("U-001", &[("src/core/a.txt", "good\n\n")]);
    let culprit = r.integrate("U-002", &[("src/core/b.txt", "bad\n")]);
    r.commit(&[("docs/notes.md", "notes\n")], "docs: notes");
    let row = "grep -qx good src/core/b.txt";
    let report = r.find(&anchor, row, &[], &sh);
    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
    assert_eq!(c.upstream, vec![Node::new("core", "U-002")]);
    assert_eq!(report.touched, 1);
    assert!(r.probe_is_pristine_at(&r.head()));
}

#[test]
fn a_flaky_row_is_concluded_a_flake() {
    let (r, anchor) = repo();
    r.integrate("U-002", &[("src/core/b.txt", "good\nmore\n")]);
    r.integrate("U-001", &[("src/core/a.txt", "good\n\n")]);
    r.commit(&[("docs/notes.md", "notes\n")], "docs: notes");
    // nothing broke it: the row fails every third run, wherever it runs,
    // counted in a file outside every checkout
    let counter = r.outside.join("runs");
    let row = format!(
        "n=$(cat '{c}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{c}'; {LITTER}; [ $((n % 3)) -ne 0 ]",
        c = counter.display()
    );
    let report = r.find(&anchor, &row, &[], &sh);
    assert!(
        matches!(report.finding, Finding::Flake { .. }),
        "expected a flake, got {report:?}"
    );
    assert!(report.line().contains("is flaky"), "{}", report.line());
    assert!(report.line().contains(&row));
    assert!(r.probe_is_pristine_at(&r.head()));
}

#[test]
fn a_probe_whose_build_fails_counts_as_a_failure_of_the_row() {
    let (r, anchor) = repo();
    let culprit = r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    // the build fails wherever a.txt is bad: the culprit is still named
    let setup = vec!["grep -qx good src/core/a.txt".to_string()];
    let report = r.find(&anchor, "true", &setup, &sh);
    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
    assert!(!report.notes.is_empty());
    assert!(report.notes[0].contains("counted as a failure of the row"));
}

#[test]
fn nothing_between_anchor_and_base_is_inconclusive_without_probing() {
    let (r, anchor) = repo();
    let report = r.find(&anchor, "false", &[], &sh);
    assert_eq!(report.candidates, 0);
    assert_eq!(report.probes, 0);
    assert!(matches!(
        report.finding,
        Finding::Inconclusive { best: None, .. }
    ));
    assert!(!r.probe.exists());
}

#[test]
fn the_base_check_tells_a_rotten_row_from_one_the_batch_broke() {
    let (r, _) = repo();
    r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    let rotten = "grep -qx good src/core/a.txt".to_string();
    let fine = "grep -qx good src/core/b.txt".to_string();
    let rows: BTreeMap<Node, Vec<String>> = [
        (api(), vec![rotten.clone()]),
        (Node::new("api", "U-002"), vec![fine.clone()]),
    ]
    .into();
    let prepare = PristineCheckout::new(Git::new(&r.root), &r.probe);
    // a half-made checkout, as a killed run leaves one, is made again
    std::fs::create_dir_all(&r.probe).ok();
    let Judged::Done { verdict, on_base } = blame::judge(
        &[rotten.clone(), fine.clone()],
        &rows,
        &r.head(),
        &[LITTER.to_string()],
        &prepare,
        &sh,
    )
    .unwrap() else {
        panic!("interrupted");
    };
    assert_eq!(verdict.broken, vec![fine]);
    assert_eq!(verdict.rotten_rows, vec![rotten]);
    assert_eq!(
        verdict.rotten.keys().cloned().collect::<Vec<_>>(),
        vec![api()]
    );
    assert_eq!(on_base.len(), 2);
    // the next use starts pristine again
    write(&r.probe, "out/stale", "left over\n");
    let head = r.head();
    blame::Prepare::prepare(&prepare, &head).unwrap();
    assert!(r.probe_is_pristine_at(&head));
}

/// The seed commit every [`repo`] starts from.
const SEED: &str = "deb97593983c1f699c500e023a2715e641fb81c7";

/// A blob whose sha starts `deb97` too, found by search: `deb977ea71de…`.
const SEED_TWIN: &str = "collide 2747829\n";

#[test]
fn an_anchor_prefix_a_blob_shares_still_names_its_commit() {
    let (r, anchor) = repo();
    assert_eq!(anchor, SEED, "the seed commit's sha moved");
    let twin = r.outside.join("twin");
    std::fs::write(&twin, SEED_TWIN).unwrap();
    let blob = git(&r.root, &["hash-object", "-w", twin.to_str().unwrap()]);
    assert!(blob.starts_with("deb97") && blob != anchor);
    // the recorded anchor, abbreviated: ambiguous to a plain rev-parse
    let short = &anchor[..5];
    let plain = Command::new("git")
        .args(["rev-parse", short])
        .current_dir(&r.root)
        .output()
        .unwrap();
    assert!(!plain.status.success(), "{short} is ambiguous");
    let culprit = r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    let report = r.find(short, "grep -qx good src/core/a.txt", &[], &sh);
    let Finding::Culprit(c) = &report.finding else {
        panic!("expected a culprit, got {report:?}");
    };
    assert_eq!(c.sha, culprit);
}

#[test]
fn an_anchor_that_names_no_commit_is_inconclusive_with_a_note() {
    let (r, _) = repo();
    r.integrate("U-001", &[("src/core/a.txt", "bad\n")]);
    // a tree's sha, as no recorded anchor should be
    let tree = git(&r.root, &["rev-parse", "HEAD^{tree}"]);
    let report = r.find(&tree, "false", &[], &sh);
    assert!(matches!(
        report.finding,
        Finding::Inconclusive { best: None, .. }
    ));
    assert_eq!((report.candidates, report.probes), (0, 0));
    assert_eq!(
        report.notes,
        vec![format!("its recorded anchor {tree} names no commit here")]
    );
    assert!(
        report.line().contains("culprit not searched"),
        "{}",
        report.line()
    );
    assert!(!r.probe.exists(), "nothing was probed");
}

#[test]
fn a_deleted_checkout_is_made_again() {
    let (r, _) = repo();
    let prepare = PristineCheckout::new(Git::new(&r.root), &r.probe);
    let head = r.head();
    blame::Prepare::prepare(&prepare, &head).unwrap();
    assert!(r.probe_is_pristine_at(&head));
    // deleted behind git's back: git still lists the worktree
    std::fs::remove_dir_all(&r.probe).unwrap();
    assert!(git(&r.root, &["worktree", "list", "--porcelain"]).contains("prunable"));
    blame::Prepare::prepare(&prepare, &head).unwrap();
    assert!(r.probe_is_pristine_at(&head));
}
