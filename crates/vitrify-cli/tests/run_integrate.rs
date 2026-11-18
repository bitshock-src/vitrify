//! The serial integrator on sh-only inline mini projects: green path lands
//! evidence commits; superseded-token rejection; expired-lease skip; the
//! write-set check (foreign paths, own spec text, unmanaged paths, the
//! ownership rule behind every test posture, single-file regions); own-path
//! conflict → escalation signal; red gate → clean revert; lost-race redo;
//! the input check (built from the text mainline holds now); the Lint and
//! Test commands in the gate; batches (one gate for several units, red ones
//! sorted out one by one); benign mainline movement during a gate; the
//! gate board while a row runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_cli::git::Git;
use vitrify_cli::layout::Layout;
use vitrify_cli::run::integrate::{
    GateMemo, IntegrateRequest, Integrator, Outcome, benign, sdd_commands,
};
use vitrify_cli::run::mainline::Mainline;
use vitrify_cli::run::progress::GateBoard;

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

fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
}

/// Write a file under `dir`, creating parent directories.
fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const SDD: &str = "---\nsdd: mini\nschema_version: 2.0\ntitle: mini\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - src/\n---\n\n# mini\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\nunmanaged: []\n";

/// The mini SDD with explicit scope roots and `unmanaged` globs.
fn sdd_with(scope_roots: &[&str], unmanaged: &[&str]) -> String {
    let roots: String = scope_roots.iter().map(|r| format!("  - {r}\n")).collect();
    let unmanaged = if unmanaged.is_empty() {
        "unmanaged: []\n".to_string()
    } else {
        let items: String = unmanaged.iter().map(|g| format!("  - {g}\n")).collect();
        format!("unmanaged:\n{items}")
    };
    format!(
        "---\nsdd: mini\nschema_version: 2.0\ntitle: mini\nversion: 1.0.0\nstatus: Active\nscope_roots:\n{roots}---\n\n# mini\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\n{unmanaged}"
    )
}

fn unit_md(uid: &str, file: &str, group: &str) -> String {
    format!(
        "---\nunit: {uid}\ntitle: {uid}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: []\nparallel_group: {group}\nowns: [{file}]\nconsumes: []\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n---\n\n# {uid}\n\n## Requirements\n\n- **{uid}.R1** \u{2014} THE SYSTEM SHALL ship `{file}`.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f {file}` | exit 0 |\n"
    )
}

/// A unit owning `owns`, depending on `deps`, verified by `checks` (one
/// Verification row each).
fn unit_md_with(uid: &str, owns: &[&str], deps: &[&str], checks: &[&str]) -> String {
    let rows: String = checks
        .iter()
        .map(|c| format!("| R1 | `{c}` | exit 0 |\n"))
        .collect();
    format!(
        "---\nunit: {uid}\ntitle: {uid}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: [{}]\nparallel_group: \u{2014}\nowns: [{}]\nconsumes: []\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n---\n\n# {uid}\n\n## Requirements\n\n- **{uid}.R1** \u{2014} THE SYSTEM SHALL pass its checks.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n{rows}",
        deps.join(", "),
        owns.join(", ")
    )
}

/// A single-file SDD with two independent units owning src/one.txt and
/// src/two.txt.
fn single_sdd() -> String {
    let unit = |uid: &str, file: &str| {
        format!(
            "### {uid} \u{2014} {file}\n\n```yaml\nunit: {uid}\ntitle: {uid}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: []\nparallel_group: \u{2014}\nowns: [{file}]\nconsumes: []\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n```\n\n#### Requirements\n\n- **{uid}.R1** \u{2014} THE SYSTEM SHALL ship `{file}`.\n\n#### Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f {file}` | exit 0 |\n\n"
        )
    };
    format!(
        "---\nsdd: solo\nschema_version: 2.0\ntitle: solo\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - src/\n---\n\n# solo\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\nunmanaged: []\n\n## Work Units\n\n{}{}",
        unit("U-001", "src/one.txt"),
        unit("U-002", "src/two.txt")
    )
}

/// A repository and one SDD in it.
#[derive(Clone)]
struct View {
    root: PathBuf,
    home: PathBuf,
    /// Where the unit worktrees and the integration checkout live: outside
    /// the repository's working tree.
    wt_root: PathBuf,
    /// The SDD's root relative to the repository root, and its slug.
    sdd_rel: PathBuf,
    slug: String,
}

struct Rig {
    _tmp: tempfile::TempDir,
    view: View,
}

impl std::ops::Deref for Rig {
    type Target = View;
    fn deref(&self) -> &View {
        &self.view
    }
}

impl Drop for Rig {
    /// The gate ran in this process, so its temp slots are under the
    /// system temp root, keyed by the integration checkout — and kept, by
    /// design. This checkout is going: take them along.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(vitrify_cli::gate::slots_root(&self.view.mainline()));
    }
}

/// A repository seeded with `files` (repo-relative path, content), committed
/// on `main`. The SDD under test is the first file listed that carries an
/// `sdd:` slug.
fn rig_with(files: &[(&str, String)]) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let home = tmp.path().join("vhome");
    std::fs::create_dir_all(&root).unwrap();
    for (rel, content) in files {
        write(&root, rel, content);
    }
    let root = root.canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    let (sdd_rel, slug) = files
        .iter()
        .find_map(|(rel, content)| {
            let slug = content
                .lines()
                .find_map(|l| l.strip_prefix("sdd: "))?
                .to_string();
            // a bundle is named by its directory, a single-file SDD by its
            // file — whatever that file is called
            match rel.strip_suffix("/sdd.md") {
                Some(dir) => Some((PathBuf::from(dir), slug)),
                None => Some((PathBuf::from(rel), slug)),
            }
        })
        .expect("an SDD among the seeded files");
    // a home `vitrify init` set up: every command needs one
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), vitrify_core::content::CONFIG.body).unwrap();
    let wt_root = tmp.path().canonicalize().unwrap().join("wt");
    // subprocesses of this rig keep their temp files inside it
    std::fs::create_dir_all(tmp.path().join("t")).unwrap();
    Rig {
        view: View {
            root,
            home,
            wt_root,
            sdd_rel,
            slug,
        },
        _tmp: tmp,
    }
}

/// The default mini: two independent units owning src/one.txt and
/// src/two.txt.
fn rig() -> Rig {
    rig_with(&[
        ("specs/mini/sdd.md", SDD.to_string()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ])
}

impl View {
    /// The same repository, seen through another of its SDDs.
    fn sdd(&self, sdd_rel: &str, slug: &str) -> View {
        View {
            sdd_rel: PathBuf::from(sdd_rel),
            slug: slug.to_string(),
            ..self.clone()
        }
    }

    fn sdd_flag(&self) -> String {
        self.sdd_rel.to_string_lossy().to_string()
    }

    fn vitrify(&self, args: &[&str], cwd: &Path) -> (bool, String, String) {
        let out = Command::new(vitrify_bin())
            .args(args)
            .current_dir(cwd)
            .env("VITRIFY_HOME", &self.home)
            .env("TMPDIR", self.wt_root.parent().unwrap().join("t"))
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// Claim a unit and give it its worktree and branch; returns the claim
    /// sequence.
    fn claim(&self, unit: &str, lease: &str) -> i64 {
        let sdd = self.sdd_flag();
        let (ok, stdout, stderr) = self.vitrify(
            &[
                "claim",
                "--unit",
                unit,
                "--holder",
                "w1",
                "--lease-minutes",
                lease,
                "--sdd",
                &sdd,
            ],
            &self.root,
        );
        assert!(ok, "claim failed: {stderr}");
        let claim: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        let seq = claim["claim_seq"].as_i64().unwrap();
        let commit = claim["commit"].as_str().unwrap();
        let branch = self.branch(unit, seq);
        let wt = self.worktree(unit, seq);
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        git(
            &self.root,
            &[
                "worktree",
                "add",
                wt.to_str().unwrap(),
                "-b",
                &branch,
                commit,
            ],
        );
        seq
    }

    /// Commit the handoff, as the orchestrator does.
    fn handoff(&self, unit: &str, seq: i64) {
        let wt = self.worktree(unit, seq);
        git(&wt, &["add", "-A"]);
        git(
            &wt,
            &["commit", "-q", "-m", &format!("sdd: build {unit} (c{seq})")],
        );
    }

    /// Claim a unit, let `work` edit its worktree, stamp Built and commit the
    /// branch — what a worker plus the orchestrator's handoff commit do.
    fn claim_and_then(&self, unit: &str, lease: &str, work: impl FnOnce(&Path)) -> i64 {
        let seq = self.claim(unit, lease);
        let wt = self.worktree(unit, seq);
        work(&wt);
        let sdd = self.sdd_flag();
        let (ok, _, stderr) = self.vitrify(
            &[
                "trace", "record", "--unit", unit, "--stamp", "built", "--sdd", &sdd,
            ],
            &wt,
        );
        assert!(ok, "trace record failed: {stderr}");
        self.handoff(unit, seq);
        seq
    }

    /// Claim a unit and build its branch; `write_artifact` controls whether
    /// the owned file is actually produced (red-gate rehearsal skips it).
    fn claim_and_build(&self, unit: &str, file: &str, write_artifact: bool, lease: &str) -> i64 {
        self.claim_and_then(unit, lease, |wt| {
            if write_artifact {
                write(wt, file, "artifact\n");
            }
        })
    }

    fn branch(&self, unit: &str, seq: i64) -> String {
        format!("sdd/{}/{unit}/c{seq}", self.slug)
    }

    fn worktree(&self, unit: &str, seq: i64) -> PathBuf {
        self.wt_root.join(format!("{}-{unit}-c{seq}", self.slug))
    }

    /// The integration checkout.
    fn mainline(&self) -> PathBuf {
        self.wt_root.join("mainline")
    }

    fn ml(&self) -> Mainline {
        Mainline {
            primary: Git::new(&self.root),
            layout: Layout::at(&self.root, &self.wt_root),
            sdd_rel: self.sdd_rel.clone(),
            branch: "main".to_string(),
            push: false,
        }
    }

    fn integrator(&self) -> Integrator {
        Integrator::new(self.ml(), 1)
    }

    fn request(unit: &str, seq: i64) -> IntegrateRequest {
        IntegrateRequest {
            unit: unit.to_string(),
            token_seq: seq,
            builder_holder: "w1".to_string(),
            integrator_id: "integrator".to_string(),
        }
    }

    fn integrate(&self, unit: &str, seq: i64) -> Outcome {
        self.integrate_hooked(unit, seq, None)
    }

    fn integrate_hooked(&self, unit: &str, seq: i64, hook: Option<&dyn Fn()>) -> Outcome {
        self.integrator()
            .integrate(&View::request(unit, seq), &mut GateMemo::default(), hook)
            .unwrap()
    }

    /// Integrate several units as one batch, `jobs` Verification rows at a
    /// time.
    fn integrate_batch(&self, units: &[(&str, i64)], jobs: usize) -> Vec<Outcome> {
        let reqs: Vec<IntegrateRequest> = units.iter().map(|(u, s)| View::request(u, *s)).collect();
        Integrator::new(self.ml(), jobs)
            .integrate_batch(&reqs, &mut GateMemo::default(), None)
            .unwrap()
    }

    fn main_head(&self) -> String {
        git(&self.root, &["rev-parse", "refs/heads/main"])
    }
}

#[test]
fn green_path_lands_evidence_commits() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let outcome = r.integrate("U-001", seq);
    let Outcome::Verified { verify_commit, .. } = outcome else {
        panic!("expected Verified, got {outcome:?}");
    };
    assert_eq!(r.main_head(), verify_commit);

    // mainline carries merge + verify commits and the full evidence
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(log.contains("sdd: verify U-001 (c1)"));
    assert!(log.contains("sdd: integrate U-001 (c1)"));
    let unit = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(unit.contains("status: Verified"));
    assert!(unit.contains("claimed_by: —"));
    assert!(unit.contains("trace:\n"));
    let journal = git(&r.root, &["show", "main:specs/mini/journal.ndjson"]);
    let events: Vec<&str> = journal.lines().collect();
    assert!(
        events
            .iter()
            .any(|l| l.contains(r#""event":"built","unit":"U-001","actor":"w1""#))
    );
    assert!(
        events
            .iter()
            .any(|l| l.contains(r#""event":"verified","unit":"U-001","actor":"integrator""#))
    );
    assert!(
        events
            .iter()
            .any(|l| l.contains(r#""event":"audit","actor":"audit","findings":0"#))
    );
    let ledger = git(&r.root, &["show", "main:specs/mini/ledger.md"]);
    assert!(ledger.contains("| U-001 | Verified"));
    // branch and worktree pruned
    let branches = git(&r.root, &["branch", "--list", "sdd/mini/U-001/c1"]);
    assert!(branches.is_empty());
    assert!(!r.worktree("U-001", 1).exists());
    // nothing vitrify builds or gates in lives inside the working tree
    assert!(!r.root.join(".vitrify/mainline").exists());
    assert!(!r.root.join(".vitrify/worktrees").exists());
    assert!(r.mainline().join("src/one.txt").exists());
}

#[test]
fn superseded_token_rejected_even_if_green() {
    let r = rig();
    // c1 claimed with an instantly-lapsing lease, built green
    let seq1 = r.claim_and_build("U-001", "src/one.txt", true, "0");
    assert_eq!(seq1, 1);
    // reclaimed: c2 supersedes c1
    let (ok, stdout, stderr) = r.vitrify(&["claim", "--unit", "U-001", "--holder", "w2"], &r.root);
    assert!(ok, "{stderr}");
    let c2: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(c2["claim_seq"], 2);

    let outcome = r.integrate("U-001", seq1);
    let Outcome::RejectedFencing { reason } = outcome else {
        panic!("expected RejectedFencing, got {outcome:?}");
    };
    assert!(reason.contains("superseded token"), "{reason}");
    // nothing landed
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(!log.contains("integrate U-001"));
}

#[test]
fn expired_lease_with_current_token_skips() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "0"); // lapses immediately
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::SkippedLeaseExpired),
        "got {outcome:?}"
    );
}

// ------------------------------------------------------------ write-set check

#[test]
fn write_outside_owns_rejected_before_merge() {
    let r = rig();
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        write(wt, "src/two.txt", "overreach\n"); // U-002's file
    });
    let head = r.main_head();
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, detail } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["src/two.txt".to_string()]);
    assert!(
        detail.contains("src/two.txt: not owned by U-001"),
        "{detail}"
    );
    // rejected before merging: nothing merged, nothing published
    assert_eq!(r.main_head(), head);
    let ml = r.mainline();
    assert_eq!(git(&ml, &["rev-parse", "HEAD"]), head);
    assert!(git(&ml, &["status", "--porcelain"]).is_empty());
}

#[test]
fn write_outside_every_units_ownership_rejected() {
    let r = rig();
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        write(wt, "NOTES.md", "a stray file outside scope_roots\n");
    });
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, .. } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["NOTES.md".to_string()]);
}

#[test]
fn own_spec_text_edit_rejected() {
    // a unit is judged by its spec as authored: its implementer may not
    // rewrite its own Verification table (or any other spec text)
    let r = rig();
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        let unit = wt.join("specs/mini/units/U-001-one.md");
        let text = std::fs::read_to_string(&unit).unwrap();
        std::fs::write(&unit, text.replace("`test -f src/one.txt`", "`true`")).unwrap();
    });
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, detail } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["specs/mini/units/U-001-one.md".to_string()]);
    assert!(detail.contains("spec text edited"), "{detail}");
}

#[test]
fn unmanaged_paths_are_read_only() {
    // `unmanaged` exempts a path from the orphan audit, not from the
    // write-set: tool output a build regenerates belongs in .gitignore
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_with(&["src/"], &["build/"])),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        write(wt, "build/out.log", "tool output\n");
    });
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, .. } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["build/out.log".to_string()]);
}

#[test]
fn gitignored_byproducts_never_reach_the_check() {
    let r = rig_with(&[
        ("specs/mini/sdd.md", SDD.to_string()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (".gitignore", ".vitrify/\nbuild/\n".to_string()),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        write(wt, "build/out.log", "tool output\n");
    });
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
}

#[test]
fn sdd_files_are_read_only_whatever_owns_says() {
    // `*` crosses `/`, so `*.md` matches every spec file — which stay
    // read-only anyway
    let r = rig_with(&[
        ("specs/mini/sdd.md", SDD.to_string()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with(
                "U-001",
                &["src/one.txt", "*.md"],
                &[],
                &["test -f src/one.txt"],
            ),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        write(wt, "docs/notes.md", "owned through *.md\n");
        let other = wt.join("specs/mini/units/U-002-two.md");
        let text = std::fs::read_to_string(&other).unwrap();
        std::fs::write(&other, text.replace("`test -f src/two.txt`", "`true`")).unwrap();
    });
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, detail } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["specs/mini/units/U-002-two.md".to_string()]);
    assert!(detail.contains("a file of the SDD"), "{detail}");
}

// ---------------------------------------------- test postures: who may write

#[test]
fn unit_writing_its_own_tests_lands() {
    // posture: the unit writes its own tests — it owns them, and its
    // Verification table runs them
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_with(&["src/", "tests/"], &[])),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with(
                "U-001",
                &["src/one.txt", "tests/one.sh"],
                &[],
                &["sh tests/one.sh"],
            ),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "tests/one.sh", "test \"$(cat src/one.txt)\" = one\n");
        write(wt, "src/one.txt", "one\n");
    });
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    assert!(!git(&r.root, &["show", "main:tests/one.sh"]).is_empty());
}

#[test]
fn tests_owned_by_another_unit_are_read_only() {
    // postures: a test unit writes the tests first, or they were written
    // before the build and belong to a unit already Verified — either way
    // another unit owns them, and the implementer cannot change them
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_with(&["src/", "tests/"], &[])),
        (
            "specs/mini/units/U-001-tests.md",
            unit_md_with("U-001", &["tests/two.sh"], &[], &["test -f tests/two.sh"]),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md_with("U-002", &["src/two.txt"], &["U-001"], &["sh tests/two.sh"]),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "tests/two.sh", "test \"$(cat src/two.txt)\" = two\n");
    });
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );

    // U-002's implementer "fixes" the test instead of the code
    let seq = r.claim_and_then("U-002", "60", |wt| {
        write(wt, "src/two.txt", "wrong\n");
        write(wt, "tests/two.sh", "true\n");
    });
    let outcome = r.integrate("U-002", seq);
    let Outcome::RejectedWriteSet { paths, detail } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["tests/two.sh".to_string()]);
    assert!(detail.contains("not owned by U-002"), "{detail}");
}

#[test]
fn source_code_is_never_read() {
    // vitrify judges paths and exit codes, never code: a cross-unit import
    // with no consumed contract is the SDD's own boundary checker's concern
    // (run as a gate command), not the audit's
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_with(&["src/", "tests/"], &[])),
        (
            "specs/mini/units/U-001-tests.md",
            unit_md_with("U-001", &["tests/"], &[], &["test -f tests/test_two.py"]),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md_with(
                "U-002",
                &["src/two.py"],
                &["U-001"],
                &["test -f src/two.py"],
            ),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "tests/test_two.py", "from src.two import VALUE\n");
        write(wt, "tests/two_test.rs", "use crate::two::VALUE;\n");
        write(
            wt,
            "tests/two.test.ts",
            "import { VALUE } from \"../src/two\";\n",
        );
    });
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
}

#[test]
fn tests_outside_the_sdd_judge_but_cannot_be_edited() {
    // posture: tests that already exist outside scope_roots — no unit owns
    // them; Verification runs them, and nobody may change them
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_with(&["src/"], &[])),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with("U-001", &["src/one.txt"], &[], &["sh qa/one.sh"]),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md_with("U-002", &["src/two.txt"], &[], &["test -f src/two.txt"]),
        ),
        (
            "qa/one.sh",
            "test \"$(cat src/one.txt)\" = one\n".to_string(),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| write(wt, "src/one.txt", "one\n"));
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );

    let seq = r.claim_and_then("U-002", "60", |wt| {
        write(wt, "src/two.txt", "two\n");
        write(wt, "qa/one.sh", "true\n");
    });
    let outcome = r.integrate("U-002", seq);
    let Outcome::RejectedWriteSet { paths, .. } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["qa/one.sh".to_string()]);
}

#[test]
fn single_file_work_may_change_only_its_own_handoff_fields() {
    let r = rig_with(&[("specs/solo.sdd.md", single_sdd())]);
    // U-001 also rewrites U-002's Verification row inside the shared file
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "artifact\n");
        let sdd = wt.join("specs/solo.sdd.md");
        let text = std::fs::read_to_string(&sdd).unwrap();
        std::fs::write(&sdd, text.replace("`test -f src/two.txt`", "`true`")).unwrap();
    });
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedWriteSet { paths, detail } = outcome else {
        panic!("expected RejectedWriteSet, got {outcome:?}");
    };
    assert_eq!(paths, vec!["specs/solo.sdd.md".to_string()]);
    assert!(detail.contains("spec text edited"), "{detail}");

    // U-002 stamps only its own region: lands
    let seq = r.claim_and_then("U-002", "60", |wt| write(wt, "src/two.txt", "artifact\n"));
    let outcome = r.integrate("U-002", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
}

// ---------------------------------------------------------- merge and gate

#[test]
fn conflict_on_own_path_signals_ownership_violation() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    // meanwhile mainline got a different src/one.txt, written outside the
    // protocol — the branch stays inside its write-set, so the merge decides
    write(&r.root, "src/one.txt", "mainline version\n");
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "mainline one"]);

    let outcome = r.integrate("U-001", seq);
    let Outcome::OwnershipConflict { paths } = outcome else {
        panic!("expected OwnershipConflict, got {outcome:?}");
    };
    assert_eq!(paths, vec!["src/one.txt".to_string()]);
    // merge aborted; mainline worktree clean
    let status = git(&r.mainline(), &["status", "--porcelain"]);
    assert!(status.is_empty(), "mainline worktree dirty: {status}");
}

#[test]
fn red_gate_reverts_cleanly() {
    let r = rig();
    // built WITHOUT the artifact: `test -f src/one.txt` will fail
    let seq = r.claim_and_build("U-001", "src/one.txt", false, "60");
    let base = r.main_head();
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedGate { command, output: _ } = outcome else {
        panic!("expected RejectedGate, got {outcome:?}");
    };
    assert_eq!(command, "test -f src/one.txt");
    // nothing published; mainline worktree reverted to the base
    assert_eq!(r.main_head(), base);
    let ml = r.mainline();
    assert_eq!(git(&ml, &["rev-parse", "HEAD"]), base);
    assert!(git(&ml, &["status", "--porcelain"]).is_empty());
}

#[test]
fn what_the_gate_s_commands_leave_behind_is_not_published() {
    // the Build command rewrites a tracked file and drops a new one beside
    // it: the gate judges that tree, but what is published is the merge
    let sdd = SDD.replace(
        "- Build:     `true`",
        "- Build:     `sh -c 'echo built >> src/one.txt; echo x > stray.txt'`",
    );
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    assert_eq!(git(&r.root, &["show", "main:src/one.txt"]), "artifact");
    let tree = git(&r.root, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(!tree.contains("stray.txt"), "{tree}");
    // the verify commit holds the stamp, the journal and the ledger — only
    let verify = git(&r.root, &["show", "--name-only", "--format=", "main"]);
    for path in verify.lines() {
        assert!(path.starts_with("specs/mini/"), "{verify}");
    }
    // and the primary checkout, which followed, shows no change of its own
    assert!(git(&r.root, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_spec_edit_beside_the_handoff_fields_is_stale_inputs_not_a_conflict() {
    // mainline retitles the unit while it is being built: the line next to
    // the ones the handoff stamps. Git would call that a merge conflict in
    // the unit's own file; what it is, is text the unit must be rebuilt from.
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    assert!(text.contains("title: U-001\nstatus: "), "{text}");
    std::fs::write(
        &unit,
        text.replace("title: U-001\n", "title: U-001, renamed\n"),
    )
    .unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "mini CH-001: U-001 retitled"],
    );
    let base = r.main_head();

    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedStaleInputs { keys } = outcome else {
        panic!("expected RejectedStaleInputs, got {outcome:?}");
    };
    assert_eq!(keys, vec!["unit".to_string()]);
    assert_eq!(r.main_head(), base, "nothing published");
    assert!(git(&r.mainline(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn lost_race_redoes_from_sync_and_lands() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");

    // move mainline between the gate and the publish, exactly once
    let moved = std::sync::atomic::AtomicBool::new(false);
    let root = r.root.clone();
    let hook = move || {
        if !moved.swap(true, std::sync::atomic::Ordering::SeqCst) {
            std::fs::write(root.join("interloper.txt"), "x\n").unwrap();
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-q", "-m", "interloper"]);
        }
    };
    let outcome = r.integrate_hooked("U-001", seq, Some(&hook));
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(
        log.contains("interloper"),
        "redo must build on the new head"
    );
    assert!(log.contains("sdd: verify U-001 (c1)"));
}

#[test]
fn sdd_commands_parser() {
    let cmds = sdd_commands(SDD);
    assert_eq!(cmds.bootstrap.as_deref(), Some("true"));
    assert_eq!(cmds.build.as_deref(), Some("true"));
    assert_eq!(cmds.test.as_deref(), Some("true"));
    assert_eq!(cmds.lint.as_deref(), Some("true"));
    // `none` (no backticked command) parses as absent
    let cmds = sdd_commands(&SDD.replace("- Test:      `true`", "- Test:      none"));
    assert_eq!(cmds.test, None);
}

// ------------------------------------------------- the gate beyond the tables

/// A mini SDD with its own Lint and Test commands.
fn sdd_gated(lint: &str, test: &str) -> String {
    SDD.replace("- Lint:      `true`", &format!("- Lint:      `{lint}`"))
        .replace("- Test:      `true`", &format!("- Test:      `{test}`"))
}

/// The mini SDD whose Bootstrap leaves one line per gate run in
/// `<worktrees>/gate-runs` (the integration checkout's parent).
fn sdd_counting() -> String {
    SDD.replace(
        "- Bootstrap: `true`",
        "- Bootstrap: `sh -c 'echo run >> ../gate-runs'`",
    )
}

fn gate_runs(r: &View) -> usize {
    std::fs::read_to_string(r.wt_root.join("gate-runs"))
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

#[test]
fn lint_command_gates_integration() {
    // the unit's own rows are green; the SDD's Lint command is what objects
    let r = rig_with(&[
        (
            "specs/mini/sdd.md",
            sdd_gated("sh -c '! grep -rqs FIXME src'", "true"),
        ),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "FIXME later\n")
    });
    let base = r.main_head();
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedGate { command, .. } = outcome else {
        panic!("expected RejectedGate, got {outcome:?}");
    };
    assert_eq!(command, "sh -c '! grep -rqs FIXME src'");
    assert_eq!(r.main_head(), base, "nothing published");
    // the rejected attempt's branch is still there: its work is kept
    assert!(!git(&r.root, &["branch", "--list", "sdd/mini/U-001/c1"]).is_empty());
}

/// Two units and a suite: `src/*.check` scripts, all run by the Test
/// command. U-001's check also judges U-002's artifact — which U-002's own
/// Verification row (the file exists) does not.
fn suite_rig() -> Rig {
    let test = "sh -c 'for f in src/*.check; do [ -e \"$f\" ] && { sh \"$f\" >/dev/null 2>&1; s=$?; [ $s -eq 0 ] && continue; exit 1; }; done; exit 0'";
    assert!(!test.contains('|'), "a `|` would end the table cell");
    rig_with(&[
        ("specs/mini/sdd.md", sdd_gated("true", test)),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with(
                "U-001",
                &["src/one.txt", "src/one.check"],
                &[],
                &["test -f src/one.txt"],
            ),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md_with("U-002", &["src/two.txt"], &[], &["test -f src/two.txt"]),
        ),
    ])
}

const ONE_CHECK: &str = "test \"$(cat src/one.txt)\" = one && { test ! -e src/two.txt || test \"$(cat src/two.txt)\" = two; }\n";

#[test]
fn test_command_gates_as_a_regression_check() {
    assert!(
        ONE_CHECK.contains("||"),
        "the check is a file, not a table cell"
    );
    let r = suite_rig();
    let seq = r.claim_and_then("U-001", "60", |wt| {
        write(wt, "src/one.txt", "one\n");
        write(wt, "src/one.check", ONE_CHECK);
    });
    assert!(matches!(
        r.integrate("U-001", seq),
        Outcome::Verified { .. }
    ));

    // U-002 ships a file its own row accepts and the suite does not: every
    // Verification row of every unit is green, the whole suite is red
    let seq = r.claim_and_then("U-002", "60", |wt| write(wt, "src/two.txt", "wrong\n"));
    let base = r.main_head();
    let outcome = r.integrate("U-002", seq);
    let Outcome::RejectedGate { command, output } = outcome else {
        panic!("expected RejectedGate, got {outcome:?}");
    };
    assert!(
        command.starts_with("sh -c 'for f in src/*.check"),
        "{command}"
    );
    assert!(
        output.contains("passed before this merge and fails after it"),
        "{output}"
    );
    assert_eq!(r.main_head(), base, "nothing published");
    assert!(git(&r.mainline(), &["status", "--porcelain"]).is_empty());

    // released and built again, correctly: lands
    let (ok, _, stderr) = r.vitrify(&["release", "--unit", "U-002", "--holder", "w1"], &r.root);
    assert!(ok, "{stderr}");
    let seq = r.claim_and_then("U-002", "60", |wt| write(wt, "src/two.txt", "two\n"));
    assert_eq!(seq, 2);
    assert!(matches!(
        r.integrate("U-002", seq),
        Outcome::Verified { .. }
    ));
}

#[test]
fn test_command_already_failing_does_not_judge_a_merge() {
    // tests that await their code, a runner that fails on an empty suite, an
    // earlier breakage: while the suite is red at the head, no merge is
    // judged by it
    let r = rig_with(&[
        (
            "specs/mini/sdd.md",
            sdd_gated("true", "test -f src/two.txt"),
        ),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    // the checkout was rewound to measure the suite before the merge, and the
    // merge replayed for publication: mainline has the work
    assert!(!git(&r.root, &["show", "main:src/one.txt"]).is_empty());
    // the unit that turns it green lands too
    let seq = r.claim_and_build("U-002", "src/two.txt", true, "60");
    assert!(matches!(
        r.integrate("U-002", seq),
        Outcome::Verified { .. }
    ));
}

// ------------------------------------------------------------- input check

#[test]
fn work_built_from_text_that_changed_is_not_merged() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    // the unit's requirement is rewritten on mainline while it is being built
    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("SHALL ship", "SHALL ship, signed,")).unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "mini CH-001: U-001.R1 rewritten"],
    );
    let base = r.main_head();

    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedStaleInputs { keys } = outcome else {
        panic!("expected RejectedStaleInputs, got {outcome:?}");
    };
    assert_eq!(keys, vec!["unit".to_string()]);
    assert_eq!(r.main_head(), base, "nothing published");
    assert!(git(&r.mainline(), &["status", "--porcelain"]).is_empty());
    // never stamped Verified against text it was not built from
    let unit = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(unit.contains("status: Claimed"), "{unit}");
}

#[test]
fn a_global_region_changing_stales_work_in_flight_too() {
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let sdd = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd).unwrap();
    std::fs::write(&sdd, text.replace("\nT.\n", "\nT, revised.\n")).unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "mini CH-002: pins revised"],
    );
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedStaleInputs { keys } = outcome else {
        panic!("expected RejectedStaleInputs, got {outcome:?}");
    };
    assert_eq!(keys, vec!["global".to_string()]);
}

#[test]
fn work_without_a_built_stamp_is_rejected() {
    // the stamp is the handoff: without it nothing says what the unit was
    // built from
    let r = rig();
    let seq = r.claim("U-001", "60");
    write(&r.worktree("U-001", seq), "src/one.txt", "artifact\n");
    r.handoff("U-001", seq);
    let outcome = r.integrate("U-001", seq);
    let Outcome::RejectedNotBuilt { reason } = outcome else {
        panic!("expected RejectedNotBuilt, got {outcome:?}");
    };
    assert!(reason.contains("does not stamp U-001 Built"), "{reason}");
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(!log.contains("integrate U-001"));
}

// ------------------------------------------------------------------ batches

#[test]
fn a_batch_is_gated_once_and_published_as_one_commit() {
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_counting()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ]);
    let s1 = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let s2 = r.claim_and_build("U-002", "src/two.txt", true, "60");
    let outcomes = r.integrate_batch(&[("U-001", s1), ("U-002", s2)], 1);
    let commits: Vec<&String> = outcomes
        .iter()
        .map(|o| match o {
            Outcome::Verified { verify_commit, .. } => verify_commit,
            other => panic!("expected Verified, got {other:?}"),
        })
        .collect();
    assert_eq!(
        commits[0], commits[1],
        "one integration commit for the batch"
    );
    assert_eq!(r.main_head(), *commits[0]);
    assert_eq!(gate_runs(&r), 1, "the gate ran once for both units");

    // history keeps each unit's boundary; state is consistent
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(log.contains("sdd: verify U-001 (c1), U-002 (c1)"), "{log}");
    assert!(log.contains("sdd: integrate U-001 (c1)"));
    assert!(log.contains("sdd: integrate U-002 (c1)"));
    let (ok, stdout, stderr) = r.vitrify(&["journal", "replay"], &r.root);
    assert!(ok, "{stdout}{stderr}");
    let (ok, stdout, stderr) = r.vitrify(&["audit"], &r.root);
    assert!(ok, "{stdout}{stderr}");
    for unit in ["U-001", "U-002"] {
        assert!(!r.worktree(unit, 1).exists());
    }
    assert!(git(&r.root, &["branch", "--list", "sdd/*"]).is_empty());
}

#[test]
fn a_red_batch_is_sorted_out_one_unit_at_a_time() {
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_counting()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ]);
    let s1 = r.claim_and_build("U-001", "src/one.txt", true, "60");
    // U-002 is built without its artifact: its own row is red
    let s2 = r.claim_and_build("U-002", "src/two.txt", false, "60");
    let outcomes = r.integrate_batch(&[("U-001", s1), ("U-002", s2)], 1);
    assert!(
        matches!(outcomes[0], Outcome::Verified { .. }),
        "the good unit lands: {:?}",
        outcomes[0]
    );
    let Outcome::RejectedGate { command, .. } = &outcomes[1] else {
        panic!("expected RejectedGate, got {:?}", outcomes[1]);
    };
    assert_eq!(command, "test -f src/two.txt");
    // one run for the batch, then one each
    assert_eq!(gate_runs(&r), 3);
    let u1 = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(u1.contains("status: Verified"));
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(
        u2.contains("status: Claimed"),
        "U-002 is still its holder's"
    );
}

#[test]
fn rows_run_side_by_side_and_each_distinct_command_once() {
    // both units list the same row (as units list the lint command); with
    // gate_jobs > 1 the rows also overlap in time. A row that fails only
    // because another was running is run again alone before it counts.
    let shared = "sh -c 'echo run >> ../shared-row'";
    let exclusive = |tag: &str| {
        format!(
            "sh -c 'if mkdir ../row-lock 2>/dev/null; then sleep 0.4; rmdir ../row-lock; else exit 1; fi; : {tag}'"
        )
    };
    let (ex1, ex2) = (exclusive("one"), exclusive("two"));
    let r = rig_with(&[
        ("specs/mini/sdd.md", SDD.to_string()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with("U-001", &["src/one.txt"], &[], &[shared, &ex1]),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md_with("U-002", &["src/two.txt"], &[], &[shared, &ex2]),
        ),
    ]);
    let s1 = r.claim_and_then("U-001", "60", |wt| write(wt, "src/one.txt", "one\n"));
    let s2 = r.claim_and_then("U-002", "60", |wt| write(wt, "src/two.txt", "two\n"));
    let outcomes = r.integrate_batch(&[("U-001", s1), ("U-002", s2)], 4);
    for o in &outcomes {
        assert!(matches!(o, Outcome::Verified { .. }), "got {o:?}");
    }
    let shared_runs = std::fs::read_to_string(r.wt_root.join("shared-row")).unwrap();
    assert_eq!(
        shared_runs.lines().count(),
        1,
        "one run for a row two units list"
    );
}

#[test]
fn gate_commands_never_share_a_tmpdir_while_they_run() {
    // each row records its TMPDIR and stays alive long enough to overlap
    // with the other
    let record = |tag: &str| {
        format!("sh -c 'test -d \"$TMPDIR\" && echo \"$TMPDIR\" >> ../tmpdirs-{tag}; sleep 0.5'")
    };
    let (a, b) = (record("one"), record("two"));
    let sdd = SDD.replace(
        "- Build:     `true`",
        "- Build:     `sh -c 'echo \"$TMPDIR\" >> ../tmpdirs-build'`",
    );
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with("U-001", &["src/one.txt"], &[], &[&a, &b]),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| write(wt, "src/one.txt", "one\n"));
    // two rows at once: two temp directories
    let outcomes = r.integrate_batch(&[("U-001", seq)], 2);
    assert!(
        matches!(outcomes[0], Outcome::Verified { .. }),
        "got {:?}",
        outcomes[0]
    );
    let dir_of = |tag: &str| {
        std::fs::read_to_string(r.wt_root.join(format!("tmpdirs-{tag}")))
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    let (one, two) = (dir_of("one"), dir_of("two"));
    assert_ne!(
        one, two,
        "rows running side by side must not share a temp root"
    );
    for d in [&one, &two] {
        assert!(d.contains("/vfy-"), "{d}");
        // a slot, not a throwaway: it is still there for the next command
        // (and for any daemon the command left behind)
        assert!(Path::new(d).is_dir(), "{d}");
    }
    // commands that run one after another take the same slot again
    assert_eq!(
        dir_of("build"),
        one.rsplit_once('/').unwrap().0.to_string() + "/c0"
    );
}

// ------------------------------------------------- where the gate runs

#[test]
fn gate_runs_where_no_ignore_file_of_the_repository_covers_it() {
    // Stand-in for a checker that honours ignore files above its root (and
    // so passes without reading anything when one of them says `*`): the
    // Lint command fails if any directory above the checkout carries such a
    // file. `.vitrify/` does — which is why nothing is gated there.
    let lint = "sh -c 'd=$(pwd -P); while [ \"$d\" != / ]; do d=$(dirname \"$d\"); if [ -f \"$d/.gitignore\" ] && grep -qx \"[*]\" \"$d/.gitignore\"; then echo \"$d/.gitignore covers this checkout\" >&2; exit 1; fi; done'";
    assert!(!lint.contains('|'));
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_gated(lint, "true")),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    assert_eq!(
        std::fs::read_to_string(r.root.join(".vitrify/.gitignore")).unwrap(),
        "*\n",
        "the scratch directory is still ignored wholesale"
    );
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    // the same command inside `.vitrify/` is the defect: it would be covered
    let inside = r.root.join(".vitrify/probe");
    std::fs::create_dir_all(&inside).unwrap();
    let covered = Command::new("sh")
        .args([
            "-c",
            lint.trim_start_matches("sh -c '").trim_end_matches('\''),
        ])
        .current_dir(&inside)
        .output()
        .unwrap();
    assert!(
        !covered.status.success(),
        "the stand-in must see `.vitrify/.gitignore`"
    );
}

// --------------------------------------------------- several SDDs, one repo

fn named_sdd(slug: &str, scope: &str) -> String {
    SDD.replace("sdd: mini", &format!("sdd: {slug}"))
        .replace("  - src/", &format!("  - {scope}/"))
}

#[test]
fn two_sdds_with_the_same_unit_ids_never_share_a_name() {
    let r = rig_with(&[
        ("specs/alpha/sdd.md", named_sdd("alpha", "a")),
        (
            "specs/alpha/units/U-001-one.md",
            unit_md("U-001", "a/one.txt", "a"),
        ),
        ("specs/beta/sdd.md", named_sdd("beta", "b")),
        (
            "specs/beta/units/U-001-one.md",
            unit_md("U-001", "b/one.txt", "a"),
        ),
    ]);
    let alpha = r.sdd("specs/alpha", "alpha");
    let beta = r.sdd("specs/beta", "beta");

    // both U-001s are claimed (c1) and built at the same time
    let sa = alpha.claim_and_build("U-001", "a/one.txt", true, "60");
    let sb = beta.claim_and_build("U-001", "b/one.txt", true, "60");
    assert_eq!((sa, sb), (1, 1));
    assert_ne!(alpha.worktree("U-001", 1), beta.worktree("U-001", 1));
    let branches = git(
        &r.root,
        &["branch", "--list", "sdd/*", "--format=%(refname:short)"],
    );
    assert!(branches.contains("sdd/alpha/U-001/c1"), "{branches}");
    assert!(branches.contains("sdd/beta/U-001/c1"), "{branches}");

    // integrating alpha's leaves beta's work exactly where it was
    let beta_tip = git(&r.root, &["rev-parse", "sdd/beta/U-001/c1"]);
    let outcome = alpha.integrate("U-001", sa);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    assert!(beta.worktree("U-001", 1).join("b/one.txt").exists());
    assert_eq!(git(&r.root, &["rev-parse", "sdd/beta/U-001/c1"]), beta_tip);
    let beta_unit = git(&r.root, &["show", "main:specs/beta/units/U-001-one.md"]);
    assert!(beta_unit.contains("status: Claimed"), "{beta_unit}");

    // and beta's lands through the same integration checkout
    let outcome = beta.integrate("U-001", sb);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    for path in ["a/one.txt", "b/one.txt"] {
        assert!(!git(&r.root, &["show", &format!("main:{path}")]).is_empty());
    }
}

// ------------------------------------- mainline moving while the gate runs

#[test]
fn claims_landing_during_a_gate_do_not_void_it() {
    // a claim (or a renewal, a release, a recorded question) moves nothing a
    // gate command reads: the verdict stands and the merge is replayed on
    // the new head — the gate does not run again
    let r = rig_with(&[
        ("specs/mini/sdd.md", sdd_counting()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md("U-001", "src/one.txt", "a"),
        ),
        (
            "specs/mini/units/U-002-two.md",
            unit_md("U-002", "src/two.txt", "a"),
        ),
    ]);
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let claimed = std::sync::atomic::AtomicBool::new(false);
    let view = r.view.clone();
    let hook = move || {
        if !claimed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let (ok, _, stderr) =
                view.vitrify(&["claim", "--unit", "U-002", "--holder", "w2"], &view.root);
            assert!(ok, "{stderr}");
        }
    };
    let outcome = r.integrate_hooked("U-001", seq, Some(&hook));
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    assert_eq!(gate_runs(&r), 1, "the gate did not run again");
    let log = git(&r.root, &["log", "--format=%s", "main"]);
    assert!(log.contains("sdd: claim U-002 (c1) by w2"), "{log}");
    assert!(log.contains("sdd: verify U-001 (c1)"), "{log}");
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(u2.contains("claimed_by: w2"), "the claim survived: {u2}");
    let (ok, stdout, stderr) = r.vitrify(&["journal", "replay"], &r.root);
    assert!(ok, "{stdout}{stderr}");
}

#[test]
fn only_protocol_state_is_benign() {
    let r = rig();
    let ml = r.ml();
    let commit = |msg: &str| {
        git(&r.root, &["add", "-A"]);
        git(&r.root, &["commit", "-q", "-m", msg]);
        r.main_head()
    };
    let a = r.main_head();
    // a claim: front-matter lease fields, journal, ledger
    let (ok, _, stderr) = r.vitrify(&["claim", "--unit", "U-001", "--holder", "w1"], &r.root);
    assert!(ok, "{stderr}");
    let b = r.main_head();
    assert!(benign(&ml, &a, &b).unwrap(), "a claim is protocol state");
    // a recorded question
    write(&r.root, "specs/mini/decisions.md", "- **Q-001** — open\n");
    let c = commit("sdd: record Q-001");
    assert!(benign(&ml, &b, &c).unwrap());
    // a unit's spec text
    let unit = r.root.join("specs/mini/units/U-002-two.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("SHALL ship", "SHALL always ship")).unwrap();
    let d = commit("spec text");
    assert!(
        !benign(&ml, &c, &d).unwrap(),
        "spec text is not protocol state"
    );
    // code
    write(&r.root, "src/extra.txt", "x\n");
    let e = commit("code");
    assert!(!benign(&ml, &d, &e).unwrap());
    // the core file (its Commands decide what a gate runs)
    let sdd = r.root.join("specs/mini/sdd.md");
    let text = std::fs::read_to_string(&sdd).unwrap();
    std::fs::write(
        &sdd,
        text.replace("- Build:     `true`", "- Build:     `:`"),
    )
    .unwrap();
    let f = commit("commands");
    assert!(!benign(&ml, &e, &f).unwrap());
    // and never across histories that are not one line
    assert!(!benign(&ml, &f, &a).unwrap());
}

// ------------------------------------- putting mainline right mechanically

#[test]
fn a_spec_change_marks_what_it_touched_stale_with_the_cause() {
    // U-001 is Verified; then its text changes on mainline with no staleness
    // marks applied. The audit names the mismatch; healing applies the mark
    // — one mainline commit: status, a `stale` event carrying the cause,
    // the ledger — and the unit is claimable again.
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    assert!(matches!(
        r.integrate("U-001", seq),
        Outcome::Verified { .. }
    ));
    let ml = r.ml();

    // nothing to put right: nothing is published
    let head = r.main_head();
    let healed = ml.heal(true, true).unwrap();
    assert!(healed.commit.is_none() && healed.stale.is_empty() && healed.leases.is_empty());
    assert_eq!(r.main_head(), head);

    let unit = r.root.join("specs/mini/units/U-001-one.md");
    let text = std::fs::read_to_string(&unit).unwrap();
    std::fs::write(&unit, text.replace("SHALL ship", "SHALL ship, signed,")).unwrap();
    git(
        &r.root,
        &["commit", "-q", "-am", "mini CH-001: U-001.R1 rewritten"],
    );
    assert!(vitrify_cli::run::integrate::spec_moved(&ml, &head, &r.main_head()).unwrap());

    let healed = ml.heal(true, true).unwrap();
    assert_eq!(
        healed.stale,
        vec![(
            "U-001".to_string(),
            "recorded trace differs from current content: unit".to_string()
        )]
    );
    assert!(healed.other.is_empty(), "{:?}", healed.other);
    assert_eq!(healed.commit.as_deref(), Some(r.main_head().as_str()));
    let on_main = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(on_main.contains("status: Stale"), "{on_main}");
    let journal = git(&r.root, &["show", "main:specs/mini/journal.ndjson"]);
    assert!(
        journal.lines().last().unwrap().contains(
            r#""event":"stale","unit":"U-001","actor":"audit","cause":"recorded trace differs from current content: unit""#
        ),
        "{journal}"
    );
    assert!(git(&r.root, &["show", "main:specs/mini/ledger.md"]).contains("| U-001 | Stale"));
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", "main"]),
        "sdd: audit --fix-leases --apply-stale"
    );
    // the state is consistent again, and a second pass finds nothing to do
    let (ok, stdout, stderr) = r.vitrify(&["journal", "replay"], &r.root);
    assert!(ok, "{stdout}{stderr}");
    assert!(ml.heal(true, true).unwrap().commit.is_none());
    // claims and their like are not spec changes
    let before = r.main_head();
    let (ok, _, stderr) = r.vitrify(&["claim", "--unit", "U-002", "--holder", "w9"], &r.root);
    assert!(ok, "{stderr}");
    assert!(!vitrify_cli::run::integrate::spec_moved(&ml, &before, &r.main_head()).unwrap());
}

#[test]
fn a_lapsed_lease_elsewhere_is_not_held_against_an_integration() {
    // U-002's holder vanished and its lease ran out. That is mainline's
    // state, not U-001's doing: U-001 integrates, and the lapsed lease is
    // released on the way.
    let r = rig();
    let (ok, _, stderr) = r.vitrify(
        &[
            "claim",
            "--unit",
            "U-002",
            "--holder",
            "ghost",
            "--lease-minutes",
            "0",
        ],
        &r.root,
    );
    assert!(ok, "{stderr}");
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let outcome = r.integrate("U-001", seq);
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(u2.contains("status: Ready"), "{u2}");
    assert!(u2.contains("claim_seq: 1"), "the token is kept: {u2}");
}

#[test]
fn a_lease_lapsing_during_the_gate_is_not_held_against_the_batch() {
    // U-001 is gated green; before it is published, somebody else's claim on
    // U-002 lands and lapses. The final audit sees a lapsed lease — on a
    // unit this integration has nothing to do with. It is released, and
    // U-001 is published: not rejected for it.
    let r = rig();
    let seq = r.claim_and_build("U-001", "src/one.txt", true, "60");
    let fired = std::sync::atomic::AtomicBool::new(false);
    let view = r.view.clone();
    let hook = move || {
        if !fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let (ok, _, stderr) = view.vitrify(
                &[
                    "claim",
                    "--unit",
                    "U-002",
                    "--holder",
                    "ghost",
                    "--lease-minutes",
                    "0",
                ],
                &view.root,
            );
            assert!(ok, "{stderr}");
        }
    };
    let outcome = r.integrate_hooked("U-001", seq, Some(&hook));
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    let u1 = git(&r.root, &["show", "main:specs/mini/units/U-001-one.md"]);
    assert!(u1.contains("status: Verified"), "{u1}");
    let u2 = git(&r.root, &["show", "main:specs/mini/units/U-002-two.md"]);
    assert!(u2.contains("status: Ready"), "{u2}");
    assert!(u2.contains("claim_seq: 1"), "the token is kept: {u2}");
    let (ok, stdout, stderr) = r.vitrify(&["journal", "replay"], &r.root);
    assert!(ok, "{stdout}{stderr}");
}

#[test]
fn a_single_file_sdd_is_told_from_its_protocol_state_whatever_its_name() {
    // not `*.sdd.md`: the file `--sdd` names is the SDD
    let r = rig_with(&[("specs/solo.md", single_sdd())]);
    let ml = r.ml();
    let a = r.main_head();
    // a claim rewrites that one file: lease fields, journal, ledger
    r.claim("U-001", "60");
    let b = r.main_head();
    assert_ne!(a, b);
    assert!(benign(&ml, &a, &b).unwrap(), "a claim is protocol state");
    assert!(!vitrify_cli::run::integrate::spec_moved(&ml, &a, &b).unwrap());
    // its text is not
    let sdd = r.root.join("specs/solo.md");
    let text = std::fs::read_to_string(&sdd).unwrap();
    std::fs::write(
        &sdd,
        text.replace(
            "SHALL ship `src/two.txt`",
            "SHALL always ship `src/two.txt`",
        ),
    )
    .unwrap();
    git(&r.root, &["commit", "-q", "-am", "solo CH-001: U-002.R1"]);
    let c = r.main_head();
    assert!(!benign(&ml, &b, &c).unwrap());
    assert!(vitrify_cli::run::integrate::spec_moved(&ml, &b, &c).unwrap());
}

#[test]
fn the_gate_board_shows_a_row_while_it_runs() {
    // a row that passes only once the hold is gone: the board must show it
    // running meanwhile, then go back to idle
    let tmp = tempfile::tempdir().unwrap();
    let hold = tmp.path().join("hold");
    std::fs::write(&hold, "").unwrap();
    let row = format!(
        "test -f src/one.txt && while [ -e {} ]; do sleep 0.05; done",
        hold.display()
    );
    let r = rig_with(&[
        ("specs/mini/sdd.md", SDD.to_string()),
        (
            "specs/mini/units/U-001-one.md",
            unit_md_with("U-001", &["src/one.txt"], &[], &[&row]),
        ),
    ]);
    let seq = r.claim_and_then("U-001", "60", |wt| write(wt, "src/one.txt", "one\n"));
    let board_path = tmp.path().join("gate.json");
    let read_board = || -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(&board_path).ok()?).ok()
    };
    let outcome = std::thread::scope(|s| {
        let integrating = s.spawn(|| {
            r.integrator()
                .with_board(GateBoard::to(board_path.clone()))
                .integrate(&View::request("U-001", seq), &mut GateMemo::default(), None)
                .unwrap()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let shown = read_board().is_some_and(|v| {
                v["step"] == "rows"
                    && v["batch"] == serde_json::json!(["U-001"])
                    && v["running"]
                        .as_array()
                        .is_some_and(|rs| rs.iter().any(|x| x["command"] == row.as_str()))
            });
            if shown {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline && !integrating.is_finished(),
                "the board never showed the row running: {:?}",
                read_board()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::remove_file(&hold).unwrap();
        integrating.join().unwrap()
    });
    assert!(
        matches!(outcome, Outcome::Verified { .. }),
        "got {outcome:?}"
    );
    let v = read_board().unwrap();
    assert_eq!(v["step"], "idle");
    assert_eq!(v["batch"], serde_json::json!([]));
}
