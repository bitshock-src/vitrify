//! The single output module: stdout is data, stderr is diagnostics —
//! everywhere, no exceptions. JSON emitters match
//! the golden corpus' output formats byte for byte: non-ASCII escaped as `\uXXXX`,
//! separators `(", ", ": ")` on the single-line form, or two-space indent
//! with separators `(",", ": ")`. Normative JSON is built from ordered
//! pairs, never from maps (Conventions).

/// Ordered JSON value — insertion-ordered objects for byte-stable output.
#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn s(v: impl Into<String>) -> J {
        J::Str(v.into())
    }

    pub fn opt_str(v: Option<String>) -> J {
        match v {
            Some(s) => J::Str(s),
            None => J::Null,
        }
    }

    /// A value read back from JSON. Objects keep the order the reader gives
    /// (`serde_json` sorts keys); a number that is not an integer becomes its
    /// text, since nothing vitrify writes has one.
    pub fn from_value(v: &serde_json::Value) -> J {
        use serde_json::Value;
        match v {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Number(n) => n
                .as_i64()
                .map(J::Int)
                .unwrap_or_else(|| J::Str(n.to_string())),
            Value::String(s) => J::Str(s.clone()),
            Value::Array(items) => J::Arr(items.iter().map(J::from_value).collect()),
            Value::Object(map) => J::Obj(
                map.iter()
                    .map(|(k, v)| (k.clone(), J::from_value(v)))
                    .collect(),
            ),
        }
    }

    /// Two-space-indented form: separators `(",", ": ")`, non-ASCII escaped.
    pub fn dumps_indent2(&self) -> String {
        let mut out = String::new();
        write_indent(self, 0, &mut out);
        out
    }

    /// Single-line form — separators `(", ", ": ")`, non-ASCII escaped.
    pub fn dumps_default(&self) -> String {
        let mut out = String::new();
        write_compact(self, ", ", ": ", &mut out);
        out
    }
}

fn escape_json_ascii(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x7f => out.push(c),
            c => {
                // ensure_ascii: \uXXXX, surrogate pairs for astral chars
                let cp = c as u32;
                if cp <= 0xffff {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    let v = cp - 0x10000;
                    out.push_str(&format!(
                        "\\u{:04x}\\u{:04x}",
                        0xd800 + (v >> 10),
                        0xdc00 + (v & 0x3ff)
                    ));
                }
            }
        }
    }
    out.push('"');
}

fn write_compact(v: &J, item_sep: &str, kv_sep: &str, out: &mut String) {
    match v {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Int(i) => out.push_str(&i.to_string()),
        J::Str(s) => escape_json_ascii(s, out),
        J::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                write_compact(item, item_sep, kv_sep, out);
            }
            out.push(']');
        }
        J::Obj(pairs) => {
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                escape_json_ascii(k, out);
                out.push_str(kv_sep);
                write_compact(val, item_sep, kv_sep, out);
            }
            out.push('}');
        }
    }
}

fn write_indent(v: &J, level: usize, out: &mut String) {
    let pad = "  ".repeat(level + 1);
    let close_pad = "  ".repeat(level);
    match v {
        J::Arr(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad);
                write_indent(item, level + 1, out);
            }
            out.push('\n');
            out.push_str(&close_pad);
            out.push(']');
        }
        J::Obj(pairs) if !pairs.is_empty() => {
            out.push_str("{\n");
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad);
                escape_json_ascii(k, out);
                out.push_str(": ");
                write_indent(val, level + 1, out);
            }
            out.push('\n');
            out.push_str(&close_pad);
            out.push('}');
        }
        other => write_compact(other, ", ", ": ", out),
    }
}

/// Data to stdout. A stream that has gone away (a closed pipe, a terminal
/// that hung up) is not a reason to stop: a run that loses its terminal
/// still has claims to release.
pub fn emit(data: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout().lock(), "{data}");
}

/// Data to stdout, flushed at once, for output someone watches line by line.
/// `false` when stdout has gone away (a closed pipe, as with `| head`): a
/// reader that only watches has no reason to go on.
pub fn emit_checked(data: &str) -> bool {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{data}").and_then(|()| out.flush()).is_ok()
}

/// Diagnostics to stderr.
pub fn diag(msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{msg}");
}
