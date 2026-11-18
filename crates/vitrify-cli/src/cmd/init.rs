//! `vitrify init` — set this machine up: create `~/.vitrify/`, write
//! `config.toml` with every setting at its default and the shipped harness
//! role profiles, materialize the content (briefs, schemas, templates), then
//! probe git and the harness CLIs and record what works in the same file.
//!
//! The one command that stands the home up: every other command that
//! writes into it refuses until `config.toml` is there. `init` refuses an
//! existing `config.toml` — the file is the user's from then on, and
//! `vitrify probe` brings its probe record up to date.

use vitrify_core::CoreError;
use vitrify_core::content;

use crate::home;
use crate::out::{J, emit};

use super::probe::{self, CONFIG_LABEL, CheckLine};

pub fn run(json: bool) -> Result<u8, CoreError> {
    let Some(home_dir) = super::home_dir() else {
        return Err(CoreError::refusal(
            1,
            "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
        ));
    };
    let path = home::config_path(&home_dir);
    if path.exists() {
        return Err(CoreError::refusal(
            2,
            format!(
                "error: refusing to overwrite existing {CONFIG_LABEL} → vitrify probe brings its probe record up to date"
            ),
        ));
    }
    let version = super::brief::VERSION;

    std::fs::create_dir_all(&home_dir).map_err(|source| CoreError::Io {
        path: home_dir.clone(),
        source,
    })?;
    // the defaults first: a probe cut short leaves a working file behind,
    // which `vitrify probe` completes
    probe::write_config(&home_dir, content::CONFIG.body)?;

    let mut checks = vec![
        CheckLine::new("ok", format!("vitrify version {version}")),
        CheckLine::new(
            "ok",
            format!("created {CONFIG_LABEL} with every setting at its default"),
        ),
    ];
    if home_dir.join("harness.toml").exists() {
        checks.push(CheckLine::new(
            "note",
            format!(
                "~/.vitrify/harness.toml is not read: the role profiles are the [harness.<name>] tables of {CONFIG_LABEL}"
            ),
        ));
    }

    let probed = probe::probe_machine(&home_dir, version, content::CONFIG.body)?;
    let failed = probed.failed();
    let text = probe::updated_text(
        content::CONFIG.body,
        &probe::render_record(version, &probed),
    )?;
    probe::write_config(&home_dir, &text)?;
    checks.extend(
        probed
            .checks
            .iter()
            .map(|c| CheckLine::new(c.status, c.text.clone())),
    );
    checks.push(CheckLine::new(
        "ok",
        format!("recorded the probe in {CONFIG_LABEL}"),
    ));

    if json {
        let obj = probe::json_of(&checks, &probed, failed);
        emit(&J::Obj(obj).dumps_indent2());
    } else {
        for c in &checks {
            emit(&format!("{}: {}", c.status, c.text));
        }
    }
    Ok(if failed { 2 } else { 0 })
}
