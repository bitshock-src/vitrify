//! `vitrify new tdd|sdd [--bundle|--single] [NAME]` — materialize instance
//! templates into the current directory, refusing to overwrite (exit 2).
//! Prints the created path on stdout and the next step on stderr.

use std::path::Path;

use vitrify_core::CoreError;

use crate::cli::NewKind;
use crate::home;
use crate::out::{diag, emit};

pub fn run(kind: NewKind, single: bool, name: Option<&str>) -> Result<u8, CoreError> {
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    let version = super::brief::VERSION;
    let cwd = super::cwd();
    let resolve = |rel: &str| home::resolve(&home_dir, version, rel).map(|(b, _)| b);

    let refuse_existing = |p: &Path| -> Result<(), CoreError> {
        if p.exists() {
            Err(CoreError::refusal(
                2,
                format!("error: refusing to overwrite existing {}", p.display()),
            ))
        } else {
            Ok(())
        }
    };
    let write = |p: &Path, body: &str| -> Result<(), CoreError> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(p, body).map_err(|source| CoreError::Io {
            path: p.to_path_buf(),
            source,
        })
    };

    match kind {
        NewKind::Tdd => {
            let name = name.unwrap_or("tdd");
            let target = cwd.join(format!("{name}.md"));
            refuse_existing(&target)?;
            write(&target, &resolve("templates/tdd-template.md")?)?;
            diag(&format!("materialized {}", target.display()));
            emit(&target.display().to_string());
            diag(&format!(
                "next: fill {} per vitrify schema show --tdd",
                super::shown(&cwd, &target)
            ));
        }
        NewKind::Sdd if single => {
            let name = name.unwrap_or("scope");
            let target = cwd.join(format!("{name}.sdd.md"));
            refuse_existing(&target)?;
            write(&target, &resolve("templates/sdd-template-single.md")?)?;
            diag(&format!("materialized {}", target.display()));
            emit(&target.display().to_string());
            sdd_next(&super::shown(&cwd, &target));
        }
        NewKind::Sdd => {
            // bundle is the default shape
            let name = name.unwrap_or("scope");
            let dir = cwd.join(name);
            refuse_existing(&dir)?;
            for (rel, out) in [
                ("templates/sdd-template-bundle/sdd.md", "sdd.md"),
                (
                    "templates/sdd-template-bundle/units/U-001-foundation.md",
                    "units/U-001-foundation.md",
                ),
                (
                    "templates/sdd-template-bundle/contracts/C-001-example.md",
                    "contracts/C-001-example.md",
                ),
                ("templates/sdd-template-bundle/changelog.md", "changelog.md"),
                ("templates/sdd-template-bundle/decisions.md", "decisions.md"),
            ] {
                write(&dir.join(out), &resolve(rel)?)?;
            }
            diag(&format!("materialized bundle {}", dir.display()));
            emit(&dir.display().to_string());
            sdd_next(&super::shown(&cwd, &dir));
        }
    }
    Ok(0)
}

/// The next step after a new SDD, `path` as seen from the current directory.
fn sdd_next(path: &str) {
    diag(&format!(
        "next: fill {path} per vitrify schema show, then run vitrify lint --sdd {path}"
    ));
}
