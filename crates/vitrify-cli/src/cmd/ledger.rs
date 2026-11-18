//! `vitrify ledger regen|progress` — derived ledger output.
//! Regen message on stderr, progress line on stdout. `regen --check`
//! writes nothing: it compares the ledger on disk with a fresh derivation
//! and exits 3 when they differ.

use std::path::Path;

use vitrify_core::ledger::Check;
use vitrify_core::model::{Instance, SddHandle, UtcTime};
use vitrify_core::{CoreError, ledger};

use crate::out::{diag, emit};

/// Run `one` over the SDDs the flags name; with `--all` every line it
/// prints is given the SDD's name (`one` receives the prefix).
fn over(
    sdd: Option<&str>,
    all: bool,
    one: impl Fn(&Path, &Instance, &str) -> Result<u8, CoreError>,
) -> Result<u8, CoreError> {
    let root = super::repo_root();
    let sdds = super::targets(&root, sdd, all, &super::cwd())?;
    if !all {
        return one(&root, &Instance::load(sdds[0].clone())?, "");
    }
    Ok(super::each(&root, &sdds, |sdd: &SddHandle| {
        let under = format!("{}: ", super::label(sdd, &root));
        one(&root, &Instance::load(sdd.clone())?, &under)
    }))
}

/// `regen --check` for one SDD, printing nothing: `None` when the ledger on
/// disk is what a regeneration would write — or, unless `required`, when
/// there is none — else `(subject, cause)`, the subject the ledger's file
/// from the SDD's root.
pub fn mismatch(inst: &Instance, required: bool) -> Result<Option<(String, String)>, CoreError> {
    let today = UtcTime::now().format_date();
    let name = |path: &Path| {
        let rel = path
            .strip_prefix(&inst.root)
            .ok()
            .filter(|r| !r.as_os_str().is_empty())
            .or_else(|| path.file_name().map(Path::new))
            .unwrap_or(path);
        rel.to_string_lossy().replace('\\', "/")
    };
    Ok(match ledger::check(inst, &today)? {
        Check::Current(_) => None,
        Check::Missing(_) if !required => None,
        Check::Missing(path) => Some((name(&path), "holds no ledger".to_string())),
        Check::Differs {
            path,
            line,
            derived,
            found,
        } => Some((
            name(&path),
            format!(
                "line {line} differs from its derivation (derived `{derived}`, on disk `{found}`)"
            ),
        )),
    })
}

pub fn regen(sdd: Option<&str>, all: bool, check: bool) -> Result<u8, CoreError> {
    let today = UtcTime::now().format_date();
    over(sdd, all, |root, inst, under| {
        if check {
            return Ok(match ledger::check(inst, &today)? {
                Check::Current(path) => {
                    emit(&format!(
                        "{under}ledger current ({})",
                        super::shown(root, &path)
                    ));
                    0
                }
                Check::Missing(path) => {
                    emit(&format!(
                        "{under}ledger missing: {} holds none — run `vitrify ledger regen`",
                        super::shown(root, &path)
                    ));
                    3
                }
                Check::Differs {
                    path,
                    line,
                    derived,
                    found,
                } => {
                    emit(&format!(
                        "{under}ledger differs from its derivation at {}:{line} — run `vitrify ledger regen`",
                        super::shown(root, &path)
                    ));
                    emit(&format!("{under}  derived: {derived}"));
                    emit(&format!("{under}  on disk: {found}"));
                    3
                }
            });
        }
        let (_, msg) = ledger::regen(inst, &today)?;
        diag(&msg);
        emit(&format!("{under}{}", ledger::progress_line(&inst.units)));
        Ok(0)
    })
}

pub fn progress(sdd: Option<&str>, all: bool) -> Result<u8, CoreError> {
    over(sdd, all, |_, inst, under| {
        emit(&format!("{under}{}", ledger::progress_line(&inst.units)));
        Ok(0)
    })
}
