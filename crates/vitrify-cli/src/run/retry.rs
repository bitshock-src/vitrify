//! What an attempt that did not land leaves behind, and how the next
//! attempt gets it.
//!
//! An attempt's work is never thrown away: when it is set aside — rejected
//! by the gate, blocked, released, timed out, interrupted — its worktree is
//! committed to its own branch under a note saying why, and only then
//! removed. The next claim of that unit starts from the files the unit owns
//! on that branch, and its worker is told what went wrong. Nothing is
//! remembered between runs: the branch and its note are in git, and the
//! journal says which attempts a later verification superseded.

use std::path::Path;

use vitrify_core::model::Instance;
use vitrify_core::writeset::WriteSet;
use vitrify_core::{CoreError, journal};

use crate::git::Git;
use crate::layout;
use crate::out::diag;

/// First words of the commit that sets an attempt aside.
pub const ABANDONED: &str = "sdd: abandoned attempt";

/// The latest attempt at a unit that neither landed nor was superseded.
#[derive(Debug, Clone)]
pub struct Previous {
    pub branch: String,
    pub seq: i64,
    /// The outcome it was set aside with (`rejected-gate`, `blocked`, …);
    /// empty when the branch carries no note.
    pub outcome: String,
    /// Why, in the words recorded at the time.
    pub note: String,
}

/// The newest attempt branch of `unit` below claim `below`, among those
/// made after the unit was last Verified — earlier ones were superseded by
/// that verification. `inst` is the SDD at the head the new claim sees.
pub fn previous_attempt(
    git: &Git,
    inst: &Instance,
    unit: &str,
    below: i64,
) -> Result<Option<Previous>, CoreError> {
    let slug = layout::sdd_slug(inst);
    let (events, _) = journal::read(&inst.handle())?;
    let verified_at = events
        .iter()
        .filter(|e| e.event == "verified" && e.unit.as_deref() == Some(unit))
        .filter_map(|e| e.claim_seq)
        .max()
        .unwrap_or(0);
    let mut best: Option<(i64, String)> = None;
    for branch in git
        .branches_under(&layout::branch_prefix(&slug, unit))
        .map_err(CoreError::from)?
    {
        let Some(seq) = layout::branch_seq(&branch) else {
            continue;
        };
        if seq > verified_at && seq < below && best.as_ref().is_none_or(|(s, _)| seq > *s) {
            best = Some((seq, branch));
        }
    }
    let Some((seq, branch)) = best else {
        return Ok(None);
    };
    let message = git.commit_message(&branch).unwrap_or_default();
    let (subject, body) = message.split_once('\n').unwrap_or((message.as_str(), ""));
    let (outcome, note) = match subject.strip_prefix(ABANDONED) {
        Some(rest) => (
            rest.rsplit_once(" — ")
                .map(|(_, o)| o.trim().to_string())
                .unwrap_or_default(),
            body.trim().to_string(),
        ),
        None => (String::new(), String::new()),
    };
    Ok(Some(Previous {
        branch,
        seq,
        outcome,
        note,
    }))
}

/// Copy into the worktree `wt` every file the previous attempt added or
/// changed that the unit owns, left as uncommitted changes. Spec files are
/// never taken: the new claim's are the current truth. Returns the paths.
pub fn seed(
    wt: &Git,
    ws: &WriteSet,
    claim_commit: &str,
    prev: &Previous,
) -> Result<Vec<String>, CoreError> {
    let fork = wt
        .merge_base(claim_commit, &prev.branch)
        .map_err(CoreError::from)?;
    let owned: Vec<String> = wt
        .added_or_modified(&fork, &prev.branch)
        .map_err(CoreError::from)?
        .into_iter()
        .filter(|p| ws.covers(p))
        .collect();
    wt.restore_paths(&prev.branch, &owned)
        .map_err(CoreError::from)?;
    Ok(owned)
}

/// Set an attempt aside: commit everything in its worktree to its branch
/// under a note naming the outcome and the reason, then remove the
/// worktree. The branch stays. If the commit fails the worktree stays too —
/// it is then the only copy of the work.
pub fn abandon(primary: &Git, wt: &Path, unit: &str, seq: i64, outcome: &str, detail: &str) {
    if !wt.join(".git").exists() {
        return;
    }
    let git = primary.at(wt);
    let subject = format!("{ABANDONED} {unit} (c{seq}) — {outcome}");
    let saved = git
        .add_all()
        .and_then(|()| git.commit_note(&subject, &clip(detail, 60, 6000)));
    match saved {
        Ok(()) => {
            let _ = primary.worktree_remove(wt);
        }
        Err(e) => diag(&format!(
            "warning: could not save the attempt at {unit} (c{seq}) to its branch ({e}); its worktree is kept at {}",
            wt.display()
        )),
    }
}

/// The last `lines` lines of `text`, at most `bytes` bytes.
pub fn clip(text: &str, lines: usize, bytes: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    let mut out = all[start..].join("\n");
    if out.len() > bytes {
        let mut cut = out.len() - bytes;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = out[cut..].to_string();
    }
    out
}

/// What the orchestrator has already done for a worker.
pub struct Orchestrated<'a> {
    pub unit: &'a str,
    pub holder: &'a str,
    pub seq: i64,
    pub branch: &'a str,
    pub worktree: &'a Path,
    /// The SDD's root relative to the worktree root: the `--sdd` argument.
    pub sdd: &'a str,
}

/// The "This session" part of a worker prompt: what is already done for
/// this worker (`orchestrated`), what an earlier attempt left (`prev`, with
/// the files `seeded` from it), and why a once-Verified unit is being
/// rebuilt (`stale_cause`). Empty when there is nothing to say.
pub fn section(
    orchestrated: Option<&Orchestrated<'_>>,
    prev: Option<&Previous>,
    seeded: &[String],
    stale_cause: Option<&str>,
) -> String {
    let mut items: Vec<String> = Vec::new();
    if let Some(o) = orchestrated {
        items.push(format!(
            "- **Your claim is made and your working copy is ready.** The orchestrator claimed {} for holder `{}` (claim `c{}`) and created the worktree `{}` on branch `{}` — your current directory. Skip steps 1 and 2.",
            o.unit,
            o.holder,
            o.seq,
            o.worktree.display(),
            o.branch
        ));
        items.push(
            "- **Stay in that directory.** Run every command from it and write nothing outside it: the primary checkout and every other worktree are not yours."
                .to_string(),
        );
        items.push(
            "- **The lease is the orchestrator's.** Do not run `vitrify claim`, `vitrify renew` or `vitrify release`: the orchestrator renews the lease while you work, and releases it when you report `blocked` or `released`."
                .to_string(),
        );
        items.push(format!(
            "- **Name the SDD.** Every `vitrify` command you run takes `--sdd {}` — this repository may hold several SDDs, each with its own `{}`.",
            o.sdd, o.unit
        ));
    }
    if let Some(cause) = stale_cause {
        items.push(format!(
            "- **This unit was built and verified before, then marked Stale** ({cause}). Its earlier code is in your working copy. Take the reading set below as the current truth, bring the files this unit owns into line with it — often a small change, sometimes none — run the full unit gate, and stamp Built again."
        ));
    }
    if let Some(p) = prev {
        let how = if p.outcome.is_empty() {
            "did not land".to_string()
        } else {
            format!("did not land: `{}`", p.outcome)
        };
        let mut item = format!(
            "- **An earlier attempt at this unit {how}** (branch `{}`).",
            p.branch
        );
        if !p.note.is_empty() {
            item.push_str(" What was recorded:\n\n");
            for line in p.note.lines() {
                item.push_str("      ");
                item.push_str(line);
                item.push('\n');
            }
        }
        items.push(item.trim_end().to_string());
        if seeded.is_empty() {
            if orchestrated.is_none() {
                items.push(format!(
                    "- Its files are on that branch: `git checkout {} -- <path>` brings back one this unit owns. They are a draft written against the spec as it then stood.",
                    p.branch
                ));
            }
        } else {
            items.push(format!(
                "- **Its files are already in your working copy**, uncommitted: {}. They were written against the spec as it then stood and did not land: treat them as a draft, check every line against the reading set below, and keep, fix or replace them as that requires.",
                seeded
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if items.is_empty() {
        return String::new();
    }
    format!("## This session\n\n{}", items.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_keeps_the_tail() {
        let text = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let c = clip(&text, 3, 1000);
        assert_eq!(c, "line 98\nline 99\nline 100");
        assert_eq!(clip("αβγδ", 5, 4), "γδ");
    }

    #[test]
    fn section_says_nothing_when_there_is_nothing_to_say() {
        assert_eq!(section(None, None, &[], None), "");
    }

    #[test]
    fn section_names_the_branch_the_outcome_and_the_seeded_files() {
        let prev = Previous {
            branch: "sdd/x/U-003/c2".into(),
            seq: 2,
            outcome: "rejected-gate".into(),
            note: "failing gate: `npm test`\nexpected 3, got 4".into(),
        };
        let s = section(None, Some(&prev), &["src/a.ts".to_string()], None);
        assert!(s.starts_with("## This session\n\n"));
        assert!(s.contains("did not land: `rejected-gate`"));
        assert!(s.contains("branch `sdd/x/U-003/c2`"));
        assert!(s.contains("      expected 3, got 4"));
        assert!(s.contains("already in your working copy"));
        assert!(s.contains("`src/a.ts`"));
    }
}
