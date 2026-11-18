//! `vitrify probe` — probe this machine again and bring the probe record in
//! `~/.vitrify/config.toml` up to date. It prints what it found and what it
//! would change, and asks before it writes (`-y` answers for you); it
//! changes nothing else in the file. A home `vitrify init` has not set up is
//! refused.
//!
//! The probing is shared with `vitrify init`, which records its first probe
//! without asking: git, each harness CLI's one streaming headless spelling,
//! and each role that adds flags. Probes spawn in their own process group so
//! a hung probe is killable; absent harness CLIs are "untested", never an
//! error.

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use vitrify_core::CoreError;
use vitrify_core::harness::{self, Profiles, Role};

use crate::home::{self, Origin};
use crate::out::{J, diag, emit};

/// How every message names the file, whatever `VITRIFY_HOME` says.
pub const CONFIG_LABEL: &str = "~/.vitrify/config.toml";

/// The candidate spelling table: one streaming spelling per harness. Each
/// writes the session as it happens — events line by line — so a worker's
/// log holds everything up to the moment it stops, however it stops. A
/// harness that does not accept its spelling fails.
const HARNESSES: [(&str, &[&[&str]]); 3] = [
    (
        "claude",
        &[&[
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
        ]],
    ),
    ("codex", &[&["codex", "exec", "--json"]]),
    ("opencode", &[&["opencode", "run", "--format", "json"]]),
];

const PROBE_PROMPT: &str = "Reply with exactly: OK";

fn probe_timeout() -> Duration {
    let secs = std::env::var("VITRIFY_PROBE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60u64);
    Duration::from_secs(secs)
}

enum ProbeResult {
    /// exit status 0 within the timeout, non-empty output
    Ok,
    Failed,
    TimedOut,
    Missing,
}

/// Run argv + prompt in its own process group; SIGKILL the group on timeout.
fn probe(argv: &[&str], timeout: Duration) -> ProbeResult {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .arg(PROBE_PROMPT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProbeResult::Missing,
        Err(_) => return ProbeResult::Failed,
    };
    let pid = child.id();
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                return if status.success() && !out.trim().is_empty() {
                    ProbeResult::Ok
                } else {
                    ProbeResult::Failed
                };
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    crate::gate::kill_group(pid);
                    let _ = child.wait();
                    return ProbeResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return ProbeResult::Failed,
        }
    }
}

fn version_of(bin: &str) -> Option<String> {
    let out = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
}

pub(crate) struct HarnessReport {
    name: &'static str,
    present: bool,
    version: Option<String>,
    headless: Option<Vec<String>>,
    failure: Option<String>,
}

pub(crate) struct CheckLine {
    pub status: &'static str, // "ok" | "FAIL" | "untested" | "note" | "change"
    pub text: String,
}

impl CheckLine {
    pub fn new(status: &'static str, text: impl Into<String>) -> CheckLine {
        CheckLine {
            status,
            text: text.into(),
        }
    }
}

/// What one probe of the machine found: the lines to print, and the record
/// to keep in config.toml.
pub(crate) struct Probed {
    pub checks: Vec<CheckLine>,
    harnesses: Vec<HarnessReport>,
    git_present: bool,
    git_version: Option<String>,
}

impl Probed {
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == "FAIL")
    }
}

/// Probe git, each harness CLI, and each role of a working harness that adds
/// flags (its profile read from `config_text`); report the home's content.
/// The home must exist: `init` creates it first, `probe` refuses without it.
pub(crate) fn probe_machine(
    home_dir: &Path,
    version: &str,
    config_text: &str,
) -> Result<Probed, CoreError> {
    let timeout = probe_timeout();
    let mut checks: Vec<CheckLine> = Vec::new();

    // git
    let git_version = version_of("git");
    let git_present = git_version.is_some();
    checks.push(match &git_version {
        Some(v) => CheckLine::new("ok", format!("git present ({v})")),
        None => CheckLine::new("FAIL", "git not found on PATH"),
    });

    // harness probes
    let mut harnesses: Vec<HarnessReport> = Vec::new();
    for (name, spellings) in HARNESSES {
        let hv = version_of(name);
        if hv.is_none() && Command::new(name).arg("--help").output().is_err() {
            checks.push(CheckLine::new(
                "untested",
                format!("harness {name}: CLI not on PATH"),
            ));
            harnesses.push(HarnessReport {
                name,
                present: false,
                version: None,
                headless: None,
                failure: None,
            });
            continue;
        }
        let mut recorded: Option<Vec<String>> = None;
        let mut failure = None;
        for argv in spellings.iter() {
            match probe(argv, timeout) {
                ProbeResult::Ok => {
                    recorded = Some(argv.iter().map(|s| s.to_string()).collect());
                    break;
                }
                ProbeResult::Missing => {
                    failure = Some(format!("harness {name}: CLI not on PATH"));
                    break;
                }
                ProbeResult::TimedOut => {
                    failure = Some(format!(
                        "harness {name}: probe timed out after {}s ({})",
                        timeout.as_secs(),
                        argv.join(" ")
                    ));
                }
                ProbeResult::Failed => {
                    failure = Some(format!(
                        "harness {name}: headless probe failed — vitrify needs `{}`; update {name} until it accepts it",
                        argv.join(" ")
                    ));
                }
            }
        }
        match (&recorded, &failure) {
            (Some(argv), _) => checks.push(CheckLine::new(
                "ok",
                format!("harness {name}: headless = {}", argv.join(" ")),
            )),
            (None, Some(f)) if f.ends_with("not on PATH") => {
                checks.push(CheckLine::new("untested", f.clone()));
            }
            (None, Some(f)) => checks.push(CheckLine::new("FAIL", f.clone())),
            (None, None) => {}
        }
        let present =
            recorded.is_some() || !failure.as_deref().unwrap_or("").ends_with("not on PATH");
        harnesses.push(HarnessReport {
            name,
            present,
            version: hv,
            headless: recorded,
            failure,
        });
    }

    // the home's content: materialized, restored, refreshed — and overrides
    let states = home::ensure_materialized(home_dir, version)?;
    let overrides: Vec<_> = states
        .iter()
        .filter(|s| s.origin == Origin::Override)
        .collect();
    checks.push(CheckLine::new(
        "ok",
        format!(
            "~/.vitrify materialized ({} assets, {} overridden)",
            states.len(),
            overrides.len()
        ),
    ));
    for s in &overrides {
        checks.push(CheckLine::new(
            "note",
            if s.stale_against_embedded {
                format!(
                    "override: {} (forked from another shipped version — stale against embedded)",
                    s.rel_path
                )
            } else {
                format!("override: {}", s.rel_path)
            },
        ));
    }

    // per-role model/effort profiles for each working harness
    match harness::parse(config_text, CONFIG_LABEL) {
        Ok(profiles) => {
            // a role that adds flags gets its own real one-shot probe, so a
            // model the account cannot reach fails here, not mid-run
            for h in harnesses.iter() {
                let Some(recorded) = &h.headless else {
                    continue;
                };
                let profile = profiles.get(h.name);
                for role in Role::ALL {
                    if !profile.is_own(role) {
                        continue; // retries run as the worker does
                    }
                    let label = format!("harness {} {}", h.name, role.as_str());
                    let described = profile.describe(role);
                    if profile.role_args(role).is_empty() {
                        checks.push(CheckLine::new("ok", format!("{label}: {described}")));
                        continue;
                    }
                    let argv = profiles.argv_for(h.name, recorded, role);
                    let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
                    checks.push(match probe(&argv_ref, timeout) {
                        ProbeResult::Ok => CheckLine::new("ok", format!("{label}: {described}")),
                        ProbeResult::TimedOut => CheckLine::new(
                            "FAIL",
                            format!(
                                "{label}: probe timed out after {}s ({})",
                                timeout.as_secs(),
                                argv.join(" ")
                            ),
                        ),
                        ProbeResult::Failed | ProbeResult::Missing => CheckLine::new(
                            "FAIL",
                            format!(
                                "{label}: probe failed ({}) — fix [harness.{}.{}] in {CONFIG_LABEL}",
                                argv.join(" "),
                                h.name,
                                role.as_str()
                            ),
                        ),
                    });
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            checks.push(CheckLine::new(
                "FAIL",
                msg.strip_prefix("error: ").unwrap_or(&msg),
            ));
        }
    }

    Ok(Probed {
        checks,
        harnesses,
        git_present,
        git_version,
    })
}

// ------------------------------------------------------------ the record

/// A TOML basic string.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `[probe]` tables for what `p` found, as config.toml carries them.
pub(crate) fn render_record(version: &str, p: &Probed) -> String {
    let mut s = String::from("[probe]\n");
    s.push_str("# What `vitrify init` and `vitrify probe` found on this machine.\n");
    s.push_str("# Re-run `vitrify probe` rather than edit these tables.\n");
    s.push_str(&format!("vitrify_version = {}\n", toml_str(version)));
    s.push_str(&format!(
        "git = {{ present = {}, version = {} }}\n",
        p.git_present,
        toml_str(p.git_version.as_deref().unwrap_or(""))
    ));
    for h in &p.harnesses {
        s.push_str(&format!("\n[probe.{}]\n", h.name));
        s.push_str(&format!("present = {}\n", h.present));
        if let Some(v) = &h.version {
            s.push_str(&format!("version = {}\n", toml_str(v)));
        }
        if let Some(argv) = &h.headless {
            let quoted: Vec<String> = argv.iter().map(|a| toml_str(a)).collect();
            s.push_str(&format!("headless = [{}]\n", quoted.join(", ")));
        }
    }
    s
}

/// The key of a table header line (`[a.b]`, `[[a]]`), dotted and unquoted;
/// `None` for any other line.
fn header_key(line: &str) -> Option<String> {
    let t = line.trim_start();
    let inner = t.strip_prefix("[[").or_else(|| t.strip_prefix('['))?;
    let end = inner.find(']')?;
    Some(
        inner[..end]
            .split('.')
            .map(|part| part.trim().trim_matches('"').trim_matches('\''))
            .collect::<Vec<_>>()
            .join("."),
    )
}

fn is_probe_key(key: &str) -> bool {
    key == "probe" || key.starts_with("probe.")
}

/// `text` with every `[probe]` / `[probe.*]` table replaced by `block`, put
/// where the first one was — or at the end, when there is none. Every other
/// line is kept byte for byte, comments included.
pub(crate) fn replace_record(text: &str, block: &str) -> String {
    let mut out = String::new();
    let mut in_record = false;
    let mut placed = false;
    // the delimiter of the multi-line string a line is inside, if any
    let mut open: Option<&str> = None;
    for line in text.split_inclusive('\n') {
        let header = if open.is_none() {
            header_key(line)
        } else {
            None
        };
        for delim in ["\"\"\"", "'''"] {
            if (open.is_none() || open == Some(delim)) && line.matches(delim).count() % 2 == 1 {
                open = if open.is_some() { None } else { Some(delim) };
            }
        }
        if let Some(key) = header {
            if is_probe_key(&key) {
                if !placed {
                    out.push_str(block);
                    placed = true;
                }
                in_record = true;
                continue;
            }
            if in_record && !out.ends_with("\n\n") {
                out.push('\n');
            }
            in_record = false;
        }
        if !in_record {
            out.push_str(line);
        }
    }
    if !placed {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(block);
    }
    out
}

/// The probe record of a parsed config.toml, flattened to `probe.<path>` →
/// the value as a person reads it; empty when there is none.
fn flatten_record(table: &toml::Table) -> BTreeMap<String, String> {
    fn walk(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    walk(&format!("{prefix}.{k}"), v, out);
                }
            }
            toml::Value::String(s) => {
                out.insert(prefix.to_string(), s.clone());
            }
            toml::Value::Array(a) => {
                let parts: Vec<String> = a
                    .iter()
                    .map(|x| match x {
                        toml::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                out.insert(prefix.to_string(), parts.join(" "));
            }
            other => {
                out.insert(prefix.to_string(), other.to_string());
            }
        }
    }
    let mut out = BTreeMap::new();
    if let Some(record) = table.get("probe") {
        walk("probe", record, &mut out);
    }
    out
}

/// One difference between the recorded probe and the new one.
pub(crate) struct Change {
    pub key: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl Change {
    fn line(&self) -> String {
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "(none)".to_string());
        format!("{}: {} → {}", self.key, show(&self.from), show(&self.to))
    }
}

fn changes(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> Vec<Change> {
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| old.get(*k) != new.get(*k))
        .map(|k| Change {
            key: k.clone(),
            from: old.get(k).cloned(),
            to: new.get(k).cloned(),
        })
        .collect()
}

fn parse_config(text: &str) -> Result<toml::Table, CoreError> {
    text.parse::<toml::Table>().map_err(|e| {
        CoreError::refusal(
            2,
            format!(
                "error: unparseable {CONFIG_LABEL}: {e} → fix the line it names, or move the file aside and run vitrify init"
            ),
        )
    })
}

/// `text` with its probe record replaced by `block`, checked: everything
/// outside the record parses back unchanged, and the record is `block`.
pub(crate) fn updated_text(text: &str, block: &str) -> Result<String, CoreError> {
    let new_text = replace_record(text, block);
    let refuse = |why: &str| {
        CoreError::refusal(
            1,
            format!(
                "error: cannot update the [probe] tables of {CONFIG_LABEL} in place ({why}) → move them to the end of the file and run vitrify probe again"
            ),
        )
    };
    let mut before = parse_config(text)?;
    let mut after = new_text
        .parse::<toml::Table>()
        .map_err(|_| refuse("the result does not parse"))?;
    let expected = block
        .parse::<toml::Table>()
        .map_err(|_| refuse("the record does not parse"))?;
    if after.remove("probe") != expected.get("probe").cloned() {
        return Err(refuse("the record did not land whole"));
    }
    before.remove("probe");
    if before != after {
        return Err(refuse("a setting would change"));
    }
    Ok(new_text)
}

/// Write config.toml whole: a temporary file beside it, then a rename.
pub(crate) fn write_config(home_dir: &Path, text: &str) -> Result<(), CoreError> {
    let path = home::config_path(home_dir);
    let tmp = home_dir.join("config.toml.tmp");
    std::fs::write(&tmp, text).map_err(|source| CoreError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| CoreError::Io { path, source })
}

// ------------------------------------------------------------ the command

fn plural(n: usize) -> String {
    if n == 1 {
        "1 change".to_string()
    } else {
        format!("{n} changes")
    }
}

enum Answer {
    Yes,
    No,
    /// stdin closed before an answer: nobody is there to ask
    NoOne,
}

fn ask(n: usize) -> Answer {
    {
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "apply {} to {CONFIG_LABEL}? [y/N] ", plural(n));
        let _ = err.flush();
    }
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => {
            diag("");
            Answer::NoOne
        }
        Ok(_) => match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Answer::Yes,
            _ => Answer::No,
        },
    }
}

/// The JSON shared by `init --json` and `probe --json`.
pub(crate) fn json_of(checks: &[CheckLine], p: &Probed, failed: bool) -> Vec<(String, J)> {
    let jchecks: Vec<J> = checks
        .iter()
        .map(|c| {
            J::Obj(vec![
                ("status".into(), J::s(c.status)),
                ("text".into(), J::s(c.text.clone())),
            ])
        })
        .collect();
    let jh: Vec<J> = p
        .harnesses
        .iter()
        .map(|h| {
            J::Obj(vec![
                ("name".into(), J::s(h.name)),
                ("present".into(), J::Bool(h.present)),
                ("version".into(), J::opt_str(h.version.clone())),
                (
                    "headless".into(),
                    match &h.headless {
                        Some(argv) => J::Arr(argv.iter().map(|a| J::s(a.clone())).collect()),
                        None => J::Null,
                    },
                ),
                ("failure".into(), J::opt_str(h.failure.clone())),
            ])
        })
        .collect();
    vec![
        ("green".into(), J::Bool(!failed)),
        ("checks".into(), J::Arr(jchecks)),
        ("harnesses".into(), J::Arr(jh)),
    ]
}

pub fn run(json: bool, assume_yes: bool) -> Result<u8, CoreError> {
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    home::require_set_up(&home_dir)?;
    let path = home::config_path(&home_dir);
    let text = vitrify_core::model::read_text(&path)?;
    let table = parse_config(&text)?;
    let version = super::brief::VERSION;

    let mut checks = vec![CheckLine::new("ok", format!("vitrify version {version}"))];
    let probed = probe_machine(&home_dir, version, &text)?;
    let failed = probed.failed();
    let block = render_record(version, &probed);
    let new_text = updated_text(&text, &block)?;
    let found = changes(
        &flatten_record(&table),
        &flatten_record(&parse_config(&new_text)?),
    );
    checks.extend(
        probed
            .checks
            .iter()
            .map(|c| CheckLine::new(c.status, c.text.clone())),
    );
    if !json {
        for c in &checks {
            emit(&format!("{}: {}", c.status, c.text));
        }
    }

    let mut result: Vec<CheckLine> = found
        .iter()
        .map(|c| CheckLine::new("change", c.line()))
        .collect();
    if !json {
        for c in &result {
            emit(&format!("{}: {}", c.status, c.text));
        }
    }
    let applied = if found.is_empty() {
        result.push(CheckLine::new("ok", format!("{CONFIG_LABEL}: no changes")));
        false
    } else {
        if json && !assume_yes {
            // the changes go where the question goes
            for c in &result {
                diag(&format!("{}: {}", c.status, c.text));
            }
        }
        let answer = if assume_yes {
            Answer::Yes
        } else {
            ask(found.len())
        };
        match answer {
            Answer::Yes => {
                write_config(&home_dir, &new_text)?;
                result.push(CheckLine::new(
                    "ok",
                    format!("updated {CONFIG_LABEL} ({})", plural(found.len())),
                ));
                true
            }
            Answer::No => {
                result.push(CheckLine::new(
                    "note",
                    format!("{CONFIG_LABEL}: nothing changed"),
                ));
                false
            }
            Answer::NoOne => {
                result.push(CheckLine::new(
                    "note",
                    format!(
                        "{CONFIG_LABEL}: nothing changed — no answer on stdin; vitrify probe -y applies without asking"
                    ),
                ));
                false
            }
        }
    };
    let declined = !found.is_empty() && !applied;

    if json {
        let all: Vec<CheckLine> = checks
            .into_iter()
            .chain(result.into_iter().filter(|c| c.status != "change"))
            .collect();
        let mut obj = json_of(&all, &probed, failed);
        obj.push((
            "changes".into(),
            J::Arr(
                found
                    .iter()
                    .map(|c| {
                        J::Obj(vec![
                            ("key".into(), J::s(c.key.clone())),
                            ("from".into(), J::opt_str(c.from.clone())),
                            ("to".into(), J::opt_str(c.to.clone())),
                        ])
                    })
                    .collect(),
            ),
        ));
        obj.push(("applied".into(), J::Bool(applied)));
        emit(&J::Obj(obj).dumps_indent2());
    } else {
        for c in result.iter().filter(|c| c.status != "change") {
            emit(&format!("{}: {}", c.status, c.text));
        }
    }
    Ok(if failed || declined { 2 } else { 0 })
}

/// The headless spelling the probe recorded for one harness; the
/// orchestrator uses ONLY recorded spellings.
pub fn recorded_headless(home_dir: &Path, harness: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(home::config_path(home_dir)).ok()?;
    let table: toml::Table = text.parse().ok()?;
    let h = table.get("probe")?.as_table()?.get(harness)?.as_table()?;
    let argv = h.get("headless")?.as_array()?;
    Some(
        argv.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// The role profiles in force for a run: `~/.vitrify/config.toml`, with a
/// repository's `.vitrify/config.toml` overriding it key by key.
pub fn role_profiles(home_dir: &Path, repo_root: Option<&Path>) -> Result<Profiles, CoreError> {
    let layers = vitrify_core::config::layers(repo_root, Some(home_dir))?;
    harness::from_layers(&layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "[probe]\nvitrify_version = \"1\"\n\n[probe.claude]\npresent = true\n";

    #[test]
    fn the_record_is_appended_when_there_is_none() {
        let text = "schema = 1\n\n[run]\nworker_cap = 4 # mine\n";
        let out = replace_record(text, BLOCK);
        assert_eq!(out, format!("{text}\n{BLOCK}"));
        assert_eq!(updated_text(text, BLOCK).unwrap(), out);
    }

    #[test]
    fn only_the_record_is_replaced_in_place() {
        let text = "# top\nschema = 1\n\n[run]\nworker_cap = 2 # mine\n\n[probe]\nvitrify_version = \"0\"\n\n[probe.codex]\npresent = false\n\n[lint]\n# keep\nreadset_budget_bytes = 9\n";
        let out = updated_text(text, BLOCK).unwrap();
        assert_eq!(
            out,
            "# top\nschema = 1\n\n[run]\nworker_cap = 2 # mine\n\n[probe]\nvitrify_version = \"1\"\n\n[probe.claude]\npresent = true\n\n[lint]\n# keep\nreadset_budget_bytes = 9\n"
        );
    }

    #[test]
    fn a_header_inside_a_multi_line_string_is_not_a_table() {
        let text = "note = \"\"\"\n[probe]\n\"\"\"\n[run]\nworker_cap = 1\n";
        let out = updated_text(text, BLOCK).unwrap();
        assert!(out.starts_with(text), "{out}");
    }

    #[test]
    fn changes_name_what_moved() {
        let old: toml::Table = "[probe.claude]\npresent = true\nversion = \"1\"\n"
            .parse()
            .unwrap();
        let new: toml::Table =
            "[probe.claude]\npresent = true\nversion = \"2\"\nheadless = [\"claude\", \"-p\"]\n"
                .parse()
                .unwrap();
        let lines: Vec<String> = changes(&flatten_record(&old), &flatten_record(&new))
            .iter()
            .map(Change::line)
            .collect();
        assert_eq!(
            lines,
            [
                "probe.claude.headless: (none) → claude -p",
                "probe.claude.version: 1 → 2"
            ]
        );
        assert!(changes(&flatten_record(&new), &flatten_record(&new)).is_empty());
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
