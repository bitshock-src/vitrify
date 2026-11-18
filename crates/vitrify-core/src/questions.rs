//! Open questions (schema §3): the `Q-###` entries of an SDD's Decision Log
//! — `decisions.md` in a bundle, the `## Decisions & Open Questions`
//! section of a single file — and the `OPEN(Q-###)` markers units carry.
//!
//! One rule decides a question's status, here and nowhere else: it is
//! **open unless its entry marks it resolved** — the word `resolved` in the
//! parenthesis that follows its id. An entry that says nothing is open.

use crate::CoreError;
use crate::model::{self, SddHandle, Shape};

/// One `Q-###` entry of a Decision Log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    /// Not marked resolved.
    pub open: bool,
    /// The parenthesis after the id, without its brackets: `2026-10-19, open`.
    pub meta: String,
    /// The entry's text on one line.
    pub text: String,
}

/// `Q-<digits>` at the start of `s`, and what follows it.
fn leading_id(s: &str) -> Option<(String, &str)> {
    let rest = s.strip_prefix("Q-")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| (format!("Q-{}", &rest[..digits]), &rest[digits..]))
}

/// A list item that opens a Decision-Log entry: `- **Q-007** …`. Returns the
/// id and the rest of the line.
fn entry_start(line: &str) -> Option<(String, &str)> {
    let t = line.trim_start();
    let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    let t = t.trim_start().strip_prefix("**")?;
    let (id, rest) = leading_id(t)?;
    Some((id, rest.strip_prefix("**")?))
}

/// A line that ends the entry before it: a heading, or another list item at
/// the margin.
fn ends_entry(line: &str) -> bool {
    line.starts_with('#') || line.starts_with("- ") || line.starts_with("* ")
}

fn says_resolved(meta: &str) -> bool {
    meta.split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("resolved"))
}

/// The `Q-###` entries of a Decision Log's text, in document order.
pub fn parse(text: &str) -> Vec<Question> {
    let lines = crate::newline::split_universal(text);
    let mut out: Vec<Question> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((id, rest)) = entry_start(lines[i]) else {
            i += 1;
            continue;
        };
        let rest = rest.trim_start();
        let (meta, body) = match rest.strip_prefix('(').and_then(|r| r.split_once(')')) {
            Some((meta, body)) => (meta.trim().to_string(), body),
            None => (String::new(), rest),
        };
        let mut words: Vec<&str> = body
            .trim_start()
            .trim_start_matches(['—', '–', '-', ':'])
            .split_whitespace()
            .collect();
        // the entry runs on over indented lines, blank ones between them
        // included, until the next entry, item or heading
        let mut j = i + 1;
        while j < lines.len() {
            let line = lines[j];
            if line.trim().is_empty() {
                let next = lines[j + 1..].iter().find(|l| !l.trim().is_empty());
                if next.is_none_or(|l| !l.starts_with([' ', '\t']) || entry_start(l).is_some()) {
                    break;
                }
            } else if ends_entry(line) || entry_start(line).is_some() {
                break;
            } else {
                words.extend(line.split_whitespace());
            }
            j += 1;
        }
        out.push(Question {
            id,
            open: !says_resolved(&meta),
            meta,
            text: words.join(" "),
        });
        i = j;
    }
    out
}

/// An SDD's Decision Log; `None` when it keeps none.
pub fn decision_log(sdd: &SddHandle) -> Result<Option<String>, CoreError> {
    match sdd.shape {
        Shape::Bundle => {
            let path = sdd.root.join("decisions.md");
            if !path.is_file() {
                return Ok(None);
            }
            model::read_text(&path).map(Some)
        }
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            Ok(model::section_span(&text, "Decisions & Open Questions")
                .map(|(s, e)| text[s..e].to_string()))
        }
    }
}

/// The questions of an SDD's Decision Log.
pub fn of(sdd: &SddHandle) -> Result<Vec<Question>, CoreError> {
    Ok(decision_log(sdd)?.map(|t| parse(&t)).unwrap_or_default())
}

/// The questions `text` is marked open on: each `Q-###` of an
/// `OPEN(Q-###)` marker, once, in order.
pub fn markers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("OPEN(") {
        let at = from + rel + "OPEN(".len();
        if let Some((id, rest)) = leading_id(&text[at..])
            && rest.starts_with(')')
            && !out.contains(&id)
        {
            out.push(id);
        }
        from = at;
    }
    out
}

/// The unit a question vitrify itself landed was reported on: the entry a
/// run writes reads `reported by <holder> on U-###: <notes>`.
pub fn reported_on(text: &str) -> Option<String> {
    let rest = text.strip_prefix("reported by ")?;
    let (head, _) = rest.split_once(':')?;
    let unit = head.rsplit_once(" on ")?.1.trim();
    let digits = unit.strip_prefix("U-")?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| unit.to_string())
}

/// The entry a run lands for a question a worker stopped at — open by the
/// rule above, and saying so.
pub fn landed_entry(q: &str, date: &str, holder: &str, unit: &str, notes: &str) -> String {
    format!("- **{q}** ({date}, open) — reported by {holder} on {unit}: {notes}\n")
}

/// A line that only says there are no questions: `None.`, `_None._`,
/// `(none)`, `—`.
fn says_none(line: &str) -> bool {
    let t = line.trim();
    let t = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .unwrap_or(t);
    let t = t
        .trim_matches(|c: char| matches!(c, '*' | '_' | '(' | ')') || c.is_whitespace())
        .trim_end_matches('.');
    matches!(
        t.to_lowercase().as_str(),
        "none" | "none yet" | "none so far" | "no open questions" | "n/a" | "—" | "–"
    )
}

/// The `#` count of a heading line, or `None` for any other line.
fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    ((1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace)).then_some(hashes)
}

/// `log`, a Decision Log's text, with `entry` — one line — landed in it:
/// at the end of its `Open Questions` section when it has one, in place of
/// a line that says there are none; otherwise after its last line of text.
/// Everything else is kept as it was.
pub fn land(log: &str, entry: &str) -> String {
    // (start, end with its newline, content without it) of every line
    let mut lines: Vec<(usize, usize, &str)> = Vec::new();
    let mut at = 0;
    for piece in log.split_inclusive('\n') {
        let content = piece.trim_end_matches(['\n', '\r']);
        lines.push((at, at + piece.len(), content));
        at += piece.len();
    }
    // the lines of the Open Questions section, outside fenced code
    let mut fence: Option<&str> = None;
    let mut section: Option<(usize, usize)> = None; // heading index, level
    let mut body_end = lines.len();
    for (i, (_, _, line)) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            continue;
        }
        let Some(level) = heading_level(line) else {
            continue;
        };
        match section {
            None if line[level..]
                .trim()
                .to_lowercase()
                .starts_with("open questions") =>
            {
                section = Some((i, level));
            }
            Some((_, open_level)) if level <= open_level => {
                body_end = i;
                break;
            }
            _ => {}
        }
    }
    let (first, last) = match section {
        Some((heading, _)) => (heading + 1, body_end),
        None => (0, lines.len()),
    };
    let text_lines: Vec<usize> = (first..last)
        .filter(|&i| !lines[i].2.trim().is_empty())
        .collect();
    let mut out = String::with_capacity(log.len() + entry.len() + 2);
    match (section, text_lines.as_slice()) {
        // the section says there are none: the entry takes that line's place
        (Some(_), [only]) if says_none(lines[*only].2) => {
            let (s, e, _) = lines[*only];
            out.push_str(&log[..s]);
            out.push_str(entry);
            out.push_str(&log[e..]);
        }
        // after the last line of text: straight after a list, else after a
        // blank line
        (_, [.., last_text]) => {
            let (_, e, prev) = lines[*last_text];
            out.push_str(&log[..e]);
            if !log[..e].ends_with('\n') {
                out.push('\n');
            }
            if !(prev.starts_with("- ") || prev.starts_with("* ") || prev.starts_with([' ', '\t']))
            {
                out.push('\n');
            }
            out.push_str(entry);
            out.push_str(&log[e..]);
        }
        // an empty section: the entry under its heading
        (Some((heading, _)), []) => {
            let (_, e, _) = lines[heading];
            out.push_str(&log[..e]);
            if !log[..e].ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(entry);
            let rest = &log[e..];
            if !rest.is_empty() && !rest.starts_with(['\n', '\r']) {
                out.push('\n');
            }
            out.push_str(rest);
        }
        // nothing at all
        (None, []) => out.push_str(entry),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_unless_marked_resolved() {
        let log = "# Decisions & Open Questions\n\n- **D-001** (2026-10-12, SDD 1.3.0) — a decision.\n- **Q-001** (2026-10-19, open) — TDD §3.2 omits currency;\n  conflicts with C-004. Blocked: U-011.\n- **Q-002** (2026-10-19, resolved 2026-10-24) — settled.\n- **Q-003** (2026-11-01, SDD 1.2.0; Resolved 2026-11-02) — also settled.\n- **Q-004** — no parenthesis at all\n- **Q-005** (2026-11-03) — unresolved questions about X\n";
        let got = parse(log);
        let view: Vec<(&str, bool)> = got.iter().map(|q| (q.id.as_str(), q.open)).collect();
        assert_eq!(
            view,
            [
                ("Q-001", true),
                ("Q-002", false),
                ("Q-003", false),
                ("Q-004", true),
                ("Q-005", true),
            ]
        );
        // continuation lines are part of the entry
        assert_eq!(
            got[0].text,
            "TDD §3.2 omits currency; conflicts with C-004. Blocked: U-011."
        );
        assert_eq!(got[0].meta, "2026-10-19, open");
        // `unresolved` in the text — or in the parenthesis — is not `resolved`
        assert_eq!(got[4].text, "unresolved questions about X");
        assert!(parse("- **Q-009** (unresolved) — x\n")[0].open);
    }

    #[test]
    fn an_entry_runs_to_the_next_item_or_heading() {
        let log = "- **Q-001** (2026-10-19, open) — first line\n\n  a second paragraph, indented\n\n- not part of it\n\n## Later\n\n- **Q-002** (resolved) — x\n";
        let got = parse(log);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "first line a second paragraph, indented");
        assert!(!got[1].open);
    }

    #[test]
    fn what_a_run_lands_reads_as_open_and_names_its_unit() {
        let entry = landed_entry("Q-042", "2026-11-15", "run-1-w2", "U-001", "spec gap");
        assert_eq!(
            entry,
            "- **Q-042** (2026-11-15, open) — reported by run-1-w2 on U-001: spec gap\n"
        );
        let got = parse(&entry);
        assert!(got[0].open);
        assert_eq!(reported_on(&got[0].text).as_deref(), Some("U-001"));
        // the entry vitrify 0.2 wrote carried no status: open, by the rule
        let old = "- **Q-042** (2026-11-15) — reported by run-1-w2 on U-001: spec gap\n";
        assert!(parse(old)[0].open);
        assert_eq!(reported_on("TDD §3.2 omits currency"), None);
    }

    #[test]
    fn markers_name_each_question_once() {
        let text = "Use OPEN(Q-007) here and OPEN(Q-007) there; OPEN(Q-012).\nThe placeholder OPEN(Q-###) is prose, OPEN(Q-9x) is not a marker.";
        assert_eq!(markers(text), ["Q-007", "Q-012"]);
        assert!(markers("nothing open").is_empty());
    }

    #[test]
    fn a_landed_question_takes_the_place_of_none() {
        let q = "- **Q-999** (2026-11-18, open) — reported by run-1-w3 on U-004: which pos?\n";
        // the section that said there were none says what there is
        let log = "# Decisions & Open Questions — calc\n\n## Decision Log\n\n- **D-001** (2026-10-28) — a.\n\n## Open Questions\n\nNone.\n";
        assert_eq!(
            land(log, q),
            format!(
                "# Decisions & Open Questions — calc\n\n## Decision Log\n\n- **D-001** (2026-10-28) — a.\n\n## Open Questions\n\n{q}"
            )
        );
        for none in ["_None._", "(none)", "- None", "—", "None yet."] {
            let log = format!("## Open Questions\n\n{none}\n\n## Later\n\nkept\n");
            assert_eq!(
                land(&log, q),
                format!("## Open Questions\n\n{q}\n## Later\n\nkept\n"),
                "{none}"
            );
        }
        // under questions already there, before the next section
        let log = "## Open Questions\n\n- **Q-001** (2026-10-01, resolved 2026-10-02) — x.\n  more of it\n\n## Notes\n\nNone.\n";
        assert_eq!(
            land(log, q),
            format!(
                "## Open Questions\n\n- **Q-001** (2026-10-01, resolved 2026-10-02) — x.\n  more of it\n{q}\n## Notes\n\nNone.\n"
            )
        );
        // an empty section, or one of prose; a heading quoted in a fence is prose
        assert_eq!(
            land("## Open Questions\n\n## Notes\n", q),
            format!("## Open Questions\n\n{q}\n## Notes\n")
        );
        assert_eq!(
            land(
                "## Open Questions\n\nNothing is open yet, as of today.\n",
                q
            ),
            format!("## Open Questions\n\nNothing is open yet, as of today.\n\n{q}")
        );
        let fenced = "```\n## Open Questions\nNone.\n```\n- **D-001** — a.\n";
        assert_eq!(land(fenced, q), format!("{fenced}{q}"));
    }

    #[test]
    fn a_log_without_an_open_questions_section_gets_the_question_after_its_last_line() {
        let q = "- **Q-002** (2026-11-18, open) — reported by run-1-w1 on U-001: gap\n";
        // the template's register: after its last entry
        let log = "# Decisions & Open Questions\n\nAppend-only register.\n\n- **D-001** (2026-10-01, SDD 0.1.0) — a.\n";
        assert_eq!(land(log, q), format!("{log}{q}"));
        // a single file's section, cut before the next `##`: the blank line
        // before that heading stays after the question
        let section = "## Decisions & Open Questions\n\n- **D-001** — a.\n- **Q-001** — b.\n\n";
        assert_eq!(
            land(section, q),
            format!("## Decisions & Open Questions\n\n- **D-001** — a.\n- **Q-001** — b.\n{q}\n")
        );
        // after prose, a blank line first; a file with no final newline gets one
        assert_eq!(land("# Decisions", q), format!("# Decisions\n\n{q}"));
        assert_eq!(land("", q), q);
        // what is landed reads back as the open question it is
        let landed = land(log, q);
        let got = parse(&landed);
        assert_eq!(
            got.last().map(|g| (g.id.as_str(), g.open)),
            Some(("Q-002", true))
        );
    }
}
