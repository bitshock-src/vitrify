//! `vitrify run status`: what a run is doing, unit by unit. Everything comes
//! from what vitrify records itself — `run.json`, `gate.json`,
//! `events.jsonl`, the worker logs, process liveness, and each worker
//! worktree's git state and front matter — never from a harness's output.
//! It writes no file, takes no lock, and runs git only with
//! `--no-optional-locks`, so it can be pointed at a live run from a second
//! terminal without disturbing it.

pub mod follow;
pub mod render;
pub mod snapshot;

use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::model::UtcTime;

use crate::cli::StatusArgs;
use crate::cmd;
use crate::git::Git;
use crate::out::emit;

pub fn run(args: &StatusArgs) -> Result<u8, CoreError> {
    let root = primary_root();
    let job = resolve_job(&root, args.job.as_deref())?;
    if args.follow {
        return follow::run(&root, &job, args);
    }
    let snap = snapshot::take(&root, &job, UtcTime::now(), args.quiet_after)?;
    if args.json {
        emit(&render::json(&snap).dumps_indent2());
    } else {
        for line in render::human(&snap) {
            emit(&line);
        }
    }
    // a run was shown, whatever its state
    Ok(0)
}

/// The primary checkout, whichever checkout the verb runs in: a run's
/// records live in its `.vitrify/`.
pub fn primary_root() -> PathBuf {
    Git::new(cmd::cwd())
        .primary_root()
        .unwrap_or_else(|_| cmd::repo_root())
}

/// The job to show: `--job`, else the latest run recorded in this
/// repository. Only the job ID is taken from `.vitrify/run.json`: two runs
/// at once take turns writing that file, so everything else is read from
/// the job's own copy.
pub fn resolve_job(root: &Path, job: Option<&str>) -> Result<String, CoreError> {
    if let Some(job) = job {
        // a name, never a path out of the logs directory
        if job.is_empty() || job.contains('/') || job == ".." {
            return Err(CoreError::refusal(
                2,
                "error: --job takes a job ID such as run-20261116T140212Z-p48113",
            ));
        }
        return Ok(job.to_string());
    }
    let path = root.join(".vitrify").join("run.json");
    let none = || {
        CoreError::refusal(
            4,
            "error: no run recorded in this repository → start and watch one: vitrify brief run",
        )
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(none()),
        Err(e) => return Err(cannot_read(&path, e)),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| cannot_read(&path, e))?;
    match v["job_id"].as_str() {
        Some(job) if !job.is_empty() && !job.contains('/') && job != ".." => Ok(job.to_string()),
        _ => Err(none()),
    }
}

/// The exit-1 refusal for a record that exists but cannot be read.
pub fn cannot_read(path: &Path, e: impl std::fmt::Display) -> CoreError {
    CoreError::refusal(1, format!("error: cannot read {}: {e}", path.display()))
}
