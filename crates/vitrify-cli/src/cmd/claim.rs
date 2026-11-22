//! `vitrify claim|renew|release` — atomic lease transactions.
//! Each operation is one mainline commit (front-matter edit, journal event,
//! ledger regen) built in the ephemeral tx worktree `.vitrify/tx-<pid>` and
//! published atomically by the C-002 CAS/push discipline. Never force.

use std::path::{Path, PathBuf};

use vitrify_core::claims::{self, Op};
use vitrify_core::config::Overrides;
use vitrify_core::graph::{self, Node};
use vitrify_core::journal;
use vitrify_core::model::{self, Instance, UtcTime};
use vitrify_core::{CoreError, ledger, yamlish};

use crate::cli::ClaimArgs;
use crate::git::{Git, GitError, resolve_mainline, use_push};
use crate::out::{J, diag, emit};

/// A published transaction's outcome (the success JSON's fields).
#[derive(Debug, Clone)]
pub struct ClaimOutcome {
    pub unit: String,
    pub op: Op,
    pub holder: String,
    pub claim_seq: i64,
    pub claim_expires: Option<String>,
    pub commit: String,
    pub mainline: String,
}

/// Monotonic discriminator so in-process transactions never share a tx
/// worktree.
static TX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One mainline transaction at a time within this process. The orchestrator
/// claims, renews, releases and integrates from several threads; taking
/// turns keeps them from racing each other for the mainline ref and for the
/// primary checkout's index. Other processes are arbitrated by the CAS/push.
static MAINLINE_TX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold this across the build-and-publish of one mainline commit. Not
/// re-entrant: [`execute`] takes it itself.
pub fn tx_lock() -> std::sync::MutexGuard<'static, ()> {
    MAINLINE_TX.lock().unwrap_or_else(|e| e.into_inner())
}

/// Under `vitrify run` every published commit moves the primary checkout;
/// one note per commit would bury the run's own output.
static QUIET_RESYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WARNED_BEHIND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Report the primary checkout's fast-forwards only when they stop working.
pub fn quiet_resync(on: bool) {
    QUIET_RESYNC.store(on, std::sync::atomic::Ordering::SeqCst);
}

pub fn run(op: Op, args: &ClaimArgs) -> Result<u8, CoreError> {
    let outcome = execute(op, args)?;
    let obj = J::Obj(vec![
        ("unit".into(), J::s(outcome.unit.clone())),
        ("op".into(), J::s(op.as_str())),
        ("holder".into(), J::s(outcome.holder.clone())),
        ("claim_seq".into(), J::Int(outcome.claim_seq)),
        (
            "claim_expires".into(),
            J::opt_str(outcome.claim_expires.clone()),
        ),
        ("commit".into(), J::s(outcome.commit.clone())),
        ("mainline".into(), J::s(outcome.mainline.clone())),
    ]);
    emit(&obj.dumps_default());
    Ok(0)
}

/// What a caller that runs many transactions decides once and holds every
/// one of them to — `vitrify run` does: how commits are published (a run
/// that began by pushing must never fall back to moving the local branch
/// because one fetch failed: that would put a claim on a mainline nobody
/// else sees), and which SDD, by its path under the repository root.
pub struct Pinned<'a> {
    pub push: bool,
    pub sdd_rel: &'a Path,
}

/// Execute the transaction without printing.
pub fn execute(op: Op, args: &ClaimArgs) -> Result<ClaimOutcome, CoreError> {
    execute_with(op, args, None)
}

/// [`execute`] for the orchestrator: publish mode and SDD as `pinned` says,
/// whatever `args.sdd` and the state of the remote suggest.
pub fn execute_pinned(
    op: Op,
    args: &ClaimArgs,
    pinned: &Pinned<'_>,
) -> Result<ClaimOutcome, CoreError> {
    execute_with(op, args, Some(pinned))
}

fn execute_with(
    op: Op,
    args: &ClaimArgs,
    pinned: Option<&Pinned<'_>>,
) -> Result<ClaimOutcome, CoreError> {
    let here = Git::new(super::cwd());
    let primary_path = here.primary_root().map_err(CoreError::from)?;
    let primary = Git::new(&primary_path);

    let cfg = super::load_config(&Overrides::default())?;
    let mainline = resolve_mainline(&primary, args.mainline.as_deref(), &cfg.mainline)
        .map_err(CoreError::from)?;

    let scratch = crate::layout::ensure_scratch(&primary_path)?;

    let _turn = tx_lock();
    let push = match pinned {
        Some(p) => p.push,
        None => use_push(&primary, &mainline).map_err(CoreError::from)?,
    };
    let n = TX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tx = scratch.join(format!("tx-{}-{n}", std::process::id()));

    let result = attempt_loop(
        &primary,
        &tx,
        push,
        &mainline,
        op,
        args,
        pinned.map(|p| p.sdd_rel),
    );

    if tx.exists() {
        let _ = primary.worktree_remove(&tx);
    }
    result
}

fn attempt_loop(
    primary: &Git,
    tx: &Path,
    push: bool,
    mainline: &str,
    op: Op,
    args: &ClaimArgs,
    sdd_rel: Option<&Path>,
) -> Result<ClaimOutcome, CoreError> {
    let mut lost_race_before = false;
    for attempt in 0..3 {
        // -- sync: the head this transaction builds on
        let base = if push {
            let _ = primary.fetch_origin(mainline);
            primary
                .rev_parse(&format!("refs/remotes/origin/{mainline}"))
                .map_err(CoreError::from)?
        } else {
            primary
                .rev_parse(&format!("refs/heads/{mainline}"))
                .map_err(CoreError::from)?
        };
        if tx.exists() {
            let txgit = primary.at(tx);
            txgit.reset_hard(&base).map_err(CoreError::from)?;
            txgit.clean_fdq().map_err(CoreError::from)?;
        } else {
            primary
                .worktree_add(tx, &base, true)
                .map_err(CoreError::from)?;
        }

        // -- preconditions at this head
        let inst = load_tx_instance(tx, args.sdd.as_deref(), sdd_rel, &primary.dir)?;
        let now = UtcTime::now();
        // a pin in depends_on waits on units of other SDDs: their statuses
        // are read in the same checkout, at the same base
        let unit_graph = if super::reaches_out(&inst) {
            Some(super::repo_graph(tx, Some(&inst))?)
        } else {
            None
        };
        let node = Node::new(graph::slug_of(&inst), args.unit.as_str());
        let no_graph = |_: &str| None;
        let resolve: model::Resolve = match &unit_graph {
            Some(g) => &g.resolver(&node),
            None => &no_graph,
        };
        let plan = claims::plan_with(
            &inst,
            op,
            &args.unit,
            &args.holder,
            args.lease_minutes,
            now,
            lost_race_before,
            resolve,
        )?;

        // -- build the commit: front matter + journal + ledger
        let unit = inst.unit(&args.unit).unwrap().clone();
        let handle = inst.handle();
        model::rewrite_unit_meta(&handle, &unit, |inner| {
            let mut block = inner.to_string();
            for (k, v) in &plan.edits {
                block = yamlish::set_scalar_in_place(&block, k, v);
            }
            block
        })?;
        let (events, _) = journal::read(&handle)?;
        let mut event = plan.event.clone();
        event.seq = events.last().map(|e| e.seq).unwrap_or(0) + 1;
        event.commit = Some(base[..7.min(base.len())].to_string());
        journal::append_line(&handle, &journal::render_line(&event))?;
        let fresh = Instance::load(handle.clone())?;
        ledger::regen(&fresh, &now.format_date())?;

        let txgit = primary.at(tx);
        txgit.add_all().map_err(CoreError::from)?;
        txgit
            .commit(&plan.commit_message)
            .map_err(CoreError::from)?;
        let new = txgit.rev_parse("HEAD").map_err(CoreError::from)?;

        // -- publish atomically
        let won = publish_commit(primary, push, mainline, &new, &base)?;

        if won {
            return Ok(ClaimOutcome {
                unit: args.unit.clone(),
                op,
                holder: args.holder.clone(),
                claim_seq: plan.claim_seq,
                claim_expires: plan.claim_expires.clone(),
                commit: new,
                mainline: mainline.to_string(),
            });
        }
        lost_race_before = true;
        diag(&format!(
            "note: lost publish race on {mainline} (attempt {}); resyncing",
            attempt + 1
        ));
    }
    Err(CoreError::refusal(
        5,
        format!("error: lost race for {} after retries", args.unit),
    ))
}

/// Move mainline from `base` to `new` — a push when origin arbitrates, else
/// the local compare-and-swap — and bring the primary checkout along.
/// `false` = lost the race.
fn publish_commit(
    primary: &Git,
    push: bool,
    mainline: &str,
    new: &str,
    base: &str,
) -> Result<bool, CoreError> {
    if push {
        // Push from the primary checkout: the commit lives in the shared
        // object store.
        match primary.push_cas(new, mainline) {
            Ok(()) => {
                if let Some(cur) = primary
                    .verify_ref(&format!("refs/heads/{mainline}"))
                    .map_err(CoreError::from)?
                {
                    if primary.is_ancestor(&cur, new).map_err(CoreError::from)? {
                        // the checkout follows a branch that moved, and only
                        // then
                        if primary
                            .update_ref_cas(&format!("refs/heads/{mainline}"), new, &cur)
                            .is_ok()
                        {
                            resync_primary(primary, mainline, &cur, new)?;
                        }
                    } else {
                        diag(&format!(
                            "warning: local {mainline} diverged from origin; not updating the local branch"
                        ));
                    }
                }
                Ok(true)
            }
            Err(GitError::LostRace { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    } else {
        match primary.update_ref_cas(&format!("refs/heads/{mainline}"), new, base) {
            Ok(()) => {
                resync_primary(primary, mainline, base, new)?;
                Ok(true)
            }
            Err(GitError::LostRace { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

/// One unit to claim in a [`claim_many`] transaction.
#[derive(Debug, Clone)]
pub struct BatchClaim {
    /// The unit's SDD, as a path from the repository root.
    pub sdd_rel: PathBuf,
    pub unit: String,
    pub holder: String,
}

/// Claim several units — of one SDD or of several — in one mainline
/// transaction: one commit, one compare-and-swap or push. Each unit's
/// preconditions are checked at the transaction's base, as a claim of its
/// own would check them; a unit that is no longer claimable there is left
/// out (its result says why), the rest are claimed together. Each claimed
/// unit gets its own front-matter edits, its own fencing token and its own
/// `claim` journal event, exactly as a claim of its own would. A lost race
/// resyncs and tries again with what is still claimable. Results are
/// index-aligned with `claims`; `Err` only for a failure that is no unit's.
pub fn claim_many(
    primary: &Git,
    push: bool,
    mainline: &str,
    lease_minutes: i64,
    claims: &[BatchClaim],
) -> Result<Vec<Result<ClaimOutcome, CoreError>>, CoreError> {
    let mut out: Vec<Option<Result<ClaimOutcome, CoreError>>> =
        claims.iter().map(|_| None).collect();
    if claims.is_empty() {
        return Ok(Vec::new());
    }
    let scratch = crate::layout::ensure_scratch(&primary.dir)?;
    let _turn = tx_lock();
    let n = TX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tx = scratch.join(format!("tx-{}-{n}", std::process::id()));
    let result = (|| -> Result<(), CoreError> {
        let mut lost_race_before = false;
        for attempt in 0..3 {
            let base = if push {
                let _ = primary.fetch_origin(mainline);
                primary
                    .rev_parse(&format!("refs/remotes/origin/{mainline}"))
                    .map_err(CoreError::from)?
            } else {
                primary
                    .rev_parse(&format!("refs/heads/{mainline}"))
                    .map_err(CoreError::from)?
            };
            if tx.exists() {
                let txgit = primary.at(&tx);
                txgit.reset_hard(&base).map_err(CoreError::from)?;
                txgit.clean_fdq().map_err(CoreError::from)?;
            } else {
                primary
                    .worktree_add(&tx, &base, true)
                    .map_err(CoreError::from)?;
            }
            let now = UtcTime::now();
            let mut planned: Vec<(usize, claims::TxPlan, String)> = Vec::new();
            for (i, c) in claims.iter().enumerate() {
                if out[i].is_some() {
                    continue; // judged already: left out for good
                }
                // each unit read afresh: an earlier one may share its file
                let inst = load_tx_instance(&tx, None, Some(&c.sdd_rel), &primary.dir)?;
                let unit_graph = if super::reaches_out(&inst) {
                    Some(super::repo_graph(&tx, Some(&inst))?)
                } else {
                    None
                };
                let node = Node::new(graph::slug_of(&inst), c.unit.as_str());
                let no_graph = |_: &str| None;
                let resolve: model::Resolve = match &unit_graph {
                    Some(g) => &g.resolver(&node),
                    None => &no_graph,
                };
                let plan = match claims::plan_with(
                    &inst,
                    Op::Claim,
                    &c.unit,
                    &c.holder,
                    lease_minutes,
                    now,
                    lost_race_before,
                    resolve,
                ) {
                    Ok(plan) => plan,
                    Err(e) if e.exit_code() == 4 => {
                        // no longer claimable at this base
                        out[i] = Some(Err(e));
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let Some(unit) = inst.unit(&c.unit).cloned() else {
                    continue;
                };
                let handle = inst.handle();
                model::rewrite_unit_meta(&handle, &unit, |inner| {
                    let mut block = inner.to_string();
                    for (k, v) in &plan.edits {
                        block = yamlish::set_scalar_in_place(&block, k, v);
                    }
                    block
                })?;
                let (events, _) = journal::read(&handle)?;
                let mut event = plan.event.clone();
                event.seq = events.last().map(|e| e.seq).unwrap_or(0) + 1;
                event.commit = Some(base[..7.min(base.len())].to_string());
                journal::append_line(&handle, &journal::render_line(&event))?;
                let fresh = Instance::load(handle.clone())?;
                ledger::regen(&fresh, &now.format_date())?;
                planned.push((i, plan, graph::slug_of(&fresh).to_string()));
            }
            if planned.is_empty() {
                return Ok(());
            }
            // one unit: the message a claim of its own makes; several of one
            // SDD: their ids; of several SDDs: each with its slug
            let one_sdd = planned.iter().all(|(_, _, s)| *s == planned[0].2);
            let message = match planned.as_slice() {
                [(_, plan, _)] => plan.commit_message.clone(),
                many => format!(
                    "sdd: claim {}",
                    many.iter()
                        .map(|(i, p, slug)| if one_sdd {
                            format!("{} (c{})", claims[*i].unit, p.claim_seq)
                        } else {
                            format!("{slug} {} (c{})", claims[*i].unit, p.claim_seq)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let txgit = primary.at(&tx);
            txgit.add_all().map_err(CoreError::from)?;
            txgit.commit(&message).map_err(CoreError::from)?;
            let new = txgit.rev_parse("HEAD").map_err(CoreError::from)?;
            if publish_commit(primary, push, mainline, &new, &base)? {
                for (i, plan, _) in planned {
                    out[i] = Some(Ok(ClaimOutcome {
                        unit: claims[i].unit.clone(),
                        op: Op::Claim,
                        holder: claims[i].holder.clone(),
                        claim_seq: plan.claim_seq,
                        claim_expires: plan.claim_expires.clone(),
                        commit: new.clone(),
                        mainline: mainline.to_string(),
                    }));
                }
                return Ok(());
            }
            lost_race_before = true;
            diag(&format!(
                "note: lost publish race on {mainline} (attempt {}); resyncing",
                attempt + 1
            ));
        }
        Ok(())
    })();
    if tx.exists() {
        let _ = primary.worktree_remove(&tx);
    }
    result?;
    Ok(out
        .into_iter()
        .zip(claims)
        .map(|(r, c)| {
            r.unwrap_or_else(|| {
                Err(CoreError::refusal(
                    5,
                    format!("error: lost race for {} after retries", c.unit),
                ))
            })
        })
        .collect())
}

/// Load the instance inside the tx worktree. `--sdd` names the SDD as seen
/// from wherever the verb runs — the primary checkout or any worktree of it —
/// so its path relative to that checkout's root is what maps into the tx
/// tree. A path that lies in neither is refused: resolving it as given
/// would edit that directory in place instead of the transaction's copy.
/// `sdd_rel` (the orchestrator's pin) is that relative path already.
fn load_tx_instance(
    tx: &Path,
    sdd_arg: Option<&str>,
    sdd_rel: Option<&Path>,
    primary: &Path,
) -> Result<Instance, CoreError> {
    if let Some(rel) = sdd_rel {
        return match super::resolve_sdd_in(tx, rel) {
            Ok(handle) => Instance::load(handle),
            Err(CoreError::Refusal { code: 4, .. }) => Err(CoreError::refusal(
                4,
                format!(
                    "error: no SDD at {} at the mainline head — commit the SDD to mainline before claiming",
                    super::sdd_arg(rel)
                ),
            )),
            Err(e) => Err(e),
        };
    }
    let remapped: Option<String> = match sdd_arg {
        None => None,
        Some(arg) => {
            let mut p = PathBuf::from(arg);
            if !p.is_absolute() {
                p = super::cwd().join(&p);
            }
            let p = p.canonicalize().unwrap_or(p);
            // the checkout the verb runs in first: it may itself lie inside
            // the primary checkout's directory
            let roots = [super::repo_root(), primary.to_path_buf()];
            let rel = roots.iter().find_map(|root| {
                let root = root.canonicalize().unwrap_or_else(|_| root.clone());
                p.strip_prefix(&root).ok().map(Path::to_path_buf)
            });
            match rel {
                Some(rel) => Some(tx.join(rel).to_string_lossy().to_string()),
                None => {
                    return Err(CoreError::refusal(
                        4,
                        format!(
                            "error: --sdd {arg} is not inside this checkout — name the SDD by its path in the repository"
                        ),
                    ));
                }
            }
        }
    };
    match super::resolve_sdd(tx, remapped.as_deref(), tx) {
        Ok(handle) => Instance::load(handle),
        // several SDDs and no --sdd: say so, naming each by the path `--sdd`
        // takes (the transaction's own location means nothing to the caller)
        Err(CoreError::Refusal { code: 4, message })
            if message.starts_with("error: multiple SDDs found") =>
        {
            Err(CoreError::refusal(
                4,
                message.replace(&format!("{}/", tx.display()), ""),
            ))
        }
        Err(CoreError::Refusal { code: 4, .. }) => Err(CoreError::refusal(
            4,
            match sdd_arg {
                Some(arg) => format!(
                    "error: --sdd {arg} is not an SDD root at the mainline head — commit the SDD to mainline before claiming"
                ),
                None => "error: no SDD found at the mainline head — commit the SDD to mainline before claiming".to_string(),
            },
        )),
        Err(e) => Err(e),
    }
}

/// Where a primary checkout that could not follow its branch says so —
/// `<the commit its files hold> <the commit it was heading for>` — for the
/// next transaction, of this process or any later one, to catch it up from.
fn behind_file(primary: &Git) -> PathBuf {
    primary.dir.join(".vitrify").join("primary-behind")
}

fn read_behind(primary: &Git) -> Option<(String, String)> {
    let text = std::fs::read_to_string(behind_file(primary)).ok()?;
    let mut words = text.split_whitespace();
    let from = words.next()?.to_string();
    let target = words.next()?.to_string();
    Some((from, target))
}

fn write_behind(primary: &Git, from: &str, target: &str) {
    if crate::layout::ensure_scratch(&primary.dir).is_ok() {
        let _ = std::fs::write(behind_file(primary), format!("{from} {target}\n"));
    }
}

fn clear_behind(primary: &Git) {
    let _ = std::fs::remove_file(behind_file(primary));
}

/// Whether the checkout's index still holds `from`'s version of something
/// that differs at `head` — that is, has not been brought up to date since
/// it was left at `from`.
fn still_behind(primary: &Git, from: &str, head: &str) -> bool {
    let (Ok(moved), Ok(staged)) = (
        primary.changed_paths(from, head),
        primary.index_differs_from(head),
    ) else {
        return false; // `from` is no longer a commit here: nothing to catch up from
    };
    let staged: std::collections::HashSet<String> = staged.into_iter().collect();
    moved.iter().any(|p| staged.contains(p))
}

fn sha7(sha: &str) -> &str {
    &sha[..7.min(sha.len())]
}

fn sha12(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

/// What git said, without the plumbing around it.
fn git_reason(e: &GitError) -> String {
    match e {
        GitError::Failed { stderr, .. } if !stderr.trim().is_empty() => stderr
            .lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches("error: ")
                    .trim_start_matches("fatal: ")
            })
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

/// The command that brings a checkout holding `from` to `new` by hand —
/// what [`follow`] itself runs.
fn by_hand(from: &str, new: &str) -> String {
    format!(
        "git update-index -q --refresh && git read-tree -m -u {} {}",
        sha12(from),
        sha12(new)
    )
}

enum Follow {
    /// Another branch is checked out there: nothing of it moved.
    NotCheckedOut,
    Followed,
    /// It could not follow; the warning to print.
    Behind(String),
}

/// Bring the primary checkout's index and files to `new`, when it has
/// `mainline` checked out. `old` is what the branch pointed at before it
/// was moved — what the files hold, unless an earlier move could not be
/// followed (then [`behind_file`] says which commit they do hold).
///
/// The move is a fast-forward of index and tree (`read-tree -m -u`): what
/// the published commits changed is updated and the user's own uncommitted
/// changes ride along. If one of those — or an untracked file — is in the
/// way, nothing at all is touched: the checkout stays where it was, a marker
/// records where, and the next move catches up from there. Never `reset
/// --hard`, which would overwrite what is in the way.
fn follow(primary: &Git, mainline: &str, old: &str, new: &str) -> Follow {
    let by_hand = |from: &str| by_hand(from, new);
    match primary.current_branch() {
        Ok(Some(branch)) if branch == mainline => {}
        Ok(_) => return Follow::NotCheckedOut,
        Err(e) => {
            return Follow::Behind(format!(
                "warning: could not inspect the primary checkout ({}); if it has {mainline} checked out it is now behind — bring it up to date with: {}",
                git_reason(&e),
                by_hand(old)
            ));
        }
    }
    let mut from = old.to_string();
    if let Some((left_at, _)) = read_behind(primary) {
        if primary.clean_against(old).unwrap_or(false) || !still_behind(primary, &left_at, old) {
            clear_behind(primary); // brought up to date by hand since
        } else {
            from = left_at;
        }
    }
    if from == new {
        return Follow::Followed;
    }
    match primary.fast_forward_tree(&from, new) {
        Ok(()) => {
            clear_behind(primary);
            Follow::Followed
        }
        Err(e) => {
            write_behind(primary, &from, new);
            Follow::Behind(format!(
                "warning: the primary checkout could not follow {mainline} to {}: {}\n  It still holds {} and is behind {mainline} — commit nothing from it until it has caught up.\n  Clear what is in the way and it catches up at the next commit vitrify publishes, or now with: {}",
                sha7(new),
                git_reason(&e),
                sha7(&from),
                by_hand(&from)
            ))
        }
    }
}

/// After a commit moved `mainline` from `old` to `new`: the primary checkout
/// follows ([`follow`]). Never fatal — the commit is already published.
pub fn resync_primary(
    primary: &Git,
    mainline: &str,
    old: &str,
    new: &str,
) -> Result<(), CoreError> {
    use std::sync::atomic::Ordering;
    let quiet = QUIET_RESYNC.load(Ordering::SeqCst);
    match follow(primary, mainline, old, new) {
        Follow::NotCheckedOut => {}
        Follow::Followed => {
            WARNED_BEHIND.store(false, Ordering::SeqCst);
            if !quiet {
                diag(&format!(
                    "note: primary checkout fast-forwarded to {}",
                    sha7(new)
                ));
            }
        }
        // under `vitrify run`: said once, until it has caught up again
        Follow::Behind(warning) => {
            if !(quiet && WARNED_BEHIND.swap(true, Ordering::SeqCst)) {
                diag(&warning);
            }
        }
    }
    Ok(())
}

/// A primary checkout that an earlier commit left behind: try again now.
/// Returns the warning when it still cannot follow. Like every move of the
/// checkout, this is one mainline writer's turn ([`tx_lock`]).
pub fn catch_up_primary(primary: &Git, mainline: &str) -> Option<String> {
    read_behind(primary)?;
    let head = primary.rev_parse(&format!("refs/heads/{mainline}")).ok()?;
    match follow(primary, mainline, &head, &head) {
        Follow::Behind(warning) => Some(warning),
        Follow::Followed | Follow::NotCheckedOut => None,
    }
}

/// What [`catch_up_primary`] would have to do, without doing it.
pub fn behind_note(primary: &Git, mainline: &str) -> Option<String> {
    let (left_at, _) = read_behind(primary)?;
    if !primary
        .current_branch()
        .ok()?
        .is_some_and(|b| b == mainline)
    {
        return None;
    }
    let head = primary.rev_parse(&format!("refs/heads/{mainline}")).ok()?;
    if primary.clean_against(&head).unwrap_or(false) || !still_behind(primary, &left_at, &head) {
        return None;
    }
    Some(format!(
        "note: the primary checkout is behind {mainline} (it holds {}); a run brings it up to date before it starts — by hand: {}",
        sha7(&left_at),
        by_hand(&left_at, &head)
    ))
}
