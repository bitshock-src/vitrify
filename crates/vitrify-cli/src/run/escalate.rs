//! Escalation client: assemble the evidence prompt from
//! the resolved template, make ONE inference call through the strongest
//! probed harness, parse the structured verdict, and return it typed. The
//! orchestrator enforces verdicts; this module performs no state change.
//! One malformed reply gets one re-ask with the parse error
//! appended; a second malformed reply yields `abort`.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use vitrify_core::CoreError;

use crate::home;
use crate::out::J;

#[derive(Debug, Clone)]
pub struct Evidence {
    pub unit: String,
    pub branch: String,
    pub fencing: String,
    pub command: String,
    pub output_tail: String,
    pub journal_tail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictKind {
    Retry,
    Release,
    Block,
    Abort,
}

impl VerdictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictKind::Retry => "retry",
            VerdictKind::Release => "release",
            VerdictKind::Block => "block",
            VerdictKind::Abort => "abort",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub verdict: VerdictKind,
    pub unit: Option<String>,
    pub q: Option<String>,
    pub reason: String,
}

fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Fill the template's evidence slots: output tail truncated to the
/// last 100 lines, journal tail to the last 20 events.
pub fn build_prompt(
    home_dir: &Path,
    version: &str,
    evidence: &Evidence,
) -> Result<String, CoreError> {
    let (template, _) = home::resolve(home_dir, version, "briefs/escalation.md")?;
    Ok(template
        .replace("{unit}", &evidence.unit)
        .replace("{branch}", &evidence.branch)
        .replace("{fencing}", &evidence.fencing)
        .replace("{command}", &evidence.command)
        .replace("{output_tail}", &tail_lines(&evidence.output_tail, 100))
        .replace("{journal_tail}", &tail_lines(&evidence.journal_tail, 20)))
}

fn invoke(argv: &[String], prompt: &str) -> Result<String, CoreError> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .arg(prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| {
        CoreError::refusal(1, format!("error: failed to spawn escalation call: {e}"))
    })?;
    // Read on a thread, into a buffer this thread can take at any moment:
    // an interrupt must not wait for the judge, and a process the judge
    // left behind holding the pipe open must not hold the run.
    let reply = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let reader = child.stdout.take().map(|mut stdout| {
        let reply = std::sync::Arc::clone(&reply);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                reply
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(&chunk[..n]);
            }
        })
    });
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {}
        }
        if crate::signals::interrupted() {
            crate::gate::kill_group(child.id());
            let _ = child.wait();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // the judge has exited: its output is in the pipe; give the reader a
    // moment to drain it, no longer
    if let Some(reader) = reader {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !reader.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let bytes = reply.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_verdict(reply: &str) -> Result<Verdict, String> {
    // Replies arrive through the recorded headless spelling, so they may be
    // wrapped in the same JSON envelope/event stream as worker reports;
    // unwrap identically, then parse the last non-empty, non-fence line.
    let text = super::worker::extract_final_text(reply);
    let Some(line) = super::worker::last_content_line(&text) else {
        return Err("empty reply".to_string());
    };
    let v: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| format!("invalid JSON: {e}"))?;
    let kind = match v.get("verdict").and_then(|x| x.as_str()) {
        Some("retry") => VerdictKind::Retry,
        Some("release") => VerdictKind::Release,
        Some("block") => VerdictKind::Block,
        Some("abort") => VerdictKind::Abort,
        other => return Err(format!("unknown verdict {other:?}")),
    };
    let q = v
        .get("q")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if kind == VerdictKind::Block && q.is_none() {
        return Err("verdict `block` requires `q`".to_string());
    }
    Ok(Verdict {
        verdict: kind,
        unit: v
            .get("unit")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        q,
        reason: v
            .get("reason")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// One escalation: prompt → call → parse; one re-ask on a malformed reply;
/// a second malformed reply ⇒ `abort` (safe default). Evidence + replies +
/// the verdict land in `logs_dir/escalation-<n>.json`.
pub fn escalate(
    home_dir: &Path,
    version: &str,
    argv_template: &[String],
    evidence: &Evidence,
    logs_dir: &Path,
    index: u32,
) -> Result<(Verdict, PathBuf), CoreError> {
    let prompt = build_prompt(home_dir, version, evidence)?;
    let mut replies: Vec<String> = Vec::new();

    let first = invoke(argv_template, &prompt)?;
    replies.push(first.clone());
    let verdict = match parse_verdict(&first) {
        Ok(v) => v,
        Err(err1) => {
            let reask = format!(
                "{prompt}\n\nYour previous reply could not be parsed ({err1}). Reply again with ONLY the single JSON verdict object as the last non-empty line."
            );
            let second = invoke(argv_template, &reask)?;
            replies.push(second.clone());
            match parse_verdict(&second) {
                Ok(v) => v,
                Err(err2) => Verdict {
                    verdict: VerdictKind::Abort,
                    unit: Some(evidence.unit.clone()),
                    q: None,
                    reason: format!("malformed escalation reply twice ({err2})"),
                },
            }
        }
    };

    std::fs::create_dir_all(logs_dir).map_err(|source| CoreError::Io {
        path: logs_dir.to_path_buf(),
        source,
    })?;
    let record = J::Obj(vec![
        ("unit".into(), J::s(evidence.unit.clone())),
        ("branch".into(), J::s(evidence.branch.clone())),
        ("fencing".into(), J::s(evidence.fencing.clone())),
        ("command".into(), J::s(evidence.command.clone())),
        (
            "output_tail".into(),
            J::s(tail_lines(&evidence.output_tail, 100)),
        ),
        (
            "journal_tail".into(),
            J::s(tail_lines(&evidence.journal_tail, 20)),
        ),
        (
            "replies".into(),
            J::Arr(replies.iter().map(|r| J::s(r.clone())).collect()),
        ),
        ("verdict".into(), J::s(verdict.verdict.as_str())),
        ("verdict_unit".into(), J::opt_str(verdict.unit.clone())),
        ("verdict_q".into(), J::opt_str(verdict.q.clone())),
        ("reason".into(), J::s(verdict.reason.clone())),
    ]);
    let path = logs_dir.join(format!("escalation-{index}.json"));
    std::fs::write(&path, record.dumps_indent2()).map_err(|source| CoreError::Io {
        path: path.clone(),
        source,
    })?;
    Ok((verdict, path))
}
