//! Interrupt handling for `vitrify run`. SIGINT, SIGTERM and SIGHUP set a
//! flag; the orchestrator polls it, stops its workers and gate commands,
//! releases its claims and writes `run.json` before the process ends. A
//! second signal ends the process at once. A signal the process was started
//! ignoring (`nohup`, a background job of a non-interactive shell) stays
//! ignored.
//!
//! Also here: sending a signal to a process or a process group, for the
//! workers and commands vitrify starts in groups of their own.
//!
//! No crate is involved: the four C library calls are declared here, and
//! the handler touches nothing but an atomic (and `_exit`), which is all a
//! signal handler may safely do.

use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub const SIGHUP: i32 = 1;
pub const SIGINT: i32 = 2;
pub const SIGTERM: i32 = 15;

/// The signal that interrupted the run (0 = none).
static PENDING: AtomicI32 = AtomicI32::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The run itself decided to stop (an error, an `abort` verdict).
static STOPPING: AtomicBool = AtomicBool::new(false);

/// `SIG_DFL` / `SIG_IGN`: the default and the ignoring disposition, on
/// every supported platform.
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const SIGKILL: c_int = 9;
/// `EPERM`: the target exists but may not be signalled.
const EPERM: i32 = 1;

unsafe extern "C" {
    fn signal(signum: c_int, handler: usize) -> usize;
    fn raise(signum: c_int) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn _exit(status: c_int) -> !;
}

extern "C" fn on_signal(sig: c_int) {
    if PENDING.swap(sig, Ordering::SeqCst) != 0 {
        // SAFETY: `_exit` is async-signal-safe. A second signal means the
        // operator will not wait for the shutdown already under way.
        unsafe { _exit(128 + sig) }
    }
}

/// Route SIGINT, SIGTERM and SIGHUP to the flag. Idempotent.
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handler = on_signal as extern "C" fn(c_int) as usize;
    for sig in [SIGHUP, SIGINT, SIGTERM] {
        // SAFETY: `signal` with a handler that only stores to an atomic (or
        // calls `_exit`) is sound; the handler is a plain `extern "C"` fn
        // that lives for the whole program.
        unsafe {
            // whoever started this process ignoring the signal meant it
            if signal(sig, handler) == SIG_IGN {
                signal(sig, SIG_IGN);
            }
        }
    }
}

/// SIGKILL every process of the group `pgid` leads. Groups 0 and 1 are
/// never a child's: `kill(-1, …)` would mean "everything", so they are
/// refused outright.
pub fn kill_group(pgid: u32) {
    if pgid > 1 && pgid <= i32::MAX as u32 {
        // SAFETY: a plain system call; a group that is already gone is an
        // error return, not undefined behaviour.
        unsafe {
            kill(-(pgid as c_int), SIGKILL);
        }
    }
}

/// Whether process `pid` — or, with `group`, any member of the process
/// group it leads — still exists (signal 0: the existence check).
pub fn alive(pid: u32, group: bool) -> bool {
    if pid <= 1 || pid > i32::MAX as u32 {
        return false;
    }
    let target = if group { -(pid as c_int) } else { pid as c_int };
    // SAFETY: signal 0 sends nothing; it only reports whether the target
    // exists.
    let sent = unsafe { kill(target, 0) };
    sent == 0 || std::io::Error::last_os_error().raw_os_error() == Some(EPERM)
}

/// Whether this process handles interrupts itself — if so, its children run
/// in their own process groups and are stopped by the orchestrator, not by
/// the terminal.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// The pending interrupt, if any.
pub fn pending() -> Option<i32> {
    match PENDING.load(Ordering::SeqCst) {
        0 => None,
        sig => Some(sig),
    }
}

/// Ask everything that polls for an interrupt to stop, without a signal:
/// the run is ending on its own account.
pub fn request_stop() {
    STOPPING.store(true, Ordering::SeqCst);
}

/// Whether running commands should stop: a signal arrived, or the run is
/// ending.
pub fn interrupted() -> bool {
    pending().is_some() || STOPPING.load(Ordering::SeqCst)
}

pub fn name(sig: i32) -> &'static str {
    match sig {
        SIGHUP => "SIGHUP",
        SIGINT => "SIGINT",
        SIGTERM => "SIGTERM",
        _ => "signal",
    }
}

/// End the process the way the signal would have: restore the default
/// disposition and raise it again, so the parent sees a death by signal
/// rather than an exit code.
pub fn die_by(sig: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: restoring the default disposition and raising the signal in
    // the current thread; nothing else runs afterwards.
    unsafe {
        signal(sig, SIG_DFL);
        raise(sig);
    }
    std::process::exit(128 + sig)
}
