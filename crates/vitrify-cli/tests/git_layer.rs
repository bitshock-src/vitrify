//! CAS semantics, bare-origin push arbitration,
//! mainline precedence, worktree lifecycle, committer fallback.

use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_cli::git::{Git, GitError, resolve_mainline, use_push};

fn sh_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_repo(dir: &Path) {
    sh_git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    sh_git(dir, &["add", "-A"]);
    sh_git(dir, &["commit", "-q", "-m", "A"]);
}

fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) -> String {
    std::fs::write(dir.join(name), content).unwrap();
    sh_git(dir, &["add", "-A"]);
    sh_git(dir, &["commit", "-q", "-m", msg]);
    sh_git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn cas_succeeds_at_expected_old_and_loses_on_moved_ref() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path());
    let git = Git::new(tmp.path());
    let a = git.rev_parse("HEAD").unwrap();

    // build a new commit on a detached worktree so the branch stays put
    let wt = tmp.path().join("wt");
    git.worktree_add(&wt, &a, true).unwrap();
    let b = commit_file(&wt, "b.txt", "b\n", "B");

    // CAS at the expected old oid wins
    git.update_ref_cas("refs/heads/main", &b, &a).unwrap();
    assert_eq!(git.rev_parse("refs/heads/main").unwrap(), b);

    // a second CAS claiming the stale old oid loses the race
    let c = commit_file(&wt, "c.txt", "c\n", "C");
    let err = git.update_ref_cas("refs/heads/main", &c, &a).unwrap_err();
    assert!(matches!(err, GitError::LostRace { .. }), "{err:?}");
    assert_eq!(git.rev_parse("refs/heads/main").unwrap(), b);
}

#[test]
fn bare_origin_push_arbitration_rejects_the_loser() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    sh_git(&origin, &["init", "-q", "--bare", "-b", "main"]);

    let seed = tmp.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    init_repo(&seed);
    sh_git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh_git(&seed, &["push", "-q", "origin", "main"]);

    let w1 = tmp.path().join("w1");
    let w2 = tmp.path().join("w2");
    for w in [&w1, &w2] {
        sh_git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), w.to_str().unwrap()],
        );
    }

    let c1 = commit_file(&w1, "one.txt", "1\n", "one");
    let c2 = commit_file(&w2, "two.txt", "2\n", "two");

    let g1 = Git::new(&w1);
    let g2 = Git::new(&w2);
    assert!(use_push(&g1, "main").unwrap());
    g1.push_cas(&c1, "main").unwrap();

    // the second push is a non-fast-forward rejection → LostRace
    let err = g2.push_cas(&c2, "main").unwrap_err();
    assert!(matches!(err, GitError::LostRace { .. }), "{err:?}");
}

#[test]
fn mainline_precedence_table() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path());
    let git = Git::new(tmp.path());

    // flag wins over everything
    assert_eq!(
        resolve_mainline(&git, Some("flagline"), "cfgline").unwrap(),
        "flagline"
    );
    // repo config wins next
    assert_eq!(resolve_mainline(&git, None, "cfgline").unwrap(), "cfgline");
    // current branch (no origin/HEAD here)
    assert_eq!(resolve_mainline(&git, None, "").unwrap(), "main");

    // origin/HEAD wins over the current branch
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    sh_git(&origin, &["init", "-q", "--bare", "-b", "trunk"]);
    sh_git(
        tmp.path(),
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh_git(tmp.path(), &["push", "-q", "origin", "main:trunk"]);
    sh_git(
        tmp.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    assert_eq!(resolve_mainline(&git, None, "").unwrap(), "trunk");

    // detached HEAD, no origin/HEAD → main/master probe
    let det = tempfile::tempdir().unwrap();
    init_repo(det.path());
    let head = sh_git(det.path(), &["rev-parse", "HEAD"]);
    sh_git(det.path(), &["checkout", "-q", "--detach", &head]);
    let gd = Git::new(det.path());
    assert_eq!(resolve_mainline(&gd, None, "").unwrap(), "main");
}

#[test]
fn worktree_lifecycle_and_primary_root() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path());
    let git = Git::new(tmp.path());
    let head = git.rev_parse("HEAD").unwrap();

    let wt = tmp.path().join(".vitrify/tx-123");
    git.worktree_add(&wt, &head, true).unwrap();
    assert!(wt.join("a.txt").exists());

    // primary_root from inside the worktree resolves to the primary checkout
    let from_wt = git.at(&wt).primary_root().unwrap();
    assert_eq!(
        from_wt.canonicalize().unwrap(),
        tmp.path().canonicalize().unwrap()
    );

    // branch worktree at a ref
    git.create_branch("sdd/U-001/c1", &head).unwrap();
    let wt2 = tmp.path().join(".vitrify/worktrees/U-001-c1");
    std::fs::create_dir_all(wt2.parent().unwrap()).unwrap();
    git.worktree_add(&wt2, "sdd/U-001/c1", false).unwrap();
    assert!(wt2.join("a.txt").exists());

    git.worktree_remove(&wt).unwrap();
    git.worktree_remove(&wt2).unwrap();
    assert!(!wt.exists());
    assert!(!wt2.exists());
    git.delete_branch("sdd/U-001/c1").unwrap();
    assert!(git.verify_ref("refs/heads/sdd/U-001/c1").unwrap().is_none());
}

#[test]
fn committer_fallback_when_user_email_unset() {
    let tmp = tempfile::tempdir().unwrap();
    // init without identity env
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(tmp.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    run(&["init", "-q", "-b", "main"]);
    std::fs::write(tmp.path().join("f.txt"), "x\n").unwrap();

    // Git::commit must fall back to vitrify <vitrify@local>
    struct EnvGuard(Vec<(String, Option<String>)>);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => unsafe { std::env::set_var(k, v) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }
    let keys = [
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "EMAIL",
    ];
    let _guard = EnvGuard(
        keys.iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect(),
    );
    for k in &keys {
        unsafe { std::env::remove_var(k) };
    }
    unsafe {
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    }

    let git = Git::new(tmp.path());
    assert!(git.user_email().unwrap().is_none());
    git.add_all().unwrap();
    git.commit("sdd: test commit").unwrap();
    let who = run(&["log", "-1", "--format=%cn <%ce>"]);
    assert_eq!(who, "vitrify <vitrify@local>");
}

#[test]
fn ancestor_and_clean_checks() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo(tmp.path());
    let git = Git::new(tmp.path());
    let a = git.rev_parse("HEAD").unwrap();
    let b = commit_file(tmp.path(), "b.txt", "b\n", "B");
    assert!(git.is_ancestor(&a, &b).unwrap());
    assert!(!git.is_ancestor(&b, &a).unwrap());
    assert!(git.clean_against("HEAD").unwrap());
    std::fs::write(tmp.path().join("a.txt"), "modified\n").unwrap();
    assert!(!git.clean_against("HEAD").unwrap());
    let _ = PathBuf::from(&b);
}

// ------------------------------------------------- where worktrees may live

use vitrify_cli::layout::Layout;

#[test]
fn a_worktree_root_inside_the_tree_is_excluded_locally_or_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    init_repo(&root);
    let git = Git::new(&root);

    // a directory in the working tree: ignored for this clone through
    // .git/info/exclude — no ignore file in the tree is written
    let layout = Layout::resolve(&root, None, "vitrify-wt").unwrap();
    layout.ensure_worktree_root(&git).unwrap();
    assert!(root.join("vitrify-wt").is_dir());
    let exclude = std::fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|l| l == "/vitrify-wt/"), "{exclude}");
    assert!(!root.join("vitrify-wt/.gitignore").exists());
    assert!(sh_git(&root, &["status", "--porcelain"]).is_empty());
    // idempotent
    layout.ensure_worktree_root(&git).unwrap();
    let again = std::fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert_eq!(again.matches("/vitrify-wt/").count(), 1);

    // a directory one of the repository's own ignore files covers: tools
    // that honour ignore files above their root would check nothing there
    std::fs::write(root.join(".gitignore"), "scratch/\n").unwrap();
    let covered = Layout::resolve(&root, None, "scratch/wt").unwrap();
    let err = covered.ensure_worktree_root(&git).unwrap_err();
    assert_eq!(err.exit_code(), 2);
    let text = err.to_string();
    assert!(
        text.contains("scratch/wt/ is ignored by .gitignore:1"),
        "{text}"
    );
    assert!(text.contains("run.worktree_dir"), "{text}");

    // outside the working tree nothing of the repository can cover it
    let outside = tmp
        .path()
        .canonicalize()
        .unwrap()
        .join("../vitrify-layout-probe");
    let far = Layout::at(&root, &outside);
    assert!(far.in_tree().is_none());
}

#[test]
fn attempt_branches_are_listed_and_never_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    init_repo(&root);
    let git = Git::new(&root);
    let head = git.rev_parse("HEAD").unwrap();
    git.create_branch("sdd/app/U-001/c1", &head).unwrap();
    git.create_branch("sdd/app/U-001/c3", &head).unwrap();
    git.create_branch("sdd/app/U-0010/c1", &head).unwrap();
    git.create_branch("sdd/other/U-001/c2", &head).unwrap();
    let mut under = git.branches_under("sdd/app/U-001/").unwrap();
    under.sort();
    assert_eq!(under, ["sdd/app/U-001/c1", "sdd/app/U-001/c3"]);
    assert!(git.branch_exists("sdd/app/U-001/c1").unwrap());
    assert!(!git.branch_exists("sdd/app/U-001/c2").unwrap());

    // a new worktree never takes over an existing branch or directory
    let wt = root.join("wt-a");
    let err = git.worktree_add_branch(&wt, "sdd/app/U-001/c1", &head);
    assert!(err.is_err(), "the branch exists: nothing may be replaced");
    assert!(!wt.exists());
    git.worktree_add_branch(&wt, "sdd/app/U-001/c2", &head)
        .unwrap();
    assert!(wt.join("a.txt").exists());
    assert!(
        git.worktree_add_branch(&wt, "sdd/app/U-001/c4", &head)
            .is_err()
    );
}
