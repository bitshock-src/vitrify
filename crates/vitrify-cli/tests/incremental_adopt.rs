//! Adoption on temp repositories: a Built attempt whose trace still matches
//! is adoptable, and once re-labelled under a new claim it lands through the
//! ordinary gate; a spec change since names the moved input; an attempt not
//! stamped Built, or already judged by a gate, goes to a worker; the
//! re-label refuses an existing branch and a claim that supersedes nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_cli::cmd::resolve_sdd_in;
use vitrify_cli::git::Git;
use vitrify_cli::incremental::adopt::{self, Refusal};
use vitrify_cli::layout::Layout;
use vitrify_cli::run::integrate::{GateMemo, IntegrateRequest, Integrator, Outcome};
use vitrify_cli::run::mainline::Mainline;
use vitrify_core::model::Instance;

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

const SDD: &str = "---\nsdd: mini\nschema_version: 2.0\ntitle: mini\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - src/\n---\n\n# mini\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\nunmanaged: []\n";

fn unit_md(uid: &str, file: &str, says: &str) -> String {
    format!(
        "---\nunit: {uid}\ntitle: {uid}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: []\nparallel_group: \u{2014}\nowns: [{file}]\nconsumes: []\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n---\n\n# {uid}\n\n## Requirements\n\n- **{uid}.R1** \u{2014} THE SYSTEM SHALL ship `{file}` {says}.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f {file}` | exit 0 |\n"
    )
}

const UNIT1: &str = "specs/mini/units/U-001-one.md";

struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    wt_root: PathBuf,
    tmpdir: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(vitrify_cli::gate::slots_root(
            &self.wt_root.join("mainline"),
        ));
    }
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let root = base.join("repo");
    let home = base.join("vhome");
    write(&root, "specs/mini/sdd.md", SDD);
    write(&root, UNIT1, &unit_md("U-001", "src/one.txt", "once"));
    write(
        &root,
        "specs/mini/units/U-002-two.md",
        &unit_md("U-002", "src/two.txt", "once"),
    );
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), vitrify_core::content::CONFIG.body).unwrap();
    let tmpdir = base.join("t");
    std::fs::create_dir_all(&tmpdir).unwrap();
    Rig {
        root,
        home,
        wt_root: base.join("wt"),
        tmpdir,
        _tmp: tmp,
    }
}

impl Rig {
    fn vitrify(&self, args: &[&str], cwd: &Path) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .args(args)
            .current_dir(cwd)
            .env("VITRIFY_HOME", &self.home)
            .env("TMPDIR", &self.tmpdir)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// Claim a unit for `holder`; returns the claim sequence and the claim
    /// commit.
    fn claim(&self, unit: &str, holder: &str) -> (i64, String) {
        let (ok, stdout, stderr) = self.vitrify(
            &[
                "claim",
                "--unit",
                unit,
                "--holder",
                holder,
                "--lease-minutes",
                "60",
                "--sdd",
                "specs/mini",
            ],
            &self.root,
        );
        assert!(ok, "claim failed: {stderr}");
        let claim: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        (
            claim["claim_seq"].as_i64().unwrap(),
            claim["commit"].as_str().unwrap().to_string(),
        )
    }

    fn release(&self, unit: &str, holder: &str) {
        let (ok, _, stderr) = self.vitrify(
            &[
                "release",
                "--unit",
                unit,
                "--holder",
                holder,
                "--sdd",
                "specs/mini",
            ],
            &self.root,
        );
        assert!(ok, "release failed: {stderr}");
    }

    fn worktree(&self, unit: &str, seq: i64) -> PathBuf {
        self.wt_root.join(format!("mini-{unit}-c{seq}"))
    }

    /// An attempt a worker worked on and a run then set aside with
    /// `outcome`: claimed, the file written, stamped Built when `stamp`,
    /// committed to its branch under the set-aside note, its claim released.
    fn attempt(&self, unit: &str, file: &str, stamp: bool, outcome: &str) -> i64 {
        let (seq, commit) = self.claim(unit, "w1");
        let wt = self.worktree(unit, seq);
        std::fs::create_dir_all(&self.wt_root).unwrap();
        let branch = format!("sdd/mini/{unit}/c{seq}");
        git(
            &self.root,
            &[
                "worktree",
                "add",
                wt.to_str().unwrap(),
                "-b",
                &branch,
                &commit,
            ],
        );
        write(&wt, file, "artifact\n");
        if stamp {
            let (ok, _, stderr) = self.vitrify(
                &[
                    "trace",
                    "record",
                    "--unit",
                    unit,
                    "--stamp",
                    "built",
                    "--sdd",
                    "specs/mini",
                ],
                &wt,
            );
            assert!(ok, "trace record failed: {stderr}");
        }
        git(&wt, &["add", "-A"]);
        git(
            &wt,
            &["commit", "-q", "-m", &format!("sdd: build {unit} (c{seq})")],
        );
        git(
            &wt,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("sdd: abandoned attempt {unit} (c{seq}) \u{2014} {outcome}"),
                "-m",
                "the run ended",
            ],
        );
        git(
            &self.root,
            &["worktree", "remove", "--force", wt.to_str().unwrap()],
        );
        self.release(unit, "w1");
        seq
    }

    /// The SDD as mainline holds it now, from a fresh checkout of it.
    fn head(&self) -> (Instance, PathBuf) {
        let dir = self.wt_root.join("head");
        if dir.exists() {
            git(&dir, &["reset", "-q", "--hard", "main"]);
        } else {
            std::fs::create_dir_all(&self.wt_root).unwrap();
            git(
                &self.root,
                &["worktree", "add", "--detach", dir.to_str().unwrap(), "main"],
            );
        }
        let inst = Instance::load(resolve_sdd_in(&dir, Path::new("specs/mini")).unwrap()).unwrap();
        (inst, dir)
    }

    fn adoptable(&self, unit: &str) -> Result<adopt::Attempt, Refusal> {
        let (inst, dir) = self.head();
        adopt::adoptable(&Git::new(&self.root), &inst, &dir, unit).unwrap()
    }

    /// Commit a change to mainline in the primary checkout.
    fn commit_on_main(&self, rel: &str, content: &str, msg: &str) {
        git(&self.root, &["checkout", "-q", "main"]);
        git(&self.root, &["reset", "-q", "--hard", "main"]);
        write(&self.root, rel, content);
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", msg]);
    }

    fn mainline(&self) -> Mainline {
        Mainline {
            primary: Git::new(&self.root),
            layout: Layout::at(&self.root, &self.wt_root),
            sdd_rel: PathBuf::from("specs/mini"),
            slug: "mini".to_string(),
            branch: "main".to_string(),
            push: false,
        }
    }
}

#[test]
fn a_built_attempt_with_a_matching_trace_is_adopted_and_lands_through_the_gate() {
    let r = rig();
    let old = r.attempt("U-001", "src/one.txt", true, "interrupted");
    let attempt = r.adoptable("U-001").expect("adoptable");
    assert_eq!(attempt.seq, old);
    assert_eq!(attempt.branch, "sdd/mini/U-001/c1");
    assert_eq!(
        attempt.commit,
        git(&r.root, &["rev-parse", &attempt.branch])
    );

    // the scheduler claims the unit again and re-labels the attempt
    let (new, claim_commit) = r.claim("U-001", "adopter");
    assert!(new > old);
    let primary = Git::new(&r.root);
    let scratch = r.worktree("U-001", new);
    let branch = adopt::relabel(&primary, "mini", &attempt, new, &claim_commit, &scratch).unwrap();
    assert_eq!(branch, format!("sdd/mini/U-001/c{new}"));
    // one commit on the new claim's commit, carrying the attempt's work
    assert_eq!(
        git(&r.root, &["rev-parse", &format!("{branch}^")]),
        claim_commit
    );
    assert_eq!(
        git(&r.root, &["log", "-1", "--format=%s", &branch]),
        format!("sdd: adopt U-001 (c{old}) as c{new}")
    );
    assert_eq!(
        git(&r.root, &["show", &format!("{branch}:src/one.txt")]),
        "artifact"
    );
    let stamped = git(&r.root, &["show", &format!("{branch}:{UNIT1}")]);
    assert!(stamped.contains("status: Built"), "{stamped}");
    assert!(stamped.contains(&format!("claim_seq: {new}")), "{stamped}");
    assert!(!scratch.exists());

    // the old token can never land; the new claim's branch goes through
    // the ordinary gate, with no worker
    let integrate = |seq: i64| {
        Integrator::new(r.mainline(), 1)
            .integrate(
                &IntegrateRequest {
                    unit: "U-001".to_string(),
                    token_seq: seq,
                    builder_holder: "adopter".to_string(),
                    integrator_id: "integrator".to_string(),
                },
                &mut GateMemo::default(),
                None,
            )
            .unwrap()
    };
    let stale = integrate(old);
    assert!(
        matches!(stale, Outcome::RejectedFencing { .. }),
        "{stale:?}"
    );
    let landed = integrate(new);
    assert!(matches!(landed, Outcome::Verified { .. }), "{landed:?}");
    let unit = git(&r.root, &["show", &format!("main:{UNIT1}")]);
    assert!(unit.contains("status: Verified"));
}

#[test]
fn relabel_refuses_an_existing_branch_and_a_claim_that_supersedes_nothing() {
    let r = rig();
    r.attempt("U-001", "src/one.txt", true, "interrupted");
    let attempt = r.adoptable("U-001").unwrap();
    let primary = Git::new(&r.root);
    let main_at = git(&r.root, &["rev-parse", "main"]);
    let scratch = r.worktree("U-001", 2);
    let err =
        adopt::relabel(&primary, "mini", &attempt, attempt.seq, &main_at, &scratch).unwrap_err();
    assert!(err.to_string().contains("must supersede"), "{err}");
    git(&r.root, &["branch", "sdd/mini/U-001/c2", "main"]);
    let err = adopt::relabel(&primary, "mini", &attempt, 2, &main_at, &scratch).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    // nothing moved, nothing made
    assert_eq!(git(&r.root, &["rev-parse", "sdd/mini/U-001/c2"]), main_at);
    assert!(!scratch.exists());
}

#[test]
fn an_attempt_that_wrote_outside_its_write_set_goes_to_a_worker() {
    let r = rig();
    r.attempt("U-001", "src/two.txt", true, "interrupted");
    let refusal = r.adoptable("U-001").unwrap_err();
    assert_eq!(
        refusal,
        Refusal::WriteSet {
            branch: "sdd/mini/U-001/c1".to_string(),
            paths: vec!["src/two.txt".to_string()]
        }
    );
}

#[test]
fn a_spec_change_since_names_the_moved_input() {
    let r = rig();
    r.attempt("U-001", "src/one.txt", true, "lease-lost");
    let current = git(&r.root, &["show", &format!("main:{UNIT1}")]);
    r.commit_on_main(
        UNIT1,
        &format!("{}\n", current.replace("once", "twice")),
        "spec: U-001 ships it twice",
    );
    let refusal = r.adoptable("U-001").unwrap_err();
    let Refusal::TraceMoved { branch, keys } = &refusal else {
        panic!("expected TraceMoved, got {refusal:?}");
    };
    assert_eq!(branch, "sdd/mini/U-001/c1");
    assert_eq!(keys, &vec!["unit".to_string()]);
    assert!(refusal.describe().contains("moved: unit"));
}

#[test]
fn an_attempt_not_stamped_built_goes_to_a_worker() {
    let r = rig();
    r.attempt("U-001", "src/one.txt", false, "interrupted");
    let refusal = r.adoptable("U-001").unwrap_err();
    assert!(
        matches!(&refusal, Refusal::NotBuilt { status, .. } if status == "Claimed"),
        "{refusal:?}"
    );
}

#[test]
fn an_attempt_the_gate_rejected_is_not_adopted_again() {
    let r = rig();
    r.attempt("U-001", "src/one.txt", true, "rejected-gate");
    let refusal = r.adoptable("U-001").unwrap_err();
    assert_eq!(
        refusal,
        Refusal::Judged {
            branch: "sdd/mini/U-001/c1".to_string(),
            outcome: "rejected-gate".to_string()
        }
    );
}

#[test]
fn no_attempt_no_adoption() {
    let r = rig();
    assert_eq!(r.adoptable("U-002").unwrap_err(), Refusal::NoBranch);
}
