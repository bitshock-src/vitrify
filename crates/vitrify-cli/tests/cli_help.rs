//! The help conformance test: every command documents itself, every
//! `vitrify …` line in the briefs, schemas and docs parses against the
//! command tree, and the root help lists every command once. The help
//! texts are pinned by insta snapshots.
//!
//! Command-line extraction from markdown, the rules in full:
//! 1. Candidates are inline code spans and fenced-code lines that start
//!    with `vitrify ` — also after a leading `$ ` or `nohup `.
//! 2. A line whose second word starts with a digit is an output sample
//!    (`vitrify 0.5.0 procedures — …`) and is skipped.
//! 3. `[…]` optional groups are dropped (nested brackets included);
//!    `…` and `...` are dropped.
//! 4. The command ends at a shell operator (`|`, `||`, `>`, `>>`, `2>`,
//!    `&&`, `;`, `&`, `<` redirect), a `#` comment, or a prose marker
//!    (`·`, `→`, `—`, `(`); a trailing `,` `.` `:` `)` is trimmed.
//! 5. `a|b` alternatives inside one word take the first.
//! 6. `<placeholder>`, `{placeholder}` and an all-caps word (`KIND`,
//!    `N`) become a dummy value: the first possible value where the
//!    argument has a fixed set, else `1` (valid as text and as a number).
//!    A placeholder in the command position (`vitrify <command> --help`)
//!    stands for every command, so the line is skipped.
//! 7. `--help`, `-h` and `--version` parse as success, and so does a
//!    command named without its required arguments (`vitrify claim` in
//!    prose): what must exist is every command and flag the line names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::{Command, CommandFactory};
use vitrify_cli::cli::Cli;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn root() -> Command {
    let mut cmd = Cli::command();
    cmd.build();
    cmd
}

/// Every command path below the root (`["pin", "sync"]`), depth first,
/// clap's generated `help` subcommand apart.
fn command_paths(cmd: &Command) -> Vec<Vec<String>> {
    fn walk(cmd: &Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
        for sub in cmd.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            let mut path = prefix.to_vec();
            path.push(sub.get_name().to_string());
            out.push(path.clone());
            walk(sub, &path, out);
        }
    }
    let mut out = Vec::new();
    walk(cmd, &[], &mut out);
    out
}

fn find<'a>(cmd: &'a Command, path: &[String]) -> &'a Command {
    path.iter().fold(cmd, |c, name| {
        c.find_subcommand(name)
            .unwrap_or_else(|| panic!("no subcommand {name}"))
    })
}

/// Paths that run something: no subcommands, or subcommands optional
/// (`run`, beside `run status`).
fn runnable_paths(cmd: &Command) -> Vec<String> {
    command_paths(cmd)
        .into_iter()
        .filter(|p| {
            let c = find(cmd, p);
            c.get_subcommands().all(|s| s.get_name() == "help") || !c.is_subcommand_required_set()
        })
        .map(|p| p.join(" "))
        .collect()
}

fn long_help(cmd: &Command, path: &[String]) -> String {
    find(cmd, path).clone().render_long_help().to_string()
}

// ---------------------------------------------------------------- tree

#[test]
fn every_command_has_long_help_with_examples_and_exit_status() {
    let cmd = root();
    let mut failures = Vec::new();
    for path in command_paths(&cmd) {
        let sub = find(&cmd, &path);
        let name = path.join(" ");
        let block = sub
            .get_after_long_help()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if block.trim().is_empty() {
            failures.push(format!("vitrify {name}: no long help block"));
            continue;
        }
        for heading in ["Exit status:", "Examples:"] {
            if !block.lines().any(|l| l.starts_with(heading)) {
                failures.push(format!("vitrify {name}: no `{heading}` block"));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn every_argument_has_help() {
    let cmd = root();
    let mut failures = Vec::new();
    let mut paths = vec![Vec::new()];
    paths.extend(command_paths(&cmd));
    for path in paths {
        let sub = find(&cmd, &path);
        for arg in sub.get_arguments() {
            let id = arg.get_id().as_str();
            if id == "help" || id == "version" {
                continue;
            }
            let has = arg
                .get_help()
                .is_some_and(|h| !h.to_string().trim().is_empty())
                || arg
                    .get_long_help()
                    .is_some_and(|h| !h.to_string().trim().is_empty());
            if !has {
                failures.push(format!(
                    "vitrify {}: argument `{id}` has no help",
                    path.join(" ")
                ));
            }
            for pv in arg.get_possible_values() {
                if pv.is_hide_set() {
                    continue;
                }
                if pv.get_help().is_none() {
                    failures.push(format!(
                        "vitrify {}: value `{}` of `{id}` has no help",
                        path.join(" "),
                        pv.get_name()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

// ------------------------------------------------------------ root help

/// The command paths named in the long root help's "Commands by task"
/// section: indented lines whose first column is a command path.
fn listed_long(help: &str) -> Vec<String> {
    let section = help
        .split("Commands by task:")
        .nth(1)
        .expect("a Commands by task section")
        .split("\nConventions:")
        .next()
        .unwrap();
    section
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .filter_map(|l| l.trim().split("  ").next())
        .map(|s| s.trim().to_string())
        .collect()
}

/// The command paths named in the short root help's "Commands, by task"
/// section: the `·`-separated entries after each group label.
fn listed_short(help: &str) -> Vec<String> {
    let section = help
        .split("Commands, by task:")
        .nth(1)
        .expect("a Commands, by task section")
        .split("\n\n")
        .next()
        .unwrap();
    let mut out = Vec::new();
    for line in section.lines() {
        // The entries start after the label column (two or more spaces).
        let trimmed = line.trim_start();
        let entries = match trimmed.find("   ") {
            Some(i) if line.len() - trimmed.len() <= 2 => &trimmed[i..],
            _ => trimmed,
        };
        out.extend(
            entries
                .split('·')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
    }
    out
}

fn assert_lists_every_command_once(which: &str, listed: Vec<String>, runnable: &[String]) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &listed {
        *counts.entry(entry.clone()).or_default() += 1;
    }
    let mut failures = Vec::new();
    for (entry, n) in &counts {
        if *n > 1 {
            failures.push(format!("{which}: `{entry}` listed {n} times"));
        }
        if !runnable.contains(entry) {
            failures.push(format!("{which}: `{entry}` is not a command"));
        }
    }
    for path in runnable {
        if !counts.contains_key(path) {
            failures.push(format!("{which}: `{path}` is not listed"));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn root_help_lists_every_command_once() {
    let cmd = root();
    let runnable = runnable_paths(&cmd);
    let long = cmd.clone().render_long_help().to_string();
    let short = cmd.clone().render_help().to_string();
    assert_lists_every_command_once("vitrify --help", listed_long(&long), &runnable);
    assert_lists_every_command_once("vitrify -h", listed_short(&short), &runnable);
}

#[test]
fn root_short_and_long_help_differ() {
    let cmd = root();
    let long = cmd.clone().render_long_help().to_string();
    let short = cmd.clone().render_help().to_string();
    assert_ne!(short, long);
    assert!(short.contains("Run 'vitrify --help'"));
    for part in [
        "Start here",
        "Commands by task:",
        "Conventions:",
        "Exit status:",
        env!("CARGO_PKG_VERSION"),
        "https://github.com/bitshock-src/vitrify",
    ] {
        assert!(long.contains(part), "vitrify --help lacks {part:?}");
    }
    for code in 0..=6 {
        assert!(
            long.lines().any(|l| l.starts_with(&format!("  {code}  "))),
            "vitrify --help lacks exit status {code}"
        );
    }
}

// ------------------------------------------------- command-line extraction

/// A `vitrify …` command found in a file: where, and its text as written.
struct Found {
    at: String,
    text: String,
}

fn strip_prefix(s: &str) -> Option<&str> {
    let s = s.trim();
    let s = s.strip_prefix("$ ").unwrap_or(s).trim_start();
    let s = s.strip_prefix("nohup ").unwrap_or(s).trim_start();
    s.starts_with("vitrify ").then_some(s)
}

/// The candidate command lines of one markdown file (rule 1).
fn extract_markdown(rel: &str, body: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in body.lines().enumerate() {
        let at = format!("{rel}:{}", i + 1);
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            if let Some(cmd) = strip_prefix(line) {
                out.push(Found {
                    at,
                    text: cmd.to_string(),
                });
            }
            continue;
        }
        for span in code_spans(line) {
            if let Some(cmd) = strip_prefix(span) {
                out.push(Found {
                    at: at.clone(),
                    text: cmd.to_string(),
                });
            }
        }
    }
    out
}

/// The inline code spans of one line (single or double backticks).
fn code_spans(line: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let ticks = rest[start..].chars().take_while(|&c| c == '`').count();
        let fence = &rest[start..start + ticks];
        let after = &rest[start + ticks..];
        match after.find(fence) {
            Some(end) => {
                spans.push(after[..end].trim());
                rest = &after[end + ticks..];
            }
            None => break,
        }
    }
    spans
}

/// Drop `[…]` groups, nesting included (rule 3).
fn drop_optional_groups(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_placeholder(tok: &str) -> bool {
    (tok.starts_with('<') && tok.ends_with('>') && tok.len() > 2)
        || (tok.starts_with('{') && tok.ends_with('}') && tok.len() > 2)
        || (tok.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && tok.chars().any(|c| c.is_ascii_uppercase()))
}

/// Rules 2–6: the argv to parse, or `None` when the line is skipped.
fn to_argv(text: &str, tree: &Command) -> Option<Vec<String>> {
    let text = drop_optional_groups(text)
        .replace('…', " ")
        .replace("...", " ");
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words
        .get(1)
        .is_some_and(|w| w.starts_with(|c: char| c.is_ascii_digit()))
    {
        return None;
    }
    // Rule 4: cut at shell operators, comments and prose markers.
    const STOP: [&str; 13] = [
        "|", "||", ">", ">>", "2>", "2>&1", "&&", ";", "&", "<", "·", "→", "—",
    ];
    if let Some(cut) = words.iter().position(|w| {
        STOP.contains(w)
            || w.starts_with('#')
            || w.starts_with('(')
            || w.starts_with('>')
            || w.starts_with("2>")
    }) {
        words.truncate(cut);
    }
    if let Some(i) = words.iter().position(|w| w.ends_with(';')) {
        words.truncate(i + 1);
        words[i] = words[i].trim_end_matches(';');
    }
    let mut argv: Vec<String> = Vec::new();
    let mut cmd = tree;
    let mut positionals_seen = 0usize;
    let mut descending = true;
    for (i, raw) in words.iter().enumerate() {
        let mut w = raw.trim_end_matches([',', '.', ':', ')']).to_string();
        if i == 0 {
            argv.push(w);
            continue;
        }
        if w.is_empty() {
            continue;
        }
        // Rule 5: `a|b` → `a` (a placeholder spelled `<a|b>` included).
        if w.contains('|') && !is_placeholder(&w) {
            w = w.split('|').next().unwrap().to_string();
        }
        if descending && !w.starts_with('-') {
            if let Some(sub) = cmd.find_subcommand(&w) {
                cmd = sub;
                argv.push(w);
                continue;
            }
            if is_placeholder(&w) && cmd.has_subcommands() && cmd.is_subcommand_required_set() {
                return None; // rule 6: a placeholder for the command itself
            }
        }
        if w.starts_with('-') {
            descending = false;
        }
        if is_placeholder(&w) {
            let prev = argv.last().map(String::as_str).unwrap_or("");
            let arg = if let Some(long) = prev.strip_prefix("--") {
                cmd.get_arguments().find(|a| a.get_long() == Some(long))
            } else {
                let p = cmd.get_positionals().nth(positionals_seen);
                positionals_seen += 1;
                descending = false;
                p
            };
            let value = arg
                .and_then(|a| a.get_possible_values().into_iter().next())
                .map(|pv| pv.get_name().to_string())
                .unwrap_or_else(|| "1".to_string());
            argv.push(value);
        } else {
            if !w.starts_with('-') && !argv.last().is_some_and(|p| p.starts_with("--")) {
                positionals_seen += 1;
            }
            argv.push(w);
        }
    }
    Some(argv)
}

fn parse_failure(argv: &[String], tree: &Command) -> Option<String> {
    use clap::error::ErrorKind;
    match tree.clone().try_get_matches_from(argv) {
        Ok(_) => None,
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                    | ErrorKind::MissingRequiredArgument
                    | ErrorKind::MissingSubcommand
            ) =>
        {
            None
        }
        Err(e) => Some(
            e.to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
    }
}

fn markdown_files() -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    let root = repo_root();
    let mut files = Vec::new();
    for dir in [
        "crates/vitrify-core/assets/briefs",
        "crates/vitrify-core/assets/schemas",
    ] {
        // a missing directory fails loudly: skipping it would silently drop
        // its files from the conformance check
        let rd = std::fs::read_dir(root.join(dir)).unwrap_or_else(|e| panic!("{dir}: {e}"));
        files.extend(
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md")),
        );
    }
    walk(&root.join("docs"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (rel, p)
        })
        .filter(|(rel, _)| rel != "docs/about/release-notes.md")
        .collect()
}

#[test]
fn every_command_line_in_briefs_schemas_and_docs_parses() {
    let tree = root();
    let mut failures = Vec::new();
    let mut parsed = 0usize;
    for (rel, path) in markdown_files() {
        let body = std::fs::read_to_string(&path).unwrap();
        for found in extract_markdown(&rel, &body) {
            let Some(argv) = to_argv(&found.text, &tree) else {
                continue;
            };
            parsed += 1;
            if let Some(err) = parse_failure(&argv, &tree) {
                failures.push(format!("{}: {}: {err}", found.at, found.text));
            }
        }
    }
    assert!(parsed > 50, "only {parsed} command lines found");
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn every_example_in_command_help_parses() {
    let tree = root();
    let mut failures = Vec::new();
    for path in command_paths(&tree) {
        let block = find(&tree, &path)
            .get_after_long_help()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let mut in_examples = false;
        for line in block.lines() {
            if line.starts_with("Examples:") {
                in_examples = true;
                // An example may sit on the heading's own line.
                let rest = line.trim_start_matches("Examples:");
                if let Some(cmd) = strip_prefix(rest) {
                    check_example(&tree, &path, cmd, &mut failures);
                }
                continue;
            }
            if in_examples {
                if !line.starts_with(' ') && !line.trim().is_empty() {
                    in_examples = false;
                    continue;
                }
                if let Some(cmd) = strip_prefix(line) {
                    check_example(&tree, &path, cmd, &mut failures);
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

fn check_example(tree: &Command, path: &[String], cmd: &str, failures: &mut Vec<String>) {
    if let Some(argv) = to_argv(cmd, tree)
        && let Some(err) = parse_failure(&argv, tree)
    {
        failures.push(format!(
            "vitrify {} --help: Examples: {cmd}: {err}",
            path.join(" ")
        ));
    }
}

#[test]
fn extraction_rules() {
    let tree = root();
    let argv = |s: &str| to_argv(s, &tree).map(|v| v.join(" "));
    assert_eq!(
        argv("vitrify new tdd|sdd [--bundle | --single] [name]").as_deref(),
        Some("vitrify new tdd")
    );
    assert_eq!(
        argv("vitrify brief <task> [--unit U-### --holder ID]").as_deref(),
        Some("vitrify brief onboard")
    );
    assert_eq!(
        argv("vitrify trace record --unit U-001 --stamp <kind>").as_deref(),
        Some("vitrify trace record --unit U-001 --stamp built")
    );
    assert_eq!(
        argv("vitrify journal append --event KIND --claim-seq <seq>").as_deref(),
        Some("vitrify journal append --event claim --claim-seq 1")
    );
    assert_eq!(
        argv("vitrify audit; echo $?").as_deref(),
        Some("vitrify audit")
    );
    assert_eq!(
        argv("vitrify run > run.out 2> run.err &").as_deref(),
        Some("vitrify run")
    );
    assert_eq!(
        argv("vitrify locate --units · vitrify run status").as_deref(),
        Some("vitrify locate --units")
    );
    assert_eq!(argv("vitrify <command> --help"), None);
    assert_eq!(argv("vitrify 0.5.0 procedures — print one"), None);
    assert_eq!(
        strip_prefix("$ nohup vitrify run --sdd specs/calc"),
        Some("vitrify run --sdd specs/calc")
    );
    let spans = code_spans("run `vitrify lint --sdd x` then ``vitrify check``");
    assert_eq!(spans, vec!["vitrify lint --sdd x", "vitrify check"]);
    assert!(parse_failure(&["vitrify".into(), "mirror".into()], &tree).is_some());
    assert!(parse_failure(&["vitrify".into(), "--help".into()], &tree).is_none());
}

// ------------------------------------------------------------ snapshots

#[test]
fn snapshot_root_help() {
    let cmd = root();
    insta::assert_snapshot!("root_short", cmd.clone().render_help().to_string());
    insta::assert_snapshot!("root_long", cmd.clone().render_long_help().to_string());
}

#[test]
fn snapshot_command_help() {
    let cmd = root();
    for path in command_paths(&cmd) {
        let name = path.join("_");
        insta::assert_snapshot!(name, long_help(&cmd, &path));
    }
}
