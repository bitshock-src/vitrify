//! Worker dispatch and report extraction. One worker = one
//! OS process: recorded argv template + generated prompt as the final
//! argument, cwd = the unit worktree, its own Unix process group, a private
//! `TMPDIR`, stdout/stderr written straight to
//! `.vitrify/logs/<job>/worker-<holder>.{out,err}` as the harness produces
//! them — with a streaming spelling, a worker that is killed leaves its
//! session up to that point.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use vitrify_core::CoreError;

/// The C-004 worker report line.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub unit: String,
    pub outcome: String, // built | blocked | released
    pub branch: String,
    pub claim_seq: i64,
    pub q: Option<String>,
    pub notes: String,
}

/// A spawned worker: child process (own process group), log paths.
pub struct WorkerHandle {
    pub holder: String,
    pub unit: String,
    pub child: Child,
    pub pid: u32,
    pub out_path: PathBuf,
    pub err_path: PathBuf,
}

/// Spawn one worker per C-003. A missing template binary is a named error —
/// never guess a spelling.
pub fn spawn(
    argv_template: &[String],
    prompt: &str,
    cwd: &Path,
    logs_dir: &Path,
    holder: &str,
    unit: &str,
) -> Result<WorkerHandle, CoreError> {
    spawn_in(argv_template, prompt, cwd, logs_dir, holder, unit, None)
}

/// [`spawn`] with a private temp directory: workers build side by side, and
/// test runners that share one temp root trip over each other.
pub fn spawn_in(
    argv_template: &[String],
    prompt: &str,
    cwd: &Path,
    logs_dir: &Path,
    holder: &str,
    unit: &str,
    tmpdir: Option<&Path>,
) -> Result<WorkerHandle, CoreError> {
    if argv_template.is_empty() {
        return Err(CoreError::refusal(
            2,
            "error: empty worker argv template: ~/.vitrify/config.toml records no headless spelling → vitrify probe",
        ));
    }
    std::fs::create_dir_all(logs_dir).map_err(|source| CoreError::Io {
        path: logs_dir.to_path_buf(),
        source,
    })?;
    let out_path = logs_dir.join(format!("worker-{holder}.out"));
    let err_path = logs_dir.join(format!("worker-{holder}.err"));
    let out_file = std::fs::File::create(&out_path).map_err(|source| CoreError::Io {
        path: out_path.clone(),
        source,
    })?;
    let err_file = std::fs::File::create(&err_path).map_err(|source| CoreError::Io {
        path: err_path.clone(),
        source,
    })?;

    let mut cmd = Command::new(&argv_template[0]);
    cmd.args(&argv_template[1..])
        .arg(prompt)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .process_group(0);
    if let Some(tmp) = tmpdir {
        cmd.env("TMPDIR", tmp);
    }
    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CoreError::refusal(
                2,
                format!(
                    "error: probed worker binary not found: {} — re-run `vitrify probe`",
                    argv_template[0]
                ),
            )
        } else {
            CoreError::refusal(1, format!("error: failed to spawn worker: {e}"))
        }
    })?;
    let pid = child.id();
    Ok(WorkerHandle {
        holder: holder.to_string(),
        unit: unit.to_string(),
        child,
        pid,
        out_path,
        err_path,
    })
}

/// SIGKILL a worker's whole process group (`kill(2)`, declared in
/// `signals.rs` — no shell, no subprocess, no crate).
pub fn kill_group(pid: u32) {
    crate::gate::kill_group(pid);
}

/// The extraction rule: if the output is the
/// probed JSON envelope/event stream, extract the assistant's final text;
/// else raw stdout; in either case parse the LAST NON-EMPTY LINE against
/// the schema — models habitually wrap the line in a markdown code fence,
/// so fence-delimiter lines don't count as content. `None` =
/// missing/unparseable (treated as released + failed).
pub fn extract_report(stdout_text: &str) -> Option<Report> {
    let text = extract_final_text(stdout_text);
    if let Some(report) = last_content_line(&text).and_then(parse_report_line) {
        return Some(report);
    }
    // An event stream with a line that is not an event (a truncated final
    // write, a stray warning) is still an event stream: take the last event
    // whose text ends in a report line.
    stdout_text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .filter_map(|v| envelope_text(&v))
        .find_map(|t| last_content_line(&t).and_then(parse_report_line))
}

/// The last non-empty line that is not a code-fence delimiter (``` / ~~~,
/// with or without an info string).
pub(crate) fn last_content_line(text: &str) -> Option<&str> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("```") && !l.starts_with("~~~"))
}

/// Shared with the escalation client: verdict replies arrive through the
/// same recorded spellings and need the same unwrapping.
pub(crate) fn extract_final_text(stdout_text: &str) -> String {
    // the whole output one JSON value: a single envelope, or a bare reply
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout_text) {
        if let Some(t) = envelope_text(&v) {
            return t;
        }
        return stdout_text.to_string();
    }
    // NDJSON event stream: every non-empty line is JSON
    let lines: Vec<&str> = stdout_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.len() > 1 {
        let parsed: Vec<Option<serde_json::Value>> = lines
            .iter()
            .map(|l| serde_json::from_str(l.trim()).ok())
            .collect();
        if parsed.iter().all(Option::is_some) {
            // the assistant's final text: last event carrying one
            for v in parsed.iter().rev().flatten() {
                if let Some(t) = envelope_text(v) {
                    return t;
                }
            }
        }
    }
    stdout_text.to_string()
}

/// Text payload of a harness JSON envelope/event, across the probed
/// formats' spellings.
fn envelope_text(v: &serde_json::Value) -> Option<String> {
    for key in ["result", "text", "last_agent_message", "content"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    // claude stream-json: {"type":"assistant","message":{"content":[{"type":"text","text":...}]}}
    if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
        let mut out = String::new();
        for item in content {
            if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn parse_report_line(line: &str) -> Option<Report> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let outcome = v.get("outcome")?.as_str()?.to_string();
    if !["built", "blocked", "released"].contains(&outcome.as_str()) {
        return None;
    }
    Some(Report {
        unit: v.get("unit")?.as_str()?.to_string(),
        outcome,
        branch: v
            .get("branch")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        claim_seq: v.get("claim_seq").and_then(|x| x.as_i64()).unwrap_or(0),
        q: v.get("q").and_then(|x| x.as_str()).map(str::to_string),
        notes: v
            .get("notes")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// The missing/unparseable-report rule: treated as `released` with
/// `notes: "unparseable report"`; the unit counts failed for the wave.
pub fn report_or_released(stdout_text: &str, unit: &str) -> (Report, bool) {
    match extract_report(stdout_text) {
        Some(r) => (r, true),
        None => (
            Report {
                unit: unit.to_string(),
                outcome: "released".to_string(),
                branch: String::new(),
                claim_seq: 0,
                q: None,
                notes: "unparseable report".to_string(),
            },
            false,
        ),
    }
}
