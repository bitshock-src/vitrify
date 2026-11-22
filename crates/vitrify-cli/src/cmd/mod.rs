//! Command implementations and the shared invocation context.

pub mod audit;
pub mod brief;
pub mod change;
pub mod check;
pub mod claim;
pub mod init;
pub mod journal;
pub mod ledger;
pub mod lint;
pub mod locate;
pub mod new;
pub mod pin;
pub mod probe;
pub mod questions;
pub mod readset;
pub mod run;
pub mod run_status;
pub mod schema;
pub mod trace;

use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_core::CoreError;
use vitrify_core::audit as core_audit;
use vitrify_core::config::{self, Config, Overrides};
use vitrify_core::contract::{self, Peers};
use vitrify_core::graph::UnitGraph;
use vitrify_core::model::{self, Instance, SddHandle};

/// The governed files under the SDD's `scope_roots` — what git would commit
/// at `root` — or `None` outside a git work tree, where callers walk the
/// filesystem instead ([`core_audit::walk_scope`]).
pub fn governed_files(root: &Path, inst: &Instance) -> Option<Vec<String>> {
    crate::git::Git::new(root)
        .committable_files(&inst.meta.get_list("scope_roots"))
        .ok()
}

/// Every file git would commit at `root` — `None` outside a git work tree.
fn committable_files(root: &Path) -> Option<Vec<String>> {
    crate::git::Git::new(root)
        .committable_files(&[".".to_string()])
        .ok()
}

/// SDD resolution ([`model::resolve_sdd`]) whose repo-wide search covers
/// only what git would commit, so git's ignore rules decide where an SDD
/// can be found. Every verb resolves through here.
pub fn resolve_sdd(root: &Path, sdd_arg: Option<&str>, cwd: &Path) -> Result<SddHandle, CoreError> {
    if sdd_arg.is_some() {
        // an explicit --sdd never searches
        return model::resolve_sdd(root, sdd_arg, cwd);
    }
    model::resolve_sdd_with(root, None, cwd, committable_files(root).as_deref())
}

/// Every SDD in the repository ([`model::discover`]), searched the same way.
pub fn discover(root: &Path) -> Vec<SddHandle> {
    model::discover_with(root, committable_files(root).as_deref())
}

/// The SDDs of the repository at `root`, as the place an external
/// contract's authority is looked up in — found only if something asks.
pub fn peers(root: &Path) -> Peers<'_> {
    Peers::new(move || discover(root))
}

/// The SDDs of the repository at `root` that load, each with the error of
/// one that does not (a schema version this vitrify does not read), in
/// discovery order. `own`, loaded already, stands in for its discovered
/// copy, or joins them when discovery does not find it.
pub fn loaded(root: &Path, own: Option<&Instance>) -> (Vec<Instance>, Vec<(SddHandle, CoreError)>) {
    let mut insts: Vec<Instance> = Vec::new();
    let mut refused: Vec<(SddHandle, CoreError)> = Vec::new();
    for h in discover(root) {
        if let Some(own) = own
            && same_root(&h.root, &own.root)
        {
            insts.push(own.clone());
            continue;
        }
        match Instance::load(h.clone()) {
            Ok(inst) => insts.push(inst),
            Err(e) => refused.push((h, e)),
        }
    }
    if let Some(own) = own
        && !insts.iter().any(|i| same_root(&i.root, &own.root))
    {
        insts.push(own.clone());
    }
    (insts, refused)
}

fn same_root(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The repository's unit graph over every SDD that loads ([`loaded`]):
/// what pins in `depends_on` resolve against.
pub fn repo_graph(root: &Path, own: Option<&Instance>) -> Result<UnitGraph, CoreError> {
    UnitGraph::build(&loaded(root, own).0)
}

/// Whether a unit of `inst` names, in `depends_on`, anything but a unit of
/// its own SDD — so its claimability needs the repository's graph.
pub fn reaches_out(inst: &Instance) -> bool {
    inst.units.iter().any(|u| {
        u.meta
            .get_list("depends_on")
            .iter()
            .any(|d| inst.unit(d).is_none())
    })
}

/// The SDDs a verb acts on: the one `--sdd` names, or the repository's only
/// one; with `--all`, every SDD of the repository in discovery order. None
/// found is exit 4 either way.
pub fn targets(
    root: &Path,
    sdd_arg: Option<&str>,
    all: bool,
    cwd: &Path,
) -> Result<Vec<SddHandle>, CoreError> {
    if !all {
        return Ok(vec![resolve_sdd(root, sdd_arg, cwd)?]);
    }
    let found = discover(root);
    if found.is_empty() {
        return Err(CoreError::refusal(4, model::NO_SDD));
    }
    Ok(found)
}

/// `path` as output names it: from the repository root when it lies under
/// it, as it is otherwise.
pub fn shown(root: &Path, path: &Path) -> String {
    let rel = path
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let (root, path) = (root.canonicalize().ok()?, path.canonicalize().ok()?);
            path.strip_prefix(&root).ok().map(Path::to_path_buf)
        });
    match rel {
        Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => path.display().to_string(),
    }
}

/// How one SDD is named where a verb reports on several: its slug, or —
/// when it has none — where it lies.
pub fn label(sdd: &SddHandle, root: &Path) -> String {
    match contract::slug(sdd) {
        "" => shown(root, &sdd.root),
        slug => slug.to_string(),
    }
}

/// One verb run over several SDDs: `one` is run for each — an error from
/// one is reported under its name and the rest still run — and the exit
/// code is the worst of theirs: an unexpected error before any refusal or
/// finding.
pub fn each(
    root: &Path,
    sdds: &[SddHandle],
    mut one: impl FnMut(&SddHandle) -> Result<u8, CoreError>,
) -> u8 {
    let mut codes: Vec<u8> = Vec::new();
    for sdd in sdds {
        codes.push(match one(sdd) {
            Ok(code) => code,
            Err(e) => {
                let text = e.to_string();
                crate::out::diag(&format!(
                    "{}: {}",
                    label(sdd, root),
                    text.strip_prefix("error: ").unwrap_or(&text)
                ));
                e.exit_code()
            }
        });
    }
    if codes.contains(&1) {
        1
    } else {
        codes.into_iter().max().unwrap_or(0)
    }
}

/// An SDD's root relative to the root of the checkout it was resolved in —
/// the one name for it that holds in every checkout of the repository (the
/// primary, a unit worktree, the integration checkout, a transaction).
/// `None` when the SDD lies outside that checkout.
pub fn sdd_rel(checkout: &Path, sdd_root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = sdd_root.strip_prefix(checkout) {
        return Some(rel.to_path_buf());
    }
    let checkout = checkout.canonicalize().ok()?;
    let sdd_root = sdd_root.canonicalize().ok()?;
    sdd_root.strip_prefix(&checkout).ok().map(Path::to_path_buf)
}

/// Resolve the SDD at `rel` ([`sdd_rel`]) inside `checkout`. Never searches:
/// a repository with several SDDs resolves the same one everywhere.
pub fn resolve_sdd_in(checkout: &Path, rel: &Path) -> Result<SddHandle, CoreError> {
    let at = checkout.join(rel);
    model::resolve_sdd(checkout, Some(&at.to_string_lossy()), checkout)
}

/// `rel` as a `--sdd` argument that works from any checkout's root.
pub fn sdd_arg(rel: &Path) -> String {
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() { ".".to_string() } else { s }
}

/// `opts` with the governed-file listing filled in from git — every audit
/// call goes through here, so the orphan scan follows git's ignore rules.
pub fn audit_options(
    mut opts: core_audit::Options,
    root: &Path,
    inst: &Instance,
) -> core_audit::Options {
    opts.scope_files = governed_files(root, inst);
    opts
}

/// The repository containing the current working directory:
/// `git rev-parse --show-toplevel`, else cwd.
pub fn repo_root() -> PathBuf {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(o) = out
        && o.status.success()
        && let Ok(s) = String::from_utf8(o.stdout)
    {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    cwd()
}

pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Home-dir resolution: `$VITRIFY_HOME` if set, else `$HOME/.vitrify`.
pub fn home_dir() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("VITRIFY_HOME")
        && !h.is_empty()
    {
        return Some(PathBuf::from(h));
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".vitrify"))
}

/// Load configuration with its precedence: environment, the repository's
/// `.vitrify/config.toml`, `~/.vitrify/config.toml`, the default.
pub fn load_config(overrides: &Overrides) -> Result<Config, CoreError> {
    let env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k.starts_with("VITRIFY_"))
        .collect();
    config::load(Some(&repo_root()), home_dir().as_deref(), &env, overrides)
}
