//! Running an SDD's own commands — the §5 Commands and Verification-table
//! rows — through `sh -c`: the one sanctioned shell use, and the one place
//! it happens.
//!
//! Every command gets a `TMPDIR` of its own for as long as it runs, so
//! concurrent test sessions (the gate's own rows, and workers building in
//! parallel) never share a temp root. The directories are slots, not
//! throwaways: a slot is exclusive while in use and stays where it is
//! afterwards, to be taken by the next command — so a build daemon a command
//! leaves behind keeps a temp directory that exists, and toolchains that tie
//! a daemon to its temp directory see the same few directories every time.
//! Under `vitrify run` each command also runs in its own process group and
//! is stopped when the run is interrupted.

use std::collections::HashMap;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use vitrify_core::CoreError;
use vitrify_core::audit::{self, CommandRunner};
use vitrify_core::model::{self, Instance, Shape, Unit};
use vitrify_core::trace::hash12;

use crate::signals;

/// Told as each command starts and ends: how the integrator reports its
/// progress while a gate that prints nothing is passing.
pub trait Watch: Sync {
    fn started(&self, cmd: &str);
    fn ended(&self, cmd: &str);
}

/// A watcher that does nothing, for callers no one is watching.
pub struct Unwatched;

impl Watch for Unwatched {
    fn started(&self, _cmd: &str) {}
    fn ended(&self, _cmd: &str) {}
}

/// One finished command.
#[derive(Debug, Clone)]
pub struct Ran {
    pub code: i32,
    /// The last 100 lines of stdout and stderr, as they were written.
    pub output: String,
    /// Stopped because the run was interrupted, not because it failed.
    pub interrupted: bool,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == 0 && !self.interrupted
    }
}

/// A temp directory held by one command or one worker. Exclusive while this
/// value lives; the directory itself is left in place for the next holder.
pub struct TmpSlot {
    pub dir: PathBuf,
    held: std::fs::File,
}

impl Drop for TmpSlot {
    /// Unlock before the file closes. A lock belongs to the open file, and
    /// every child forked meanwhile — by any thread — shares that until it
    /// execs: left to the close, the lock can outlive the slot, and the next
    /// command pass the slot over for another.
    fn drop(&mut self) {
        let _ = self.held.unlock();
    }
}

/// Where the temp slots of `scope` live: a private directory under the
/// system temp root, named by a hash of the user and the scope (kept short —
/// tools put sockets under `TMPDIR`, and socket paths are length-limited).
pub fn slots_root(scope: &Path) -> PathBuf {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let key = hash12(&format!("{user}\0{}", scope.display()));
    std::env::temp_dir().join(format!("vfy-{}", &key[..10]))
}

/// Take the first free slot of `kind` for `scope` (the checkout commands
/// run in, or the root workers build under): the same few directories are
/// handed out again and again, never two holders at once — across
/// processes too, since a slot is held by a file lock.
pub fn tmp_slot(scope: &Path, kind: &str) -> std::io::Result<TmpSlot> {
    let root = slots_root(scope);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    for k in 0..4096 {
        let held = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(format!("{kind}{k}.lock")))?;
        match held.try_lock() {
            Ok(()) => {
                let dir = root.join(format!("{kind}{k}"));
                std::fs::create_dir_all(&dir)?;
                // The system's temp cleaner removes what has not been used
                // for days. Taking a slot is using it: say so, or a lock
                // file created long ago could be swept from under its
                // holder — and the slot handed out twice.
                let now = std::time::SystemTime::now();
                let _ = held.set_modified(now);
                if let Ok(d) = std::fs::File::open(&dir) {
                    let _ = d.set_modified(now);
                }
                return Ok(TmpSlot { dir, held });
            }
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temp directory slot"))
}

fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// SIGKILL a whole process group.
pub fn kill_group(pgid: u32) {
    signals::kill_group(pgid);
}

/// Whether a process (`pid`) — or, with `group`, any member of the process
/// group it leads — still exists.
pub fn alive(pid: u32, group: bool) -> bool {
    signals::alive(pid, group)
}

/// Run one command via `sh -c` in `cwd`, with a `TMPDIR` of its own.
pub fn run_command(cmd: &str, cwd: &Path) -> Ran {
    let failed = |why: String| Ran {
        code: -1,
        output: why,
        interrupted: false,
    };
    if signals::interrupted() {
        return Ran {
            code: -1,
            output: String::new(),
            interrupted: true,
        };
    }
    let slot = match tmp_slot(cwd, "c") {
        Ok(slot) => slot,
        Err(e) => return failed(format!("cannot take a temp directory: {e}")),
    };
    let tmp = &slot.dir;
    let log = tmp.with_extension("out");
    let cleanup = || {
        let _ = std::fs::remove_file(&log);
    };
    let (out, err) = match std::fs::File::create(&log).and_then(|f| Ok((f.try_clone()?, f))) {
        Ok(pair) => pair,
        Err(e) => {
            cleanup();
            return failed(format!("cannot capture output: {e}"));
        }
    };
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .env("TMPDIR", tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    let own_group = signals::installed();
    if own_group {
        // the terminal's Ctrl-C reaches only vitrify, which stops the group
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            cleanup();
            return failed(e.to_string());
        }
    };
    let mut interrupted = false;
    let status = if own_group {
        let mut nap = Duration::from_millis(2);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(_) => break None,
            }
            if signals::interrupted() {
                kill_group(child.id());
                interrupted = true;
                break child.wait().ok();
            }
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(50));
        }
    } else {
        child.wait().ok()
    };
    let text = std::fs::read(&log)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    cleanup();
    Ran {
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        output: tail(&text, 100),
        interrupted,
    }
}

/// Run one command, telling `w` as it starts and ends.
pub fn run_command_watched(cmd: &str, cwd: &Path, w: &dyn Watch) -> Ran {
    w.started(cmd);
    let ran = run_command(cmd, cwd);
    w.ended(cmd);
    ran
}

/// Run `cmds` in `cwd`, at most `jobs` at once; results are index-aligned.
///
/// With `jobs` > 1 a command that fails while others were running is run
/// once more on its own, in order: only a failure that repeats alone
/// counts, so rows that merely disturb each other cannot turn a gate red.
/// `stop_at_failure` ends the pass at the first (confirmed) failure — later
/// commands are `None`.
pub fn run_rows(
    cmds: &[String],
    cwd: &Path,
    jobs: usize,
    stop_at_failure: bool,
) -> Vec<Option<Ran>> {
    run_rows_watched(cmds, cwd, jobs, stop_at_failure, &Unwatched)
}

/// `run_rows`, telling `w` as each command starts and ends — the parallel
/// pass and each re-run that confirms a failure alone.
pub fn run_rows_watched(
    cmds: &[String],
    cwd: &Path,
    jobs: usize,
    stop_at_failure: bool,
    w: &dyn Watch,
) -> Vec<Option<Ran>> {
    let mut results: Vec<Option<Ran>> = vec![None; cmds.len()];
    if jobs <= 1 || cmds.len() <= 1 {
        for (i, cmd) in cmds.iter().enumerate() {
            let ran = run_command_watched(cmd, cwd, w);
            let stop = ran.interrupted || (stop_at_failure && !ran.ok());
            results[i] = Some(ran);
            if stop {
                break;
            }
        }
        return results;
    }

    let next = AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Ran>>> =
        cmds.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(cmds.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= cmds.len() || signals::interrupted() {
                        break;
                    }
                    let ran = run_command_watched(&cmds[i], cwd, w);
                    *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(ran);
                }
            });
        }
    });
    for (i, slot) in slots.into_iter().enumerate() {
        results[i] = slot.into_inner().unwrap_or_else(|e| e.into_inner());
    }
    // confirm each failure alone, in table order
    for (i, cmd) in cmds.iter().enumerate() {
        let Some(first) = &results[i] else { continue };
        if first.ok() || first.interrupted {
            continue;
        }
        let again = run_command_watched(cmd, cwd, w);
        let failed = !again.ok();
        results[i] = Some(again);
        if failed && stop_at_failure {
            for later in results.iter_mut().skip(i + 1) {
                if later.as_ref().is_some_and(|r| !r.ok()) {
                    *later = None; // unconfirmed: not judged
                }
            }
            break;
        }
    }
    results
}

/// The text of a unit's entry: its unit file (bundle) or its `### U-###`
/// region (single).
pub fn unit_text(inst: &Instance, unit: &Unit) -> Result<String, CoreError> {
    match inst.shape {
        Shape::Bundle => model::read_text(&unit.file),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            Ok(model::find_unit_span(&text, &unit.unit)
                .map(|(s, e)| text[s..e].to_string())
                .unwrap_or_default())
        }
    }
}

/// The commands of a unit's Verification table, in table order.
pub fn verification_table(inst: &Instance, unit: &Unit) -> Result<Vec<String>, CoreError> {
    Ok(audit::verification_commands(&unit_text(inst, unit)?))
}

/// The audit's command runner (check 4): plain `sh -c`, one at a time.
pub struct ShRunner;

impl CommandRunner for ShRunner {
    fn run(&self, cmd: &str, cwd: &Path) -> i32 {
        run_command(cmd, cwd).code
    }
}

/// Every Verified unit's Verification rows, each distinct command once, in
/// unit and table order: what `MemoRunner::for_verified` runs, so a caller
/// can count them first.
pub fn verified_commands(inst: &Instance) -> Result<Vec<String>, CoreError> {
    let mut cmds: Vec<String> = Vec::new();
    for u in &inst.units {
        if u.meta.get_str_or("status", "Draft") == "Verified" {
            for cmd in verification_table(inst, u)? {
                if !cmds.contains(&cmd) {
                    cmds.push(cmd);
                }
            }
        }
    }
    Ok(cmds)
}

/// A runner that already knows the exit codes of the commands it will be
/// asked for: every Verified unit's Verification rows, each distinct command
/// run once, `jobs` at a time. The audit then reads the results.
pub struct MemoRunner {
    memo: HashMap<String, i32>,
}

impl MemoRunner {
    /// Run every Verified unit's rows now.
    pub fn for_verified(inst: &Instance, cwd: &Path, jobs: usize) -> Result<MemoRunner, CoreError> {
        MemoRunner::for_verified_watched(inst, cwd, jobs, &Unwatched)
    }

    /// `for_verified`, telling `w` as each command starts and ends.
    pub fn for_verified_watched(
        inst: &Instance,
        cwd: &Path,
        jobs: usize,
        w: &dyn Watch,
    ) -> Result<MemoRunner, CoreError> {
        let cmds = verified_commands(inst)?;
        let results = run_rows_watched(&cmds, cwd, jobs, false, w);
        let memo = cmds
            .into_iter()
            .zip(results)
            .filter_map(|(cmd, ran)| ran.map(|r| (cmd, r.code)))
            .collect();
        Ok(MemoRunner { memo })
    }
}

impl CommandRunner for MemoRunner {
    fn run(&self, cmd: &str, cwd: &Path) -> i32 {
        match self.memo.get(cmd) {
            Some(code) => *code,
            None => run_command(cmd, cwd).code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn a_slot_let_go_is_free_at_once_though_other_threads_spawn() {
        // Children spawned by other threads share a held slot's open lock
        // file until they exec. A slot taken right after it was let go must
        // be the same one regardless.
        let scope = tempfile::tempdir().unwrap();
        let root = slots_root(scope.path());
        let stop = AtomicBool::new(false);
        let taken: Vec<std::io::Result<PathBuf>> = std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = Command::new("true").status();
                    }
                });
            }
            let taken = (0..300)
                .map(|_| {
                    let slot = tmp_slot(scope.path(), "c")?;
                    let _ = Command::new("true").status(); // held while a command runs
                    Ok(slot.dir.clone())
                })
                .collect();
            stop.store(true, Ordering::Relaxed);
            taken
        });
        let _ = std::fs::remove_dir_all(&root);
        let passed_over = taken
            .iter()
            .filter(|t| t.as_ref().ok() != Some(&root.join("c0")))
            .count();
        assert_eq!(
            passed_over,
            0,
            "{passed_over} of {} takes passed c0 over",
            taken.len()
        );
    }
}
