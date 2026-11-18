//! The yamlish front-matter subset parser and in-place writer.
//!
//! The one front-matter grammar for every verb — held by the regression
//! goldens (`fixtures/golden/`, `fixtures/example/`).
//! Deliberately NOT full YAML: masking and hashing operate on raw
//! lines, and YAML normalization would silently break byte-identical hashing.
//! Grammar: scalar `key: value`, inline `[a, b]` lists, block lists
//! (`- item` continuation lines), one-level nested maps (the `trace` block),
//! `#` comment stripping, key order preserved.

use crate::newline::split_universal;

/// A parsed yamlish value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    List(Vec<String>),
    Map(Vec<(String, String)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Cut the value at the first `#` that is preceded by whitespace, then
/// trim — inline comments drop.
fn strip_comment(v: &str) -> String {
    let bytes = v.as_bytes();
    let mut cut = v.len();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && (bytes[i - 1] as char).is_whitespace() {
            // back up over the contiguous whitespace run before the '#'
            let mut s = i;
            while s > 0 && (bytes[s - 1] as char).is_whitespace() {
                s -= 1;
            }
            cut = s;
            break;
        }
    }
    v[..cut].trim().to_string()
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Match `^([A-Za-z0-9_-]+):\s*(.*)$` → (key, rest-after-colon-and-ws).
fn match_key_line(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty() || !key.chars().all(is_key_char) {
        return None;
    }
    let rest = line[colon + 1..].trim_start();
    Some((key, rest))
}

fn strip_quotes(s: &str) -> &str {
    s.trim_matches(|c| c == '\'' || c == '"')
}

/// Parse a front-matter block into ordered (key, value) pairs.
/// Duplicate keys update the value at the first occurrence (dict semantics).
pub fn parse(block: &str) -> Vec<(String, Value)> {
    let lines = split_universal(block);
    let mut data: Vec<(String, Value)> = Vec::new();
    let mut set = |key: &str, value: Value| match data.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => data.push((key.to_string(), value)),
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty()
            || line.trim_start().starts_with('#')
            || line.starts_with(' ')
            || line.starts_with('\t')
        {
            i += 1;
            continue;
        }
        let Some((key, rest)) = match_key_line(line) else {
            i += 1;
            continue;
        };
        let raw = strip_comment(rest);
        if raw.is_empty() {
            let mut items: Vec<String> = Vec::new();
            let mut sub: Vec<(String, String)> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let nl = lines[j];
                if nl.trim().is_empty() {
                    j += 1;
                    continue;
                }
                if let Some(item) = match_block_item(nl) {
                    items.push(strip_comment(item));
                    j += 1;
                } else if let Some((sk, sv)) = match_sub_entry(nl) {
                    match sub.iter_mut().find(|(k, _)| k == sk) {
                        Some(slot) => slot.1 = strip_comment(sv),
                        None => sub.push((sk.to_string(), strip_comment(sv))),
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            let value = if !items.is_empty() {
                Value::List(items)
            } else if !sub.is_empty() {
                Value::Map(sub)
            } else {
                Value::Str(String::new())
            };
            set(key, value);
            i = j;
        } else if raw.starts_with('[') && raw.ends_with(']') {
            let items = raw[1..raw.len() - 1]
                .split(',')
                .filter(|x| !x.trim().is_empty())
                .map(|x| strip_quotes(x.trim()).to_string())
                .collect();
            set(key, Value::List(items));
            i += 1;
        } else {
            set(key, Value::Str(strip_quotes(&raw).to_string()));
            i += 1;
        }
    }
    data
}

/// Match `^\s+-\s+` and return the remainder after the marker.
fn match_block_item(line: &str) -> Option<&str> {
    let trimmed_start = line.len() - line.trim_start().len();
    if trimmed_start == 0 {
        return None;
    }
    let rest = &line[trimmed_start..];
    let rest = rest.strip_prefix('-')?;
    if rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Match `^\s+([^:\s]+):\s*(.*)$` on an indented line.
fn match_sub_entry(line: &str) -> Option<(&str, &str)> {
    let indent = line.len() - line.trim_start().len();
    if indent == 0 {
        return None;
    }
    let rest = &line[indent..];
    let colon = rest.find(':')?;
    let key = &rest[..colon];
    if key.is_empty() || key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some((key, rest[colon + 1..].trim_start()))
}

/// Split a document into front matter and body on `---` fence lines
/// (the kit's `FM_RE`: `\A---\r?\n(.*?\r?\n)---\r?\n`). Returns
/// `(inner_block, body_after_closing_fence)`; `None` when the document does
/// not begin with a fence or no closing fence line exists.
pub fn split_front_matter(doc: &str) -> Option<(&str, &str)> {
    let after_open = doc
        .strip_prefix("---\n")
        .or_else(|| doc.strip_prefix("---\r\n"))?;
    // Find the earliest line start (position after a '\n') where a closing
    // fence `---\r?\n` begins; the inner block must end with that newline.
    let bytes = after_open.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let p = i + 1;
        let rest = &after_open[p..];
        if let Some(body) = rest.strip_prefix("---\n") {
            return Some((&after_open[..p], body));
        }
        if let Some(body) = rest.strip_prefix("---\r\n") {
            return Some((&after_open[..p], body));
        }
    }
    None
}

/// Replace (first occurrence) or append a top-level scalar, preserving any
/// trailing `# comment` of the replaced line. Returns the rewritten block,
/// lines joined with LF and newline-terminated; all other lines byte-equal.
pub fn set_scalar_in_place(block: &str, key: &str, value: &str) -> String {
    let lines = split_universal(block);
    let new_line = format!("{key}: {value}");
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut done = false;
    for line in &lines {
        if !done && let Some((k, rest)) = match_key_line_exact(line, key) {
            debug_assert_eq!(k, key);
            let comment = trailing_comment(rest);
            match comment {
                Some(c) => out.push(format!("{new_line}  {c}")),
                None => out.push(new_line.clone()),
            }
            done = true;
        } else {
            out.push((*line).to_string());
        }
    }
    if !done {
        out.push(new_line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Match `^{key}:\s*(.*)$` for a specific literal key.
fn match_key_line_exact<'a>(line: &'a str, key: &str) -> Option<(&'a str, &'a str)> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.strip_prefix(':')?;
    Some((&line[..key.len()], rest.trim_start()))
}

/// The trailing comment of the old value (whitespace then `#…`), if any.
fn trailing_comment(rest: &str) -> Option<&str> {
    let bytes = rest.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && (bytes[i - 1] as char).is_whitespace() {
            return Some(&rest[i..]);
        }
    }
    None
}

/// Replace (or append) a one-level nested map block — the kit's
/// `set_trace_block` generalized. The replaced block is the `key:` line plus
/// every following blank-or-indented line; the new block is column-aligned
/// exactly like the reference kit writes it.
pub fn set_map_in_place(block: &str, key: &str, pairs: &[(String, String)]) -> String {
    let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
    let mut new_block: Vec<String> = vec![format!("{key}:")];
    for (k, v) in pairs {
        new_block.push(format!(
            "  {:<width$} {}",
            format!("{k}:"),
            v,
            width = width
        ));
    }
    let lines = split_universal(block);
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with(key) && line[key.len()..].starts_with(':') {
            i += 1;
            while i < lines.len()
                && (lines[i].trim().is_empty()
                    || lines[i].starts_with(' ')
                    || lines[i].starts_with('\t'))
            {
                i += 1;
            }
            out.extend(new_block.iter().cloned());
            placed = true;
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    if !placed {
        out.extend(new_block.iter().cloned());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
