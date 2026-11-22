//! The closed git subprocess layer. Every git
//! invocation in the binary goes through here: argv-style, never a shell,
//! never `--force` (save the one `worktree remove --force` of a unit
//! worktree). Publish failures are classified: CAS/push rejection → a typed
//! `LostRace`; anything else → an exit-1 class error with git's stderr tail.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use vitrify_core::CoreError;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("lost publish race: {stderr}")]
    LostRace { stderr: String },
    #[error("error: command failed ({code}): git {cmd}\n{stderr}")]
    Failed {
        cmd: String,
        code: i32,
        stderr: String,
    },
}

impl From<GitError> for CoreError {
    fn from(e: GitError) -> CoreError {
        match e {
            GitError::LostRace { stderr } => {
                CoreError::refusal(5, format!("error: lost publish race: {stderr}"))
            }
            other => CoreError::refusal(1, other.to_string()),
        }
    }
}

fn stderr_tail(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = text.lines().collect();
    let tail = if lines.len() > 20 {
        &lines[lines.len() - 20..]
    } else {
        &lines[..]
    };
    tail.join("\n").trim().to_string()
}

/// One fetch at a time within this process: two fetches of the same branch
/// that overlap race for the remote-tracking ref, and the loser fails.
static FETCH_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One worktree add / remove at a time within this process: they all edit
/// the repository's worktree list.
static WORKTREE_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A git handle rooted at one directory (the primary checkout or a worktree).
#[derive(Debug, Clone)]
pub struct Git {
    pub dir: PathBuf,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Git {
        Git { dir: dir.into() }
    }

    pub fn at(&self, dir: impl Into<PathBuf>) -> Git {
        Git { dir: dir.into() }
    }

    fn raw(&self, args: &[&str]) -> Result<Output, GitError> {
        Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .map_err(|e| GitError::Failed {
                cmd: args.join(" "),
                code: -1,
                stderr: e.to_string(),
            })
    }

    /// Run and require exit 0; returns trimmed stdout.
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let out = self.raw(args)?;
        if !out.status.success() {
            return Err(GitError::Failed {
                cmd: args.join(" "),
                code: out.status.code().unwrap_or(-1),
                stderr: stderr_tail(&out),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Run without checking; returns (success, stdout_trimmed, stderr_tail).
    fn run_unchecked(&self, args: &[&str]) -> Result<(bool, String, String), GitError> {
        let out = self.raw(args)?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            stderr_tail(&out),
        ))
    }

    // ------------------------------------------------------- read-only

    /// Primary checkout root via `rev-parse --git-common-dir`; bare rejected.
    pub fn primary_root(&self) -> Result<PathBuf, GitError> {
        let common = self.run(&["rev-parse", "--git-common-dir"])?;
        let mut common_path = PathBuf::from(&common);
        if !common_path.is_absolute() {
            common_path = self.dir.join(&common_path);
        }
        let common_path = common_path.canonicalize().unwrap_or(common_path);
        if common_path.file_name().is_none_or(|n| n != ".git") {
            return Err(GitError::Failed {
                cmd: "rev-parse --git-common-dir".into(),
                code: 1,
                stderr: format!(
                    "unsupported repository layout (bare?): {}",
                    common_path.display()
                ),
            });
        }
        Ok(common_path.parent().unwrap().to_path_buf())
    }

    pub fn rev_parse(&self, rev: &str) -> Result<String, GitError> {
        self.run(&["rev-parse", rev])
    }

    pub fn short_head(&self) -> Result<String, GitError> {
        self.run(&["rev-parse", "--short", "HEAD"])
    }

    /// `rev-parse -q --verify <ref>`; `None` when the ref does not exist.
    pub fn verify_ref(&self, name: &str) -> Result<Option<String>, GitError> {
        let (ok, stdout, _) = self.run_unchecked(&["rev-parse", "-q", "--verify", name])?;
        Ok(if ok && !stdout.is_empty() {
            Some(stdout)
        } else {
            None
        })
    }

    /// Current branch short name; `None` when detached.
    pub fn current_branch(&self) -> Result<Option<String>, GitError> {
        let (ok, stdout, _) = self.run_unchecked(&["symbolic-ref", "-q", "--short", "HEAD"])?;
        Ok(if ok && !stdout.is_empty() {
            Some(stdout)
        } else {
            None
        })
    }

    /// `origin/HEAD` target branch name (`main` from `origin/main`).
    pub fn origin_head_branch(&self) -> Result<Option<String>, GitError> {
        let (ok, stdout, _) =
            self.run_unchecked(&["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"])?;
        Ok(if ok && !stdout.is_empty() {
            Some(
                stdout
                    .split_once('/')
                    .map(|(_, b)| b.to_string())
                    .unwrap_or(stdout),
            )
        } else {
            None
        })
    }

    pub fn user_email(&self) -> Result<Option<String>, GitError> {
        let (ok, stdout, _) = self.run_unchecked(&["config", "user.email"])?;
        Ok(if ok && !stdout.is_empty() {
            Some(stdout)
        } else {
            None
        })
    }

    pub fn has_origin(&self) -> Result<bool, GitError> {
        Ok(self.run_unchecked(&["remote", "get-url", "origin"])?.0)
    }

    pub fn fetch_origin(&self, branch: &str) -> Result<bool, GitError> {
        let _turn = FETCH_TURN.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.run_unchecked(&["fetch", "origin", branch])?.0)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<String, GitError> {
        self.run(&["merge-base", a, b])
    }

    /// The object each of `names` (`<rev>:<path>`, `<rev>^{commit}`, …)
    /// names, in order, `None` where there is none: one `git cat-file
    /// --batch-check` for all of them.
    pub fn object_ids(&self, names: &[String]) -> Result<Vec<Option<String>>, GitError> {
        use std::io::Write;
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let failed = |e: std::io::Error| GitError::Failed {
            cmd: "cat-file --batch-check".to_string(),
            code: -1,
            stderr: e.to_string(),
        };
        let mut child = Command::new("git")
            .args(["cat-file", "--batch-check=%(objectname)"])
            .current_dir(&self.dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(failed)?;
        // written from a thread of its own: a long list would otherwise
        // fill the pipe both ways
        let input: String = names.iter().map(|n| format!("{n}\n")).collect();
        let stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(input.as_bytes());
            }
        });
        let out = child.wait_with_output().map_err(failed)?;
        let _ = writer.join();
        if !out.status.success() {
            return Err(GitError::Failed {
                cmd: "cat-file --batch-check".to_string(),
                code: out.status.code().unwrap_or(-1),
                stderr: stderr_tail(&out),
            });
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let ids: Vec<Option<String>> = text
            .lines()
            .map(|l| (!l.ends_with(" missing") && !l.contains(' ')).then(|| l.to_string()))
            .collect();
        if ids.len() != names.len() {
            return Err(GitError::Failed {
                cmd: "cat-file --batch-check".to_string(),
                code: -1,
                stderr: format!("{} answers for {} names", ids.len(), names.len()),
            });
        }
        Ok(ids)
    }

    /// Repo-relative paths that differ between two commits (`git diff
    /// --name-only --no-renames -z`): a rename lists both its sides.
    pub fn changed_paths(&self, from: &str, to: &str) -> Result<Vec<String>, GitError> {
        self.name_list(&["diff", "--name-only", "--no-renames", "-z", from, to])
    }

    /// Paths whose entry in the index differs from `rev`.
    pub fn index_differs_from(&self, rev: &str) -> Result<Vec<String>, GitError> {
        self.name_list(&["diff", "--cached", "--name-only", "--no-renames", "-z", rev])
    }

    /// The NUL-separated path list a diff prints.
    fn name_list(&self, args: &[&str]) -> Result<Vec<String>, GitError> {
        let out = self.raw(args)?;
        if !out.status.success() {
            return Err(GitError::Failed {
                cmd: args.join(" "),
                code: out.status.code().unwrap_or(-1),
                stderr: stderr_tail(&out),
            });
        }
        Ok(out
            .stdout
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect())
    }

    /// The files under `roots` that git would commit — tracked, or untracked
    /// and not ignored (`ls-files --cached --others --exclude-standard`) —
    /// present on disk, relative to this directory; per root in order,
    /// sorted (`.` lists everything). Nested repositories and anything under
    /// `.git` or `.vitrify` are left out. What git ignores is never part of
    /// the governed tree or an SDD candidate, whatever the ecosystem.
    pub fn committable_files(&self, roots: &[String]) -> Result<Vec<String>, GitError> {
        let mut out = Vec::new();
        for sr in roots {
            let args = [
                "--literal-pathspecs",
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                sr.as_str(),
            ];
            let o = self.raw(&args)?;
            if !o.status.success() {
                return Err(GitError::Failed {
                    cmd: args.join(" "),
                    code: o.status.code().unwrap_or(-1),
                    stderr: stderr_tail(&o),
                });
            }
            let mut files: Vec<String> = o
                .stdout
                .split(|&b| b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .filter(|p| !p.ends_with('/'))
                .filter(|p| !p.split('/').any(|c| c == ".git" || c == ".vitrify"))
                .filter(|p| self.dir.join(p).is_file())
                .collect();
            files.sort();
            files.dedup();
            out.extend(files);
        }
        Ok(out)
    }

    /// A file's exact text at a commit (`git show <rev>:<path>`); `None` when
    /// the path is absent there or is not UTF-8.
    pub fn show_text(&self, rev: &str, path: &str) -> Result<Option<String>, GitError> {
        let out = self.raw(&["show", &format!("{rev}:{path}")])?;
        if !out.status.success() {
            return Ok(None);
        }
        Ok(String::from_utf8(out.stdout).ok())
    }

    pub fn is_ancestor(&self, a: &str, b: &str) -> Result<bool, GitError> {
        Ok(self
            .run_unchecked(&["merge-base", "--is-ancestor", a, b])?
            .0)
    }

    pub fn status_porcelain(&self) -> Result<String, GitError> {
        self.run(&["status", "--porcelain"])
    }

    /// Run with `--no-optional-locks` first and require exit 0; returns raw
    /// stdout. For looking at a tree another process is working in: git then
    /// never takes `index.lock` just to refresh its stat cache.
    fn run_readonly(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let mut argv = vec!["--no-optional-locks"];
        argv.extend_from_slice(args);
        let out = self.raw(&argv)?;
        if !out.status.success() {
            return Err(GitError::Failed {
                cmd: argv.join(" "),
                code: out.status.code().unwrap_or(-1),
                stderr: stderr_tail(&out),
            });
        }
        Ok(out.stdout)
    }

    /// Every changed or untracked path as `(XY, path)`, without taking the
    /// index lock: `run status` probes worktrees whose workers may be
    /// running git at the same moment.
    pub fn changes_readonly(&self) -> Result<Vec<(String, String)>, GitError> {
        let out =
            self.run_readonly(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        Ok(parse_porcelain_z(&out))
    }

    /// Lines added and removed against `rev` in the working tree, without
    /// taking the index lock. Binary files (`-`) count as 0.
    pub fn numstat_readonly(&self, rev: &str) -> Result<(u64, u64), GitError> {
        let out = self.run_readonly(&["diff", "--numstat", "--no-renames", rev])?;
        let text = String::from_utf8_lossy(&out);
        let mut sum = (0u64, 0u64);
        for line in text.lines() {
            let mut cols = line.split('\t');
            let added = cols.next().and_then(|c| c.parse::<u64>().ok());
            let removed = cols.next().and_then(|c| c.parse::<u64>().ok());
            sum.0 += added.unwrap_or(0);
            sum.1 += removed.unwrap_or(0);
        }
        Ok(sum)
    }

    /// `git diff --quiet <rev>` + `--cached`: clean against a commit.
    pub fn clean_against(&self, rev: &str) -> Result<bool, GitError> {
        let a = self.run_unchecked(&["diff", "--quiet", rev])?.0;
        let b = self.run_unchecked(&["diff", "--cached", "--quiet", rev])?.0;
        Ok(a && b)
    }

    /// Whether the working tree or index differs from `HEAD`, or holds
    /// untracked files git does not ignore.
    pub fn is_dirty(&self) -> Result<bool, GitError> {
        Ok(!self.status_porcelain()?.is_empty())
    }

    /// The shared `.git` directory of this repository (the primary
    /// checkout's, also from inside a worktree).
    pub fn common_dir(&self) -> Result<PathBuf, GitError> {
        let common = self.run(&["rev-parse", "--git-common-dir"])?;
        let mut path = PathBuf::from(&common);
        if !path.is_absolute() {
            path = self.dir.join(&path);
        }
        Ok(path.canonicalize().unwrap_or(path))
    }

    /// The ignore rule that covers `path` (repo-relative), as
    /// `<file>:<line> (<pattern>)` — `None` when git does not ignore it.
    /// Rules in `.gitignore` files outrank `.git/info/exclude`, so a hit in
    /// the latter means no in-tree ignore file matched.
    pub fn ignored_by(&self, path: &str) -> Result<Option<String>, GitError> {
        let (ok, stdout, _) =
            self.run_unchecked(&["check-ignore", "-v", "--no-index", "--", path])?;
        if !ok || stdout.is_empty() {
            return Ok(None);
        }
        let rule = stdout.split('\t').next().unwrap_or(&stdout);
        let mut parts = rule.splitn(3, ':');
        let (file, line, pattern) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        Ok(Some(format!("{file}:{line} ({pattern})")))
    }

    /// Add `pattern` to `.git/info/exclude` unless it is already there —
    /// ignoring a path for this clone only, with no ignore file in the tree.
    pub fn exclude_locally(&self, pattern: &str) -> Result<(), GitError> {
        let path = self.common_dir()?.join("info/exclude");
        let io = |e: std::io::Error| GitError::Failed {
            cmd: format!("write {}", path.display()),
            code: 1,
            stderr: e.to_string(),
        };
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(pattern);
        text.push('\n');
        std::fs::write(&path, text).map_err(io)
    }

    /// The full message of a commit (`log -1 --format=%B`).
    pub fn commit_message(&self, rev: &str) -> Result<String, GitError> {
        self.run(&["log", "-1", "--format=%B", rev])
    }

    /// Paths with unresolved merge conflicts in this worktree.
    pub fn conflicted_paths(&self) -> Vec<String> {
        match self.run_unchecked(&["diff", "--name-only", "--diff-filter=U"]) {
            Ok((true, stdout, _)) => stdout
                .lines()
                .map(str::to_string)
                .filter(|l| !l.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Paths added or modified between two commits (deletions left out).
    pub fn added_or_modified(&self, from: &str, to: &str) -> Result<Vec<String>, GitError> {
        let args = [
            "diff",
            "--name-only",
            "--no-renames",
            "--diff-filter=AM",
            "-z",
            from,
            to,
        ];
        let out = self.raw(&args)?;
        if !out.status.success() {
            return Err(GitError::Failed {
                cmd: args.join(" "),
                code: out.status.code().unwrap_or(-1),
                stderr: stderr_tail(&out),
            });
        }
        Ok(out
            .stdout
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect())
    }

    // ------------------------------------------------------- worktrees

    pub fn worktree_add(&self, path: &Path, rev: &str, detach: bool) -> Result<(), GitError> {
        let _turn = WORKTREE_TURN.lock().unwrap_or_else(|e| e.into_inner());
        let p = path.to_string_lossy().to_string();
        if detach {
            self.run(&["worktree", "add", "--detach", &p, rev])?;
        } else {
            self.run(&["worktree", "add", &p, rev])?;
        }
        Ok(())
    }

    /// A new worktree at `path` on a new branch forked from `rev`. Fails —
    /// and changes nothing — when the path or the branch already exists.
    pub fn worktree_add_branch(
        &self,
        path: &Path,
        branch: &str,
        rev: &str,
    ) -> Result<(), GitError> {
        let _turn = WORKTREE_TURN.lock().unwrap_or_else(|e| e.into_inner());
        let p = path.to_string_lossy().to_string();
        self.run(&["worktree", "add", "-q", &p, "-b", branch, rev])?;
        Ok(())
    }

    /// Remove a worktree. `git worktree remove` forgets it too; only when
    /// that fails (the directory was already gone) is the list pruned — a
    /// prune looks at every worktree, including one another process is in
    /// the middle of adding.
    pub fn worktree_remove(&self, path: &Path) -> Result<(), GitError> {
        let _turn = WORKTREE_TURN.lock().unwrap_or_else(|e| e.into_inner());
        let p = path.to_string_lossy().to_string();
        let (removed, _, _) = self.run_unchecked(&["worktree", "remove", "--force", &p])?;
        if !removed {
            let _ = self.run_unchecked(&["worktree", "prune"])?;
        }
        Ok(())
    }

    pub fn reset_hard(&self, rev: &str) -> Result<(), GitError> {
        self.run(&["reset", "--hard", rev])?;
        Ok(())
    }

    /// Bring index and working tree from `old` to `new` the way a
    /// fast-forward does (`read-tree -m -u`): files that differ between the
    /// two are updated, everything else — local changes included — is kept;
    /// a file with local changes, or an untracked file, in the way of an
    /// update makes it fail with nothing touched. For a checkout whose
    /// branch ref has already been moved to `new`. The index is refreshed
    /// first: without that a file merely touched since it was last read
    /// counts as changed.
    pub fn fast_forward_tree(&self, old: &str, new: &str) -> Result<(), GitError> {
        let _ = self.run_unchecked(&["update-index", "-q", "--refresh"]);
        self.run(&["read-tree", "-m", "-u", old, new])?;
        Ok(())
    }

    pub fn clean_fdq(&self) -> Result<(), GitError> {
        let _ = self.run_unchecked(&["clean", "-fdq"])?;
        Ok(())
    }

    /// Remove every file git does not track, ignored ones included, and
    /// nested repositories too (`clean -ffdx`).
    pub fn clean_all(&self) -> Result<(), GitError> {
        self.run(&["clean", "-ffdxq"])?;
        Ok(())
    }

    // ------------------------------------------------------- branches

    pub fn create_branch(&self, name: &str, rev: &str) -> Result<(), GitError> {
        self.run(&["branch", name, rev])?;
        Ok(())
    }

    pub fn delete_branch(&self, name: &str) -> Result<(), GitError> {
        let _ = self.run_unchecked(&["branch", "-D", name])?;
        Ok(())
    }

    pub fn branch_exists(&self, name: &str) -> Result<bool, GitError> {
        Ok(self.verify_ref(&format!("refs/heads/{name}"))?.is_some())
    }

    /// Short names of the local branches under `prefix` (`sdd/x/U-001`
    /// lists `sdd/x/U-001/c1`, `sdd/x/U-001/c2`, …).
    pub fn branches_under(&self, prefix: &str) -> Result<Vec<String>, GitError> {
        let pattern = format!("refs/heads/{}", prefix.trim_end_matches('/'));
        let out = self.run(&["for-each-ref", "--format=%(refname:short)", &pattern])?;
        Ok(out
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }

    // ------------------------------------------------------- write path

    pub fn add_all(&self) -> Result<(), GitError> {
        self.run(&["add", "-A"])?;
        Ok(())
    }

    /// The committer fallback `vitrify <vitrify@local>`, as `-c` options,
    /// when `user.email` is unset.
    fn identity(&self) -> Result<Vec<&'static str>, GitError> {
        Ok(if self.user_email()?.is_some() {
            Vec::new()
        } else {
            vec!["-c", "user.name=vitrify", "-c", "user.email=vitrify@local"]
        })
    }

    /// Commit with the committer fallback `vitrify <vitrify@local>` when
    /// `user.email` is unset.
    pub fn commit(&self, message: &str) -> Result<(), GitError> {
        let mut args = self.identity()?;
        args.extend(["commit", "-q", "-m", message]);
        self.run(&args)?;
        Ok(())
    }

    /// Commit whatever is staged — nothing, if nothing is — under `subject`
    /// with `body` as the rest of the message. Used to leave a note on a
    /// branch whose work is being set aside.
    pub fn commit_note(&self, subject: &str, body: &str) -> Result<(), GitError> {
        let mut args = self.identity()?;
        args.extend(["commit", "-q", "--allow-empty", "-m", subject]);
        if !body.trim().is_empty() {
            args.extend(["-m", body]);
        }
        self.run(&args)?;
        Ok(())
    }

    /// Copy `paths` as they are at `rev` into this worktree, left unstaged.
    pub fn restore_paths(&self, rev: &str, paths: &[String]) -> Result<(), GitError> {
        for chunk in paths.chunks(64) {
            let mut args = vec!["--literal-pathspecs", "checkout", rev, "--"];
            args.extend(chunk.iter().map(String::as_str));
            self.run(&args)?;
        }
        if !paths.is_empty() {
            self.run(&["reset", "-q"])?;
        }
        Ok(())
    }

    pub fn merge_no_ff(&self, branch: &str, message: &str) -> Result<(), GitError> {
        let mut args = self.identity()?;
        args.extend(["merge", "--no-ff", "-m", message, branch]);
        self.run(&args)?;
        Ok(())
    }

    pub fn merge_abort(&self) -> Result<(), GitError> {
        let _ = self.run_unchecked(&["merge", "--abort"])?;
        Ok(())
    }

    /// The CAS: `update-ref refs/heads/<mainline> <new> <old>` — updates only
    /// if the current value is `<old>`; rejection is a typed `LostRace`.
    pub fn update_ref_cas(&self, refname: &str, new: &str, old: &str) -> Result<(), GitError> {
        let (ok, _, stderr) = self.run_unchecked(&["update-ref", refname, new, old])?;
        if ok {
            Ok(())
        } else {
            Err(GitError::LostRace { stderr })
        }
    }

    /// `push origin <new>:refs/heads/<mainline>`; a push rejection is a
    /// typed `LostRace`, any other failure is an error.
    pub fn push_cas(&self, new: &str, mainline: &str) -> Result<(), GitError> {
        let refspec = format!("{new}:refs/heads/{mainline}");
        let (ok, _, stderr) = self.run_unchecked(&["push", "origin", &refspec])?;
        if ok {
            Ok(())
        } else if stderr.contains("[rejected]")
            || stderr.contains("non-fast-forward")
            || stderr.contains("fetch first")
            || stderr.contains("failed to push")
        {
            Err(GitError::LostRace { stderr })
        } else {
            Err(GitError::Failed {
                cmd: format!("push origin {refspec}"),
                code: 1,
                stderr,
            })
        }
    }

    // ------------------------------------------------------- evidence

    pub fn log_oneline(&self, n: usize) -> Result<String, GitError> {
        self.run(&["log", "--oneline", &format!("-{n}")])
    }

    // ------------------------------------------------------- incremental

    /// The commits on the first-parent chain of `to` that `from` does not
    /// reach, oldest first (`rev-list --first-parent --reverse from..to`).
    pub fn first_parent_range(&self, from: &str, to: &str) -> Result<Vec<String>, GitError> {
        let out = self.run(&[
            "rev-list",
            "--first-parent",
            "--reverse",
            &format!("{from}..{to}"),
        ])?;
        Ok(out
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// The subject line of a commit (`log -1 --format=%s`).
    pub fn subject(&self, rev: &str) -> Result<String, GitError> {
        self.run(&["log", "-1", "--format=%s", rev])
    }

    /// The abbreviated name of a commit (`rev-parse --short`).
    pub fn short_sha(&self, rev: &str) -> Result<String, GitError> {
        self.run(&["rev-parse", "--short", rev])
    }

    /// The full name of the commit `rev` names (`rev-parse -q --verify
    /// <rev>^{commit}`) — a recorded short sha, say; `None` when it names
    /// no commit here: history rewritten, a shallow clone, an ambiguous
    /// prefix.
    pub fn resolve_commit(&self, rev: &str) -> Result<Option<String>, GitError> {
        self.verify_ref(&format!("{rev}^{{commit}}"))
    }
}

/// Mainline resolution, in order of precedence:
/// `--mainline` flag > config `mainline` > the primary checkout's current
/// branch > `origin/HEAD` > `main`/`master` > error.
///
/// The checked-out branch outranks `origin/HEAD`, so a run started — or
/// resumed — from a checkout on a build branch publishes to that branch,
/// and the checkout follows what it publishes. `origin/HEAD` is consulted
/// only when the primary checkout is detached.
pub fn resolve_mainline(
    primary: &Git,
    flag: Option<&str>,
    config_mainline: &str,
) -> Result<String, GitError> {
    if let Some(m) = flag
        && !m.is_empty()
    {
        return Ok(m.to_string());
    }
    if !config_mainline.is_empty() {
        return Ok(config_mainline.to_string());
    }
    if let Some(b) = primary.current_branch()? {
        return Ok(b);
    }
    if let Some(b) = primary.origin_head_branch()? {
        return Ok(b);
    }
    for cand in ["main", "master"] {
        if primary.verify_ref(&format!("refs/heads/{cand}"))?.is_some() {
            return Ok(cand.to_string());
        }
    }
    Err(GitError::Failed {
        cmd: "resolve-mainline".into(),
        code: 1,
        stderr: "cannot resolve mainline branch; pass --mainline".into(),
    })
}

/// `use_push` arbitration: origin exists AND
/// `fetch origin <mainline>` succeeds.
pub fn use_push(primary: &Git, mainline: &str) -> Result<bool, GitError> {
    if !primary.has_origin()? {
        return Ok(false);
    }
    if !primary.fetch_origin(mainline)? {
        crate::out::diag(&format!(
            "note: origin exists but fetch of {mainline} failed; arbitrating locally this time"
        ));
        return Ok(false);
    }
    Ok(true)
}

/// `status --porcelain=v1 -z` output as `(XY, path)` pairs. A rename or
/// copy entry is followed by one more NUL-terminated field, its original
/// path, which is skipped: the entry counts once, under its new path.
pub fn parse_porcelain_z(out: &[u8]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut fields = out.split(|&b| b == 0);
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let xy = String::from_utf8_lossy(&field[..2]).into_owned();
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        if xy.contains('R') || xy.contains('C') {
            fields.next();
        }
        entries.push((xy, path));
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_z_skips_a_renames_original_path() {
        let out = b" M src/a.rs\0R  new.rs\0old.rs\0?? notes/x y.txt\0A  b.txt\0";
        assert_eq!(
            parse_porcelain_z(out),
            vec![
                (" M".to_string(), "src/a.rs".to_string()),
                ("R ".to_string(), "new.rs".to_string()),
                ("??".to_string(), "notes/x y.txt".to_string()),
                ("A ".to_string(), "b.txt".to_string()),
            ]
        );
        assert!(parse_porcelain_z(b"").is_empty());
    }
}
