//! Claim/renew/release transactions, race rehearsals
//! (local CAS and bare-origin push arbitration), lapsed-lease reclaim,
//! refusal codes, tx-worktree cleanup.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let t = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

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

/// A git repo on `main` with the calc example committed and local identity.
fn calc_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    copy_dir(&repo_root().join("fixtures/example"), &root);
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    (tmp, root)
}

fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
}

fn claim_cmd(dir: &Path, op: &str, unit: &str, holder: &str) -> Command {
    let mut c = Command::new(vitrify_bin());
    c.args([op, "--unit", unit, "--holder", holder])
        .current_dir(dir)
        .env("VITRIFY_HOME", home())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn no_tx_worktrees(root: &Path) {
    if let Ok(rd) = std::fs::read_dir(root.join(".vitrify")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            assert!(!name.starts_with("tx-"), "stale tx worktree: {name}");
        }
    }
}

#[test]
fn claim_success_publishes_one_mainline_commit() {
    let (_tmp, root) = calc_repo();
    let out = claim_cmd(&root, "claim", "U-002", "w1").output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["unit"], "U-002");
    assert_eq!(v["op"], "claim");
    assert_eq!(v["holder"], "w1");
    assert_eq!(v["claim_seq"], 1);
    assert_eq!(v["mainline"], "main");
    assert!(v["claim_expires"].as_str().unwrap().ends_with('Z'));

    // one commit, with the claim message, holding all three edits
    let subject = git(&root, &["log", "-1", "--format=%s"]);
    assert_eq!(subject, "sdd: claim U-002 (c1) by w1");
    // the primary checkout was fast-forwarded (clean, on main)
    let unit = std::fs::read_to_string(root.join("specs/calc/units/U-002-errors.md")).unwrap();
    assert!(unit.contains("status: Claimed"));
    assert!(unit.contains("claimed_by: w1"));
    assert!(unit.contains("claim_seq: 1"));
    let journal = std::fs::read_to_string(root.join("specs/calc/journal.ndjson")).unwrap();
    let last = journal.lines().last().unwrap();
    assert!(last.contains(r#""event":"claim","unit":"U-002","actor":"w1","claim_seq":1"#));
    let ledger = std::fs::read_to_string(root.join("specs/calc/ledger.md")).unwrap();
    assert!(ledger.contains("| U-002 | Claimed"));
    no_tx_worktrees(&root);
}

#[test]
fn sequential_double_claim_refused_4() {
    let (_tmp, root) = calc_repo();
    assert!(
        claim_cmd(&root, "claim", "U-002", "w1")
            .output()
            .unwrap()
            .status
            .success()
    );
    let out = claim_cmd(&root, "claim", "U-002", "w2").output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("U-002 not claimable") && stderr.contains("live lease held by w1"),
        "{stderr}"
    );
    no_tx_worktrees(&root);
}

#[test]
fn local_cas_race_yields_exactly_one_winner() {
    let (_tmp, root) = calc_repo();
    let spawn =
        |holder: &str| -> Child { claim_cmd(&root, "claim", "U-002", holder).spawn().unwrap() };
    let c1 = spawn("wA");
    let c2 = spawn("wB");
    let o1 = c1.wait_with_output().unwrap();
    let o2 = c2.wait_with_output().unwrap();
    let codes = [o1.status.code().unwrap(), o2.status.code().unwrap()];
    let winners = codes.iter().filter(|&&c| c == 0).count();
    assert_eq!(
        winners,
        1,
        "codes {codes:?}; stderr1={} stderr2={}",
        String::from_utf8_lossy(&o1.stderr),
        String::from_utf8_lossy(&o2.stderr)
    );
    let loser_code = *codes.iter().find(|&&c| c != 0).unwrap();
    assert!([4, 5].contains(&loser_code), "loser exit {loser_code}");

    // published state shows the winner's holder and a single seq increment
    let winner = if o1.status.success() { "wA" } else { "wB" };
    let unit = git(&root, &["show", "main:specs/calc/units/U-002-errors.md"]);
    assert!(unit.contains(&format!("claimed_by: {winner}")));
    assert!(unit.contains("claim_seq: 1"));
    let journal = git(&root, &["show", "main:specs/calc/journal.ndjson"]);
    assert_eq!(
        journal
            .lines()
            .filter(|l| l.contains(r#""event":"claim","unit":"U-002""#))
            .count(),
        1
    );
    no_tx_worktrees(&root);
}

#[test]
fn bare_origin_race_yields_exactly_one_winner() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);

    let (_seed_tmp, seed) = calc_repo();
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "-q", "origin", "main"]);

    let w1 = tmp.path().join("w1");
    let w2 = tmp.path().join("w2");
    for w in [&w1, &w2] {
        git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), w.to_str().unwrap()],
        );
        git(w, &["config", "user.email", "t@test"]);
        git(w, &["config", "user.name", "t"]);
    }

    let c1 = claim_cmd(&w1, "claim", "U-002", "wA").spawn().unwrap();
    let c2 = claim_cmd(&w2, "claim", "U-002", "wB").spawn().unwrap();
    let o1 = c1.wait_with_output().unwrap();
    let o2 = c2.wait_with_output().unwrap();
    let codes = [o1.status.code().unwrap(), o2.status.code().unwrap()];
    assert_eq!(
        codes.iter().filter(|&&c| c == 0).count(),
        1,
        "codes {codes:?}; stderr1={} stderr2={}",
        String::from_utf8_lossy(&o1.stderr),
        String::from_utf8_lossy(&o2.stderr)
    );
    assert!([4, 5].contains(codes.iter().find(|&&c| c != 0).unwrap()));

    let winner = if o1.status.success() { "wA" } else { "wB" };
    let unit = git(&origin, &["show", "main:specs/calc/units/U-002-errors.md"]);
    assert!(unit.contains(&format!("claimed_by: {winner}")));
    assert!(unit.contains("claim_seq: 1"));
}

#[test]
fn lapsed_lease_reclaim_increments_past_old_holder() {
    let (_tmp, root) = calc_repo();
    // seed a lapsed claim (seq 2) directly and commit it
    let upath = root.join("specs/calc/units/U-002-errors.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    let text = text
        .replace("status: Ready", "status: Claimed")
        .replace("claimed_by: —", "claimed_by: ghost")
        .replace("claim_expires: —", "claim_expires: 2020-01-01T00:00:00Z")
        .replace("claim_seq: 0", "claim_seq: 2");
    std::fs::write(&upath, text).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed lapsed claim"]);

    let out = claim_cmd(&root, "claim", "U-002", "w2").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["claim_seq"], 3, "seq increments past the old holder");
    let unit = std::fs::read_to_string(&upath).unwrap();
    assert!(unit.contains("claimed_by: w2"));
    assert!(unit.contains("claim_seq: 3"));
}

#[test]
fn renew_and_release_refusal_codes() {
    let (_tmp, root) = calc_repo();
    assert!(
        claim_cmd(&root, "claim", "U-002", "w1")
            .output()
            .unwrap()
            .status
            .success()
    );

    // renew by the holder works and extends the lease
    let out = claim_cmd(&root, "renew", "U-002", "w1").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["op"], "renew");
    assert_eq!(v["claim_seq"], 1, "renew keeps the seq");

    // renew/release by a non-holder → 4
    for op in ["renew", "release"] {
        let out = claim_cmd(&root, op, "U-002", "w2").output().unwrap();
        assert_eq!(out.status.code(), Some(4), "{op} by non-holder");
        assert!(
            String::from_utf8(out.stderr)
                .unwrap()
                .contains("is not Claimed by w2")
        );
    }

    // release by the holder → Ready, seq kept
    let out = claim_cmd(&root, "release", "U-002", "w1").output().unwrap();
    assert!(out.status.success());
    let unit = std::fs::read_to_string(root.join("specs/calc/units/U-002-errors.md")).unwrap();
    assert!(unit.contains("status: Ready"));
    assert!(unit.contains("claimed_by: —"));
    assert!(unit.contains("claim_seq: 1"));
    no_tx_worktrees(&root);
}

#[test]
fn renew_after_expiry_refused_6() {
    let (_tmp, root) = calc_repo();
    let upath = root.join("specs/calc/units/U-002-errors.md");
    let text = std::fs::read_to_string(&upath).unwrap();
    let text = text
        .replace("status: Ready", "status: Claimed")
        .replace("claimed_by: —", "claimed_by: w1")
        .replace("claim_expires: —", "claim_expires: 2020-01-01T00:00:00Z")
        .replace("claim_seq: 0", "claim_seq: 1");
    std::fs::write(&upath, text).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed expired lease"]);

    let out = claim_cmd(&root, "renew", "U-002", "w1").output().unwrap();
    assert_eq!(out.status.code(), Some(6));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("already expired — re-claim instead of renewing")
    );
    no_tx_worktrees(&root);
}

#[test]
fn claim_without_sdd_at_head_refused_4() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("README.md"), "x\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    let out = claim_cmd(&root, "claim", "U-001", "w1").output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("no SDD found at the mainline head")
    );
}

/// Two SDDs in one repository, each with a U-001.
fn two_sdd_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    for (slug, scope) in [("alpha", "a"), ("beta", "b")] {
        let dir = root.join("specs").join(slug);
        std::fs::create_dir_all(dir.join("units")).unwrap();
        std::fs::write(
            dir.join("sdd.md"),
            format!(
                "---\nsdd: {slug}\nschema_version: 2.0\ntitle: {slug}\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - {scope}/\n---\n\n# {slug}\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("units/U-001-one.md"),
            format!(
                "---\nunit: U-001\ntitle: one\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: []\nparallel_group: —\nowns: [{scope}/one.txt]\nconsumes: []\nbuilt: —\nverified: —\ntrace: —\n---\n\n# U-001\n"
            ),
        )
        .unwrap();
    }
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    (tmp, root)
}

#[test]
fn sdd_flag_names_the_sdd_from_any_checkout_of_the_repository() {
    // `--sdd` is a path from the root of the checkout the verb runs in.
    // A worker runs its verbs in its own worktree — inside the working tree
    // or far from it — and must reach the same SDD the primary checkout does.
    let (tmp, root) = two_sdd_repo();
    let claim = |dir: &Path, op: &str, sdd: &str| {
        let mut c = claim_cmd(dir, op, "U-001", "w1");
        c.args(["--sdd", sdd]);
        c.output().unwrap()
    };

    // without --sdd the repository is ambiguous: refused, candidates named
    let out = claim_cmd(&root, "claim", "U-001", "w1").output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("multiple SDDs found"), "{stderr}");
    assert!(stderr.contains("\n  --sdd specs/alpha\n"), "{stderr}");

    let out = claim(&root, "claim", "specs/beta");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let commit = v["commit"].as_str().unwrap().to_string();
    // beta's U-001 is claimed; alpha's is untouched
    assert!(
        git(&root, &["show", "main:specs/beta/units/U-001-one.md"]).contains("status: Claimed")
    );
    assert!(git(&root, &["show", "main:specs/alpha/units/U-001-one.md"]).contains("status: Ready"));

    // a worktree outside the working tree, and one inside it
    let outside = tmp
        .path()
        .canonicalize()
        .unwrap()
        .join("elsewhere/beta-U-001-c1");
    let inside = root.join(".vitrify/worktrees/beta-U-001-c1");
    for (wt, branch) in [(&outside, "work/outside"), (&inside, "work/inside")] {
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().unwrap(),
                "-b",
                branch,
                &commit,
            ],
        );
        let out = claim(wt, "renew", "specs/beta");
        assert!(
            out.status.success(),
            "renew from {}: {}",
            wt.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        // from a subdirectory, relative to where the verb runs
        let out = claim(&wt.join("specs"), "renew", "beta");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = claim(&outside, "release", "specs/beta");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(git(&root, &["show", "main:specs/beta/units/U-001-one.md"]).contains("status: Ready"));
    assert!(git(&root, &["show", "main:specs/alpha/units/U-001-one.md"]).contains("claim_seq: 0"));
}

#[test]
fn a_verb_run_in_a_linked_worktree_edits_only_the_transaction_s_copy() {
    let (_tmp, root) = two_sdd_repo();
    let far_tmp = tempfile::tempdir().unwrap();
    let far = far_tmp.path().canonicalize().unwrap();
    let wt = far.join("wt");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "work/far",
        ],
    );
    let claim = |dir: &Path, op: &str, sdd: &str| {
        let mut c = claim_cmd(dir, op, "U-001", "w1");
        c.args(["--sdd", sdd]);
        c.output().unwrap()
    };
    let unit = "specs/beta/units/U-001-one.md";

    // `--sdd` relative to the linked worktree: the claim lands on mainline…
    let out = claim(&wt, "claim", "specs/beta");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(git(&root, &["show", &format!("main:{unit}")]).contains("status: Claimed"));
    // …and the worktree's own copy is what it was: nothing is edited in place
    assert!(git(&wt, &["status", "--porcelain"]).is_empty());
    assert!(
        std::fs::read_to_string(wt.join(unit))
            .unwrap()
            .contains("status: Ready")
    );

    // an absolute path into that worktree names the same SDD
    let out = claim(&wt, "renew", wt.join("specs/beta").to_str().unwrap());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(git(&wt, &["status", "--porcelain"]).is_empty());

    // a copy of the SDD that lies in no checkout of this repository is
    // refused — and left exactly as it is
    let stray = far.join("stray/specs/beta");
    copy_dir(&root.join("specs/beta"), &stray);
    let before = std::fs::read_to_string(stray.join("units/U-001-one.md")).unwrap();
    let out = claim(&root, "renew", stray.to_str().unwrap());
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("is not inside this checkout"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(stray.join("units/U-001-one.md")).unwrap(),
        before
    );
}

// ------------------------------------- the primary checkout follows mainline

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Run one transaction; returns its stderr.
fn tx(root: &Path, op: &str, unit: &str, holder: &str) -> String {
    let out = claim_cmd(root, op, unit, holder).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{op} failed: {stderr}");
    stderr
}

#[test]
fn the_primary_checkout_follows_with_the_user_s_changes_aboard() {
    let (_tmp, root) = calc_repo();
    // work in progress in files the claim does not touch: one edit staged,
    // one not
    std::fs::write(root.join("README.md"), "my notes\n").unwrap();
    std::fs::write(root.join("tdd/calc.md"), "my staged edit\n").unwrap();
    git(&root, &["add", "tdd/calc.md"]);

    let stderr = tx(&root, "claim", "U-002", "w1");
    assert!(
        stderr.contains("note: primary checkout fast-forwarded"),
        "{stderr}"
    );
    // what the claim published is in the checkout…
    let unit = read(&root.join("specs/calc/units/U-002-errors.md"));
    assert!(unit.contains("status: Claimed"), "{unit}");
    // …and the user's changes are exactly what they were, and nothing else
    // differs from the new head
    assert_eq!(read(&root.join("README.md")), "my notes\n");
    assert_eq!(read(&root.join("tdd/calc.md")), "my staged edit\n");
    assert_eq!(git(&root, &["diff", "--name-only"]), "README.md");
    assert_eq!(
        git(&root, &["diff", "--cached", "--name-only"]),
        "tdd/calc.md"
    );
    assert!(!root.join(".vitrify/primary-behind").exists());
}

#[test]
fn a_checkout_that_cannot_follow_is_left_alone_and_catches_up_later() {
    let (_tmp, root) = calc_repo();
    let seed = git(&root, &["rev-parse", "HEAD"]);
    // the user is editing the very file the claim rewrites
    let unit_path = root.join("specs/calc/units/U-002-errors.md");
    let original = read(&unit_path);
    let mine = format!("{original}\nmy note\n");
    std::fs::write(&unit_path, &mine).unwrap();

    // the claim is published all the same; the checkout is not touched, and
    // says how to bring it up to date
    let stderr = tx(&root, "claim", "U-002", "w1");
    let c1 = git(&root, &["rev-parse", "main"]);
    assert_ne!(c1, seed);
    assert!(
        stderr.contains("warning: the primary checkout could not follow main"),
        "{stderr}"
    );
    assert!(stderr.contains("U-002-errors.md"), "{stderr}");
    assert!(stderr.contains("commit nothing from it"), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "git read-tree -m -u {} {}",
            &seed[..12],
            &c1[..12]
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains("git update-index -q --refresh && git read-tree -m -u"),
        "{stderr}"
    );
    assert!(!stderr.contains("reset --keep"), "{stderr}");
    assert_eq!(read(&unit_path), mine);
    git(&root, &["diff", "--cached", "--quiet", &seed]); // the index is the seed's

    // a second transaction with the edit still in the way: the remedy now
    // spans both commits
    let stderr = tx(&root, "renew", "U-002", "w1");
    let c2 = git(&root, &["rev-parse", "main"]);
    assert!(
        stderr.contains(&format!(
            "git read-tree -m -u {} {}",
            &seed[..12],
            &c2[..12]
        )),
        "{stderr}"
    );
    assert_eq!(read(&unit_path), mine);
    git(&root, &["diff", "--cached", "--quiet", &seed]);

    // the edit goes away: the next transaction brings the checkout all the
    // way up
    std::fs::write(&unit_path, &original).unwrap();
    let stderr = tx(&root, "release", "U-002", "w1");
    assert!(
        stderr.contains("note: primary checkout fast-forwarded"),
        "{stderr}"
    );
    assert!(git(&root, &["status", "--porcelain"]).is_empty());
    let unit = read(&unit_path);
    assert!(unit.contains("status: Ready"), "{unit}");
    assert!(unit.contains("claim_seq: 1"), "{unit}");
    assert_eq!(
        read(&root.join("specs/calc/journal.ndjson")),
        git(&root, &["show", "main:specs/calc/journal.ndjson"]) + "\n"
    );
    assert!(!root.join(".vitrify/primary-behind").exists());
}

#[test]
fn a_checkout_brought_up_to_date_by_hand_is_followed_from_there() {
    let (_tmp, root) = calc_repo();
    let seed = git(&root, &["rev-parse", "HEAD"]);
    let unit_path = root.join("specs/calc/units/U-002-errors.md");
    let original = read(&unit_path);
    std::fs::write(&unit_path, format!("{original}\nmy note\n")).unwrap();
    let stderr = tx(&root, "claim", "U-002", "w1");
    let c1 = git(&root, &["rev-parse", "main"]);
    assert!(root.join(".vitrify/primary-behind").exists());

    // the user clears the way and runs the command the warning gave, as
    // printed…
    std::fs::write(&unit_path, &original).unwrap();
    let remedy = stderr
        .lines()
        .find_map(|l| {
            l.split_once("or now with: ")
                .map(|(_, cmd)| cmd.to_string())
        })
        .unwrap_or_else(|| panic!("no remedy in: {stderr}"));
    assert!(
        remedy.ends_with(&format!(
            "git read-tree -m -u {} {}",
            &seed[..12],
            &c1[..12]
        )),
        "{remedy}"
    );
    let ran = Command::new("sh")
        .args(["-c", &remedy])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{remedy}: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(git(&root, &["status", "--porcelain"]).is_empty());
    // …then goes on working, on something else
    std::fs::write(root.join("README.md"), "my notes\n").unwrap();

    // the next transaction moves the checkout from where it is now
    let stderr = tx(&root, "renew", "U-002", "w1");
    assert!(
        stderr.contains("note: primary checkout fast-forwarded"),
        "{stderr}"
    );
    assert_eq!(git(&root, &["diff", "--name-only"]), "README.md");
    assert!(git(&root, &["diff", "--cached", "--name-only"]).is_empty());
    assert!(!root.join(".vitrify/primary-behind").exists());
}

#[test]
fn an_untracked_file_in_the_way_is_never_overwritten() {
    // mainline gains a file where the user has one of their own, untracked
    let (_tmp, root) = calc_repo();
    let seed = git(&root, &["rev-parse", "HEAD"]);
    let side_tmp = tempfile::tempdir().unwrap();
    let side = side_tmp.path().join("side");
    git(
        &root,
        &["worktree", "add", "-q", "--detach", side.to_str().unwrap()],
    );
    std::fs::write(side.join("notes.txt"), "published\n").unwrap();
    git(&side, &["add", "-A"]);
    git(&side, &["commit", "-q", "-m", "notes"]);
    let published = git(&side, &["rev-parse", "HEAD"]);
    git(&root, &["update-ref", "refs/heads/main", &published, &seed]);
    // (the checkout is now one commit behind, as after any publish; the
    // marker says so, as vitrify's own failed follow would have)
    std::fs::create_dir_all(root.join(".vitrify")).unwrap();
    std::fs::write(root.join(".vitrify/.gitignore"), "*\n").unwrap();
    std::fs::write(
        root.join(".vitrify/primary-behind"),
        format!("{seed} {published}\n"),
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "mine\n").unwrap();

    let stderr = tx(&root, "claim", "U-002", "w1");
    assert!(
        stderr.contains("Untracked working tree file 'notes.txt' would be overwritten"),
        "{stderr}"
    );
    assert_eq!(read(&root.join("notes.txt")), "mine\n");

    // moved out of the way, the checkout catches up
    std::fs::rename(root.join("notes.txt"), root.join("notes.mine.txt")).unwrap();
    let stderr = tx(&root, "renew", "U-002", "w1");
    assert!(
        stderr.contains("note: primary checkout fast-forwarded"),
        "{stderr}"
    );
    assert_eq!(read(&root.join("notes.txt")), "published\n");
    assert_eq!(read(&root.join("notes.mine.txt")), "mine\n");
    assert_eq!(git(&root, &["status", "--porcelain"]), "?? notes.mine.txt");
}

/// A vitrify home `vitrify init` has set up — every command needs one —
/// shared by this file's tests, which only read it.
fn home() -> &'static Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .arg("init")
            .env("VITRIFY_HOME", home.path())
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "vitrify init: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        home
    })
    .path()
}
