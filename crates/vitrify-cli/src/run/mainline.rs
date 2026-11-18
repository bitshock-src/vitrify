//! The run's handle on mainline: which repository, which SDD, which branch,
//! and how a commit gets there.
//!
//! The SDD is pinned by its path relative to the repository root, resolved
//! once in preflight. Every later read — in the primary checkout, a unit
//! worktree, the integration checkout or a transaction — resolves that same
//! path and never searches, so a repository with several SDDs is as
//! unambiguous after preflight as it was during it.

use std::path::{Path, PathBuf};

use vitrify_core::claims::Op;
use vitrify_core::model::{self, Instance, UtcTime};
use vitrify_core::{CoreError, audit, journal, ledger, questions, yamlish};

use crate::cli::ClaimArgs;
use crate::cmd::{self, claim};
use crate::gate::ShRunner;
use crate::git::{Git, GitError};
use crate::layout::Layout;

#[derive(Debug, Clone)]
pub struct Mainline {
    /// The primary checkout.
    pub primary: Git,
    pub layout: Layout,
    /// The SDD's root relative to the repository root.
    pub sdd_rel: PathBuf,
    /// The mainline branch.
    pub branch: String,
    /// Publish by `git push` (origin arbitrates) rather than a local CAS.
    pub push: bool,
}

/// A commit that reached mainline.
#[derive(Debug, Clone)]
pub struct Landed {
    pub commit: String,
    /// What the local branch pointed at before it was moved to the commit;
    /// `None` when it was not moved (it has diverged from origin's).
    pub local_was: Option<String>,
}

/// What [`Mainline::heal`] put right.
#[derive(Debug, Default, Clone)]
pub struct Healed {
    /// Units whose lapsed lease was released.
    pub leases: Vec<String>,
    /// Units marked Stale, each with the cause.
    pub stale: Vec<(String, String)>,
    /// Findings the audit cannot fix itself.
    pub other: Vec<audit::Finding>,
    /// The published commit, when anything changed.
    pub commit: Option<String>,
}

impl Mainline {
    /// The mainline head (fetched first when origin arbitrates).
    pub fn head(&self) -> Result<String, CoreError> {
        let name = if self.push {
            let _ = self.primary.fetch_origin(&self.branch);
            format!("refs/remotes/origin/{}", self.branch)
        } else {
            format!("refs/heads/{}", self.branch)
        };
        self.primary.rev_parse(&name).map_err(CoreError::from)
    }

    /// The pinned SDD as `checkout` holds it.
    pub fn sdd_in(&self, checkout: &Path) -> Result<Instance, CoreError> {
        Instance::load(cmd::resolve_sdd_in(checkout, &self.sdd_rel)?)
    }

    /// One claim transaction on this run's SDD — published the way this run
    /// publishes, on the SDD this run pinned, wherever the process's
    /// working directory happens to be.
    pub fn claim(
        &self,
        op: Op,
        unit: &str,
        holder: &str,
        lease_minutes: i64,
    ) -> Result<claim::ClaimOutcome, CoreError> {
        let args = ClaimArgs {
            unit: unit.to_string(),
            holder: holder.to_string(),
            lease_minutes,
            sdd: None,
            mainline: Some(self.branch.clone()),
        };
        claim::execute_pinned(
            op,
            &args,
            &claim::Pinned {
                push: self.push,
                sdd_rel: &self.sdd_rel,
            },
        )
    }

    /// Bring the detached worktree at `path` to the mainline head, creating
    /// it on first use. Returns the head.
    pub fn sync_checkout(&self, path: &Path) -> Result<String, CoreError> {
        let head = self.head()?;
        self.checkout_at(path, &head)?;
        Ok(head)
    }

    /// Bring the detached worktree at `path` to `rev`, creating it on first
    /// use; untracked files git does not ignore are removed.
    pub fn checkout_at(&self, path: &Path, rev: &str) -> Result<(), CoreError> {
        if path.join(".git").exists() {
            let wt = self.primary.at(path);
            // an interrupted merge would refuse the reset
            let _ = wt.merge_abort();
            wt.reset_hard(rev).map_err(CoreError::from)?;
            wt.clean_fdq().map_err(CoreError::from)?;
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            self.primary
                .worktree_add(path, rev, true)
                .map_err(CoreError::from)?;
        }
        Ok(())
    }

    /// Move mainline from `base` to `new`: a push when origin arbitrates,
    /// else the local compare-and-swap. `None` = lost the race.
    pub fn publish(&self, new: &str, base: &str) -> Result<Option<Landed>, CoreError> {
        let local = format!("refs/heads/{}", self.branch);
        if self.push {
            match self.primary.push_cas(new, &self.branch) {
                Ok(()) => {
                    // the local branch follows when it can
                    let mut local_was = None;
                    if let Some(cur) = self.primary.verify_ref(&local).map_err(CoreError::from)?
                        && self
                            .primary
                            .is_ancestor(&cur, new)
                            .map_err(CoreError::from)?
                        && self.primary.update_ref_cas(&local, new, &cur).is_ok()
                    {
                        local_was = Some(cur);
                    }
                    Ok(Some(Landed {
                        commit: new.to_string(),
                        local_was,
                    }))
                }
                Err(GitError::LostRace { .. }) => Ok(None),
                Err(e) => Err(e.into()),
            }
        } else {
            match self.primary.update_ref_cas(&local, new, base) {
                Ok(()) => Ok(Some(Landed {
                    commit: new.to_string(),
                    local_was: Some(base.to_string()),
                })),
                Err(GitError::LostRace { .. }) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
    }

    /// The primary checkout follows a commit that landed — when the local
    /// branch moved with it. Call it while still holding the mainline turn.
    pub fn follow(&self, landed: &Landed) -> Result<(), CoreError> {
        match &landed.local_was {
            Some(was) => claim::resync_primary(&self.primary, &self.branch, was, &landed.commit),
            None => Ok(()),
        }
    }

    /// Publish one orchestrator-owned mainline commit by the claim
    /// discipline (CAS/push, never forced). `build` edits a transaction
    /// worktree at the head and reports whether there is anything to commit;
    /// `None` = nothing was. A lost race rebuilds on the new head — more
    /// patiently than a claim does: these commits have no competitor for
    /// what they change, only for the ref.
    pub fn publish_tx(
        &self,
        commit_message: &str,
        build: &dyn Fn(&Path) -> Result<bool, CoreError>,
    ) -> Result<Option<String>, CoreError> {
        self.layout.ensure_scratch()?;
        let _turn = claim::tx_lock();
        let tx = self.layout.scratch.join(format!(
            "tx-{}-orch{}",
            std::process::id(),
            UtcTime::now().0
        ));
        let result = (|| {
            for attempt in 0..6u64 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                }
                let base = self.head()?;
                self.checkout_at(&tx, &base)?;
                if !build(&tx)? {
                    return Ok(None);
                }
                let txgit = self.primary.at(&tx);
                txgit.add_all().map_err(CoreError::from)?;
                txgit.commit(commit_message).map_err(CoreError::from)?;
                let new = txgit.rev_parse("HEAD").map_err(CoreError::from)?;
                if let Some(landed) = self.publish(&new, &base)? {
                    self.follow(&landed)?;
                    return Ok(Some(new));
                }
            }
            Err(CoreError::refusal(
                5,
                format!("error: lost publish race for `{commit_message}` after retries"),
            ))
        })();
        if tx.exists() {
            let _ = self.primary.worktree_remove(&tx);
        }
        result
    }

    /// Put right what the audit can put right mechanically, as one mainline
    /// commit: lapsed leases are released (`fix_leases`), and Built or
    /// Verified units whose recorded trace no longer matches their inputs
    /// are marked Stale with the cause (`apply_stale`) — which is what makes
    /// a spec change that lands mid-run rebuild what it touched. Everything
    /// else the audit found is returned in `other`.
    pub fn heal(&self, fix_leases: bool, apply_stale: bool) -> Result<Healed, CoreError> {
        let healed = std::cell::RefCell::new(Healed::default());
        let message = match (fix_leases, apply_stale) {
            (true, true) => "sdd: audit --fix-leases --apply-stale",
            (false, true) => "sdd: audit --apply-stale",
            _ => "sdd: audit --fix-leases",
        };
        let commit = self.publish_tx(message, &|tx| {
            let inst = self.sdd_in(tx)?;
            let (findings, edits) = audit::run(
                &inst,
                tx,
                &cmd::audit_options(
                    audit::Options {
                        skip_rotten: true,
                        fix_leases,
                        apply_stale,
                        ..Default::default()
                    },
                    tx,
                    &inst,
                ),
                &ShRunner,
                UtcTime::now(),
            )?;
            let mut h = Healed::default();
            for edit in &edits {
                match edit {
                    audit::StateEdit::FixLease { unit, .. } => h.leases.push(unit.clone()),
                    audit::StateEdit::ApplyStale { unit, cause } => {
                        h.stale.push((unit.clone(), cause.clone()))
                    }
                }
            }
            let fixed = |f: &audit::Finding| {
                (f.check == 7 && h.leases.contains(&f.subject))
                    || (f.check == 5 && h.stale.iter().any(|(u, _)| u == &f.subject))
            };
            h.other = findings.into_iter().filter(|f| !fixed(f)).collect();
            let changed = !edits.is_empty();
            if changed {
                cmd::audit::apply_edits(&inst, &edits)?;
                let fresh = Instance::load(inst.handle())?;
                ledger::regen(&fresh, &UtcTime::now().format_date())?;
            }
            *healed.borrow_mut() = h;
            Ok(changed)
        })?;
        let mut healed = healed.into_inner();
        healed.commit = commit;
        Ok(healed)
    }

    /// Land a `Q-###` Decision-Log entry as a mainline commit (a worker
    /// reports the question; the Decision Log is outside its write-set).
    /// The entry says `open`, as the schema's own entries do: a question is
    /// open unless marked resolved, and one a run has just landed is not to
    /// be read as anything else by a tool that looks for the word.
    pub fn land_q_entry(
        &self,
        q: &str,
        unit: &str,
        holder: &str,
        notes: &str,
    ) -> Result<(), CoreError> {
        let notes = notes.replace('\n', " ");
        self.publish_tx(&format!("sdd: record {q} (blocked {unit})"), &|tx| {
            let inst = self.sdd_in(tx)?;
            let entry =
                questions::landed_entry(q, &UtcTime::now().format_date(), holder, unit, &notes);
            match inst.shape {
                model::Shape::Bundle => {
                    let path = inst.root.join("decisions.md");
                    let text = std::fs::read_to_string(&path)
                        .unwrap_or_else(|_| "# Decisions & Open Questions\n".to_string());
                    if text.contains(&format!("**{q}**")) {
                        return Ok(false); // already landed
                    }
                    model::write_text(&path, &questions::land(&text, &entry))?;
                }
                model::Shape::Single => {
                    let text = model::read_text(&inst.file)?;
                    if text.contains(&format!("**{q}**")) {
                        return Ok(false);
                    }
                    let new = match model::section_span(&text, "Decisions & Open Questions") {
                        Some((s, e)) => format!(
                            "{}{}{}",
                            &text[..s],
                            questions::land(&text[s..e], &entry),
                            &text[e..]
                        ),
                        None => format!(
                            "{}\n\n## Decisions & Open Questions\n\n{entry}",
                            text.trim_end_matches('\n')
                        ),
                    };
                    model::write_text(&inst.file, &new)?;
                }
            }
            Ok(true)
        })?;
        Ok(())
    }
}

/// Append one journal event at the next sequence number.
pub fn append_event(inst: &Instance, mut event: journal::Event) -> Result<(), CoreError> {
    let handle = inst.handle();
    let (events, _) = journal::read(&handle)?;
    event.seq = events.last().map(|e| e.seq).unwrap_or(0) + 1;
    if event.at.is_empty() {
        event.at = UtcTime::now().format();
    }
    if event.sdd.is_empty() {
        event.sdd = inst.meta.get_str_or("version", "0.0.0").to_string();
    }
    journal::append_line(&handle, &journal::render_line(&event))?;
    Ok(())
}

/// The unit's front matter after a release: Ready, no holder, no expiry.
pub fn released_meta(inner: &str) -> String {
    let b = yamlish::set_scalar_in_place(inner, "status", "Ready");
    let b = yamlish::set_scalar_in_place(&b, "claimed_by", "—");
    yamlish::set_scalar_in_place(&b, "claim_expires", "—")
}
