//! The seven audit checks — finding sets held by the regression goldens
//! (`fixtures/golden/`, `fixtures/seeded/`). Check 4 (rotten Verified) re-runs Verification commands
//! through an injected [`CommandRunner`]; the real `sh -c` runner lives in
//! the CLI (the one sanctioned shell use). State edits for
//! `--fix-leases` / `--apply-stale` are returned for the caller to apply.
//!
//! The audit reads spec files and file paths only — never source code, so
//! nothing here depends on a project's language or ecosystem.

use std::path::{Path, PathBuf};

use crate::CoreError;
use crate::journal;
use crate::model::{self, Instance, Shape, Unit, UtcTime};
use crate::trace;

/// Injected command execution for check 4; returns the exit code.
pub trait CommandRunner {
    fn run(&self, cmd: &str, cwd: &Path) -> i32;
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub skip_rotten: bool,
    pub fix_leases: bool,
    pub apply_stale: bool,
    /// The governed files under `scope_roots` — root-relative, what git
    /// would commit (tracked, or untracked and not ignored) — supplied by a
    /// caller that has git. `None` walks the filesystem instead
    /// ([`walk_scope`]).
    pub scope_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: u8,
    pub subject: String,
    pub cause: String,
    pub remediation: String,
}

fn finding(check: u8, subject: impl Into<String>, cause: impl Into<String>, rem: &str) -> Finding {
    Finding {
        check,
        subject: subject.into(),
        cause: cause.into(),
        remediation: rem.to_string(),
    }
}

/// A state edit the caller applies (current worktree only; caller commits).
#[derive(Debug, Clone)]
pub enum StateEdit {
    /// `--fix-leases`: release-formalize a lapsed lease (claim_seq kept).
    FixLease { unit: String, claim_seq: i64 },
    /// `--apply-stale`: mark Stale with the finding's cause (≤ 200 chars).
    ApplyStale { unit: String, cause: String },
}

/// The filesystem walk's skip set — git's and vitrify's own directories,
/// nothing ecosystem-specific — tested on the root-relative path only (the
/// worktree-prefix rule).
const SKIP: [&str; 2] = [".git", ".vitrify"];

// ----------------------------------------------------------------- fnmatch

/// Shell-style glob match: `*`, `?`, `[seq]`, `[!seq]`;
/// `*` crosses directory separators.
pub fn fnmatch(name: &str, pat: &str) -> bool {
    fn match_at(n: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return n.is_empty();
        }
        match p[0] {
            '*' => {
                for i in 0..=n.len() {
                    if match_at(&n[i..], &p[1..]) {
                        return true;
                    }
                }
                false
            }
            '?' => !n.is_empty() && match_at(&n[1..], &p[1..]),
            '[' => {
                if n.is_empty() {
                    return false;
                }
                let close = match p.iter().skip(1).position(|&c| c == ']') {
                    Some(i) => i + 1,
                    None => return n[0] == '[' && match_at(&n[1..], &p[1..]),
                };
                let (set, negate) = if p[1] == '!' {
                    (&p[2..close], true)
                } else {
                    (&p[1..close], false)
                };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        if set[i] <= n[0] && n[0] <= set[i + 2] {
                            hit = true;
                        }
                        i += 3;
                    } else {
                        if set[i] == n[0] {
                            hit = true;
                        }
                        i += 1;
                    }
                }
                if hit != negate {
                    match_at(&n[1..], &p[close + 1..])
                } else {
                    false
                }
            }
            c => !n.is_empty() && n[0] == c && match_at(&n[1..], &p[1..]),
        }
    }
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    match_at(&n, &p)
}

/// The ownership match: exact path, glob, or directory-prefix.
pub fn matches(path: &str, glob: &str) -> bool {
    let g = glob.trim_end_matches('/');
    path == g || fnmatch(path, g) || path.starts_with(&format!("{g}/"))
}

// ----------------------------------------------------- scope + unit text

fn owned_globs(units: &[Unit]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for u in units {
        let globs: Vec<String> = u
            .meta
            .get_list("owns")
            .iter()
            .map(|g| g.trim_end().to_string())
            .collect();
        if !globs.is_empty() {
            out.push((u.unit.clone(), globs));
        }
    }
    out
}

/// The `unmanaged:` globs of an SDD's core text (§6).
pub fn unmanaged_globs(core_text: &str) -> Vec<String> {
    layout_list(core_text, "unmanaged:")
}

/// The `tests:` globs of an SDD's core text (§6, schema 2.2).
pub fn tests_globs(core_text: &str) -> Vec<String> {
    layout_list(core_text, "tests:")
}

/// Whether a `tests:` entry lies inside `scope_roots` (lint L2): a scope
/// root is the repository root (`.` or empty), or one matches the entry's
/// literal prefix — the text before its first `*`, `?` or `[`, the whole
/// entry when it has none — with trailing `/` removed.
pub fn tests_entry_in_scope(entry: &str, scope_roots: &[String]) -> bool {
    let entry = entry.trim();
    let literal = match entry.find(['*', '?', '[']) {
        Some(i) => &entry[..i],
        None => entry,
    };
    let prefix = literal.trim_end_matches('/');
    scope_roots.iter().any(|r| {
        let root = r.trim().trim_start_matches("./");
        root.is_empty() || root == "." || matches(prefix, root)
    })
}

/// The items of the list `key` (`unmanaged:`, `tests:`, …) in an SDD's
/// `System Layout & Ownership` section: the line whose trimmed text is
/// `key`, then each following indented `- item` line, until the first other
/// line. The list may sit inside a fenced block.
fn layout_list(core_text: &str, key: &str) -> Vec<String> {
    let sec = model::section(core_text, "System Layout & Ownership");
    let mut out = Vec::new();
    let mut in_block = false;
    for line in crate::newline::split_universal(sec) {
        if !in_block {
            if line.trim_end() == key || line.trim() == key {
                in_block = true;
            }
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(item) = trimmed.strip_prefix("- ")
            && line.starts_with(|c: char| c.is_whitespace())
        {
            out.push(item.trim().to_string());
        } else {
            break;
        }
    }
    out
}

/// The governed files under `scope_roots` when no git listing is available
/// (outside a repository): every file on disk, root-relative, except under
/// `.git` and `.vitrify`; per scope root in order, sorted.
pub fn walk_scope(root: &Path, scope_roots: &[String]) -> Vec<String> {
    let mut files = Vec::new();
    for sr in scope_roots {
        let base = root.join(sr);
        if !base.exists() {
            continue;
        }
        let mut found: Vec<PathBuf> = Vec::new();
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file() {
                    found.push(p);
                }
            }
        }
        found.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
        for p in found {
            let Ok(rel) = p.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            // the worktree-prefix rule: skip set on the root-relative path only
            if rel
                .components()
                .any(|c| c.as_os_str().to_str().is_some_and(|s| SKIP.contains(&s)))
            {
                continue;
            }
            files.push(rel_str);
        }
    }
    files
}

/// Verification-table commands of a unit's entry text:
/// rows of the `Verification` section's table, second column, backticks
/// stripped.
pub fn verification_commands(unit_text: &str) -> Vec<String> {
    // section: ^#{2,4}\s+Verification\s*$ … until ^#{2,3}\s or EOF
    let lines = crate::newline::split_universal(unit_text);
    let mut cmds = Vec::new();
    let mut i = 0;
    let n = lines.len();
    let is_verif_heading = |line: &str| {
        for hashes in ["####", "###", "##"] {
            if let Some(rest) = line.strip_prefix(hashes)
                && rest.starts_with(|c: char| c.is_whitespace())
                && rest.trim() == "Verification"
            {
                return true;
            }
        }
        false
    };
    let is_end_heading = |line: &str| {
        for hashes in ["###", "##"] {
            if let Some(rest) = line.strip_prefix(hashes)
                && !rest.starts_with('#')
                && rest.starts_with(|c: char| c.is_whitespace())
            {
                return true;
            }
        }
        false
    };
    while i < n {
        if is_verif_heading(lines[i]) {
            i += 1;
            while i < n && !is_end_heading(lines[i]) {
                let ln = lines[i];
                if ln.trim_start().starts_with('|') {
                    let stripped = ln.trim().trim_matches('|');
                    let cells: Vec<&str> = stripped.split('|').map(str::trim).collect();
                    if cells.len() >= 2 {
                        let first = cells[0];
                        let is_sep = first.chars().all(|c| c == '-' || c == ' ' || c == ':');
                        if !is_sep && first.to_lowercase() != "covers" {
                            let cmd = cells[1].trim().trim_matches('`').trim();
                            if !cmd.is_empty() {
                                cmds.push(cmd.to_string());
                            }
                        }
                    }
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    cmds
}

fn unit_text(inst: &Instance, u: &Unit) -> Result<String, CoreError> {
    match inst.shape {
        Shape::Bundle => model::read_text(&u.file),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            Ok(model::find_unit_span(&text, &u.unit)
                .map(|(s, e)| text[s..e].to_string())
                .unwrap_or_default())
        }
    }
}

/// Producer/consumer references parsed from a contract's text.
pub struct ContractRef {
    pub producer: Option<String>,
    pub consumers: Vec<String>,
}

fn find_unit_ids(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'U' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 2 {
                out.push(line[i..j].to_string());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_contract_ref(text: &str) -> ContractRef {
    let mut producer = None;
    let mut consumers = Vec::new();
    // Producer: first `Producer:\s*(U-\d+)` match anywhere
    if let Some(i) = text.find("Producer:") {
        let rest = text[i + 9..].trim_start();
        let ids = find_unit_ids(rest.lines().next().unwrap_or(""));
        if let Some(first) = ids.first()
            && rest.starts_with(first.as_str())
        {
            producer = Some(first.clone());
        }
    }
    // Consumers: first line containing `Consumers:`; all U-\d+ on it
    for line in text.lines() {
        if let Some(i) = line.find("Consumers:") {
            consumers = find_unit_ids(&line[i + 10..]);
            break;
        }
    }
    ContractRef {
        producer,
        consumers,
    }
}

/// Parse every contract's producer/consumer references (shared with lint L4).
pub fn contract_refs(inst: &Instance) -> Result<Vec<(String, ContractRef)>, CoreError> {
    let mut out = Vec::new();
    match inst.shape {
        Shape::Bundle => {
            let cdir = inst.root.join("contracts");
            let mut files: Vec<PathBuf> = std::fs::read_dir(&cdir)
                .map(|rd| rd.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            files.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
            for p in files {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with("C-") || !name.ends_with(".md") {
                    continue;
                }
                let digits: String = name[2..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if digits.is_empty() {
                    continue;
                }
                let cid = format!("C-{digits}");
                let text = model::read_text(&p)?;
                out.push((cid, parse_contract_ref(&text)));
            }
        }
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            for (ident, region) in model::split_h3_regions(&text, "Interface Contracts") {
                if ident.starts_with("C-") && ident[2..].starts_with(|c: char| c.is_ascii_digit()) {
                    out.push((ident, parse_contract_ref(region)));
                }
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------- run

/// The SDD at `sdd_root` as a `--sdd` value from the checkout root `root`.
fn sdd_arg(root: &Path, sdd_root: &Path) -> String {
    let rel = sdd_root
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let (root, sdd_root) = (root.canonicalize().ok()?, sdd_root.canonicalize().ok()?);
            sdd_root.strip_prefix(&root).ok().map(Path::to_path_buf)
        });
    match rel {
        Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => sdd_root.display().to_string(),
    }
}

/// Run the seven checks. Returns findings plus the state edits implied by
/// `fix_leases` / `apply_stale` (the caller applies and commits them).
pub fn run(
    inst: &Instance,
    root: &Path,
    opts: &Options,
    runner: &dyn CommandRunner,
    now: UtcTime,
) -> Result<(Vec<Finding>, Vec<StateEdit>), CoreError> {
    let units = &inst.units;
    let core_text = model::read_text(&inst.file)?;
    let scope_roots = inst.meta.get_list("scope_roots");
    let mut findings: Vec<Finding> = Vec::new();
    let sdd = sdd_arg(root, &inst.root);

    // 1. Orphans
    let owns = owned_globs(units);
    let unmanaged = unmanaged_globs(&core_text);
    let tests = if inst.schema().tests_list() {
        tests_globs(&core_text)
    } else {
        Vec::new()
    };
    let all_globs: Vec<&String> = owns
        .iter()
        .flat_map(|(_, gs)| gs.iter())
        .chain(unmanaged.iter())
        .chain(tests.iter())
        .collect();
    let scope_files = match &opts.scope_files {
        Some(files) => files.clone(),
        None => walk_scope(root, &scope_roots),
    };
    for f in &scope_files {
        if !all_globs.iter().any(|g| matches(f, g)) {
            findings.push(finding(
                1,
                f.clone(),
                "file under scope_roots is neither owned nor unmanaged (§6)",
                "give it an owner in a unit's `owns`, or list it `unmanaged` (§6), under a change entry (§12)",
            ));
        }
    }

    // 2. Ghosts
    for u in units {
        let status = u.meta.get_str_or("status", "Draft");
        if status == "Built" || status == "Verified" {
            for (uid, gs) in &owns {
                if uid != &u.unit {
                    continue;
                }
                for g in gs {
                    let hits = scope_files.iter().any(|f| matches(f, g));
                    if !hits && !root.join(g).exists() {
                        findings.push(finding(
                            2,
                            u.unit.clone(),
                            format!("owned path matches no file: {g}"),
                            "restore the artifact, or mark the unit Stale or Retired under a change entry (§12)",
                        ));
                    }
                }
            }
        }
    }

    // 3. Contract references — spec only: every consumed contract exists and
    // every contract names units of this SDD. Whether code crosses a unit
    // boundary outside a contract is a language question; vitrify never
    // reads source code (an SDD checks it with its own boundary checker).
    let crefs = contract_refs(inst)?;
    let cref_ids: Vec<&String> = crefs.iter().map(|(c, _)| c).collect();
    for u in units {
        for cid in u.meta.get_list("consumes") {
            if !cref_ids.iter().any(|c| **c == cid) {
                findings.push(finding(
                    3,
                    u.unit.clone(),
                    format!("consumes nonexistent contract {cid}"),
                    "add the contract (§7), or fix the unit's `consumes`, under a change entry (§12)",
                ));
            }
        }
    }
    let unit_ids: Vec<&String> = units.iter().map(|u| &u.unit).collect();
    for (cid, cref) in &crefs {
        let mut refs: Vec<&String> = Vec::new();
        if let Some(p) = &cref.producer {
            refs.push(p);
        }
        refs.extend(cref.consumers.iter());
        for uid in refs {
            if !unit_ids.contains(&uid) {
                findings.push(finding(
                    3,
                    cid.clone(),
                    format!("references nonexistent unit {uid}"),
                    "fix the contract's Producer and Consumers lines (§7) under a change entry (§12)",
                ));
            }
        }
    }
    // 4. Rotten Verified
    if !opts.skip_rotten {
        for u in units {
            if u.meta.get_str_or("status", "Draft") == "Verified" {
                for cmd in verification_commands(&unit_text(inst, u)?) {
                    let code = runner.run(&cmd, root);
                    if code != 0 {
                        findings.push(finding(
                            4,
                            u.unit.clone(),
                            format!("Verification command failing: {cmd} (exit {code})"),
                            &format!("mark it Stale for a rebuild: vitrify audit --apply-stale --sdd {sdd}"),
                        ));
                    }
                }
            }
        }
    }

    // 5. Trace mismatch
    for u in units {
        let status = u.meta.get_str_or("status", "Draft");
        if status != "Built" && status != "Verified" {
            continue;
        }
        match u.meta.get_map("trace") {
            Some(recorded) if !recorded.is_empty() => {
                let (current, _) = trace::compute(inst, u)?;
                let mismatches = trace::verify(recorded, &current);
                if !mismatches.is_empty() {
                    let keys: Vec<&str> = mismatches.iter().map(|m| m.key.as_str()).collect();
                    findings.push(finding(
                        5,
                        u.unit.clone(),
                        format!(
                            "recorded trace differs from current content: {}",
                            keys.join(", ")
                        ),
                        &format!(
                            "stale because {} changed: write the change entry (§12), then vitrify change apply --sdd {sdd}",
                            keys[0]
                        ),
                    ));
                }
            }
            _ => findings.push(finding(
                5,
                u.unit.clone(),
                "Built/Verified with no recorded trace",
                &format!("mark it Stale for a rebuild: vitrify audit --apply-stale --sdd {sdd}"),
            )),
        }
    }

    // 6. Projection consistency
    let handle = inst.handle();
    let (events, exists) = journal::read(&handle)?;
    if exists && !events.is_empty() {
        let proj = journal::replay(&events);
        let mismatches = journal::verify(&proj, units);
        for mm in &mismatches {
            findings.push(finding(
                6,
                mm.unit.clone(),
                format!(
                    "journal projects {}={} but front matter says {}",
                    mm.field,
                    mm.projected.repr(),
                    mm.current.repr()
                ),
                &format!(
                    "restore the front matter the journal projects (vitrify journal replay --sdd {sdd}); never edit the journal"
                ),
            ));
        }
        for of in &proj.order_findings {
            findings.push(finding(
                6,
                "journal",
                of.clone(),
                "the journal is append-only and ordered (§11): restore it from version control",
            ));
        }
    } else if units
        .iter()
        .any(|u| !matches!(u.meta.get_str_or("status", "Draft"), "Draft" | "Ready"))
    {
        findings.push(finding(
            6,
            "journal",
            "units beyond Ready but no journal exists",
            "every state transition appends a journal event (§11): return the units to Ready, or record their events with vitrify journal append",
        ));
    }
    // Stale-cause cross-check
    let mut declared_stale: Vec<String> = Vec::new();
    for e in &events {
        if e.event == "stale"
            && let Some(u) = &e.unit
        {
            declared_stale.push(u.clone());
        }
        if e.event == "change" {
            declared_stale.extend(e.stale.iter().flatten().cloned());
        }
    }
    for u in units {
        if u.meta.get_str_or("status", "Draft") == "Stale" && !declared_stale.contains(&u.unit) {
            // a current trace mismatch (or no recorded trace) justifies Stale
            let justified = match u.meta.get_map("trace") {
                Some(recorded) if !recorded.is_empty() => {
                    let (current, _) = trace::compute(inst, u)?;
                    !trace::verify(recorded, &current).is_empty()
                }
                _ => true, // no trace recorded → verify exits 4, not 0
            };
            if !justified {
                findings.push(finding(
                    6,
                    u.unit.clone(),
                    "Stale with no declaring change entry and no trace mismatch",
                    "undo the hand edit of its status: every Stale mark traces to a change entry or a trace mismatch (§11)",
                ));
            }
        }
    }

    // 7. Lapsed leases
    let mut edits: Vec<StateEdit> = Vec::new();
    let mut stale_marks: Vec<(String, String)> = Vec::new();
    for u in units {
        let v = model::unit_view(u, units, now);
        if v.status == "Claimed" && !v.lease_live {
            findings.push(finding(
                7,
                u.unit.clone(),
                format!(
                    "lease expired at {} (holder {})",
                    v.claim_expires.as_deref().unwrap_or("None"),
                    v.claimed_by.as_deref().unwrap_or("None")
                ),
                &format!(
                    "claimable again; vitrify audit --fix-leases --sdd {sdd} formalizes it (release, claim_seq kept)"
                ),
            ));
            if opts.fix_leases {
                edits.push(StateEdit::FixLease {
                    unit: u.unit.clone(),
                    claim_seq: v.claim_seq,
                });
            }
        }
    }
    if opts.apply_stale {
        for f in &findings {
            if (f.check == 4 || f.check == 5)
                && unit_ids.contains(&&f.subject)
                && !stale_marks.iter().any(|(u, _)| u == &f.subject)
            {
                let cause: String = f.cause.chars().take(200).collect();
                stale_marks.push((f.subject.clone(), cause));
            }
        }
        stale_marks.sort();
        for (unit, cause) in stale_marks {
            edits.push(StateEdit::ApplyStale { unit, cause });
        }
    }

    Ok((findings, edits))
}
