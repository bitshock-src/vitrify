//! `~/.vitrify` materialization and disk-wins resolution.
//!
//! Only `vitrify init` stands the home up: it writes `config.toml` and
//! materializes the content. Every other command first checks that it is
//! set up — `config.toml` is there — and otherwise refuses, naming
//! `vitrify init` ([`require_set_up`]); the dispatch does it for all of
//! them, and the seams that write into the home check again.
//!
//! The single resolution seam: `brief`, `new`, `schema`, and the
//! escalation template all resolve assets through here. A file on disk
//! overrides the embedded copy; a deleted file is restored on the next run;
//! an unmodified default follows the binary — it refreshes whenever the
//! embedded copy differs, an upgrade or a rebuild of the same version — and
//! overrides are left in place (`vitrify init` and `vitrify probe` report
//! them).

use std::path::{Path, PathBuf};

use vitrify_core::CoreError;
use vitrify_core::content::{self, sha256_hex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Served from the binary (no disk copy existed and one could not be written).
    Embedded,
    /// On disk, byte-identical to the shipped copy.
    Materialized,
    /// On disk, user-modified: disk wins.
    Override,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Embedded => "embedded",
            Origin::Materialized => "materialized",
            Origin::Override => "override",
        }
    }
}

/// One asset's home state, for `vitrify init` and `vitrify probe` to report.
#[derive(Debug, Clone)]
pub struct AssetState {
    pub rel_path: String,
    pub origin: Origin,
    /// Override forked from another shipped version (manifest's recorded
    /// shipped digest differs from the current embedded digest).
    pub stale_against_embedded: bool,
}

#[derive(Debug, Default)]
struct Manifest {
    vitrify_version: String,
    /// rel path → "sha256:<hex>" of the SHIPPED content last materialized.
    files: Vec<(String, String)>,
}

impl Manifest {
    fn digest_of(&self, rel: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(k, _)| k == rel)
            .map(|(_, v)| v.strip_prefix("sha256:").unwrap_or(v))
    }

    fn set(&mut self, rel: &str, hex: &str) {
        let value = format!("sha256:{hex}");
        match self.files.iter_mut().find(|(k, _)| k == rel) {
            Some(slot) => slot.1 = value,
            None => self.files.push((rel.to_string(), value)),
        }
    }
}

fn manifest_path(home: &Path) -> PathBuf {
    home.join(".manifest.toml")
}

/// `~/.vitrify/config.toml`: the machine's configuration, and the mark of a
/// home `vitrify init` has set up.
pub fn config_path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

/// The refusal every command that needs the home gives when it is not set
/// up: exit 4, naming the one command that sets it up.
pub fn not_set_up() -> CoreError {
    CoreError::refusal(
        4,
        "error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init",
    )
}

/// Refuse unless `vitrify init` has set the home up. Every command but
/// `init` passes through it, so none but `init` ever creates the home.
pub fn require_set_up(home: &Path) -> Result<(), CoreError> {
    if config_path(home).is_file() {
        Ok(())
    } else {
        Err(not_set_up())
    }
}

fn read_manifest(home: &Path) -> Manifest {
    let Ok(text) = std::fs::read_to_string(manifest_path(home)) else {
        return Manifest::default();
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return Manifest::default();
    };
    let mut m = Manifest {
        vitrify_version: table
            .get("vitrify_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        files: Vec::new(),
    };
    if let Some(files) = table.get("files").and_then(|v| v.as_table()) {
        for (k, v) in files {
            if let Some(s) = v.as_str() {
                m.files.push((k.clone(), s.to_string()));
            }
        }
    }
    m
}

fn write_manifest(home: &Path, m: &Manifest) -> Result<(), CoreError> {
    let mut text = String::from("schema = 1\n");
    text.push_str(&format!(
        "vitrify_version = \"{}\"\n\n[files]\n",
        m.vitrify_version
    ));
    let mut files = m.files.clone();
    files.sort();
    for (k, v) in &files {
        text.push_str(&format!("\"{k}\" = \"{v}\"\n"));
    }
    // atomic: temp + rename
    let tmp = home.join(".manifest.toml.tmp");
    std::fs::write(&tmp, &text).map_err(|source| CoreError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, manifest_path(home)).map_err(|source| CoreError::Io {
        path: manifest_path(home),
        source,
    })?;
    Ok(())
}

/// Ensure `~/.vitrify/` is materialized per the staleness rules; returns the
/// per-asset state for `init` and `probe` to report. Idempotent; reached by
/// every content consumer through [`resolve`], which refuses a home that is
/// not set up, and by `vitrify init`, which sets it up.
pub fn ensure_materialized(home: &Path, version: &str) -> Result<Vec<AssetState>, CoreError> {
    std::fs::create_dir_all(home).map_err(|source| CoreError::Io {
        path: home.to_path_buf(),
        source,
    })?;
    let mut manifest = read_manifest(home);
    let mut states = Vec::new();

    for asset in content::assets() {
        let disk_path = home.join(asset.rel_path);
        let disk = std::fs::read_to_string(&disk_path).ok();
        let state = match disk {
            None => {
                // absent (or deleted): materialize the shipped copy
                if let Some(parent) = disk_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                std::fs::write(&disk_path, asset.body).map_err(|source| CoreError::Io {
                    path: disk_path.clone(),
                    source,
                })?;
                manifest.set(asset.rel_path, asset.sha256);
                AssetState {
                    rel_path: asset.rel_path.to_string(),
                    origin: Origin::Materialized,
                    stale_against_embedded: false,
                }
            }
            Some(body) => {
                let disk_digest = sha256_hex(&body);
                let recorded = manifest.digest_of(asset.rel_path).map(str::to_string);
                let unmodified = recorded.as_deref() == Some(disk_digest.as_str());
                if unmodified {
                    if disk_digest != asset.sha256 {
                        // an unmodified default another build shipped — an
                        // older version, or this one built from other
                        // content: refresh
                        std::fs::write(&disk_path, asset.body).map_err(|source| CoreError::Io {
                            path: disk_path.clone(),
                            source,
                        })?;
                        manifest.set(asset.rel_path, asset.sha256);
                    }
                    AssetState {
                        rel_path: asset.rel_path.to_string(),
                        origin: if sha256_hex(
                            &std::fs::read_to_string(&disk_path).unwrap_or_default(),
                        ) == asset.sha256
                        {
                            Origin::Materialized
                        } else {
                            Origin::Override
                        },
                        stale_against_embedded: false,
                    }
                } else if disk_digest == asset.sha256 {
                    // matches the current shipped bytes (e.g. fresh manifest)
                    manifest.set(asset.rel_path, asset.sha256);
                    AssetState {
                        rel_path: asset.rel_path.to_string(),
                        origin: Origin::Materialized,
                        stale_against_embedded: false,
                    }
                } else {
                    // user override: left in place; stale when forked from
                    // another shipped version
                    let stale = recorded.is_some_and(|r| r != asset.sha256);
                    AssetState {
                        rel_path: asset.rel_path.to_string(),
                        origin: Origin::Override,
                        stale_against_embedded: stale,
                    }
                }
            }
        };
        states.push(state);
    }
    manifest.vitrify_version = version.to_string();
    write_manifest(home, &manifest)?;
    Ok(states)
}

/// Resolve one asset disk-first: the `~/.vitrify` copy wins over the
/// embedded copy. Refuses a home `vitrify init` has not set up; in one it
/// has, materializes on every call (delete-restores).
pub fn resolve(home: &Path, version: &str, rel: &str) -> Result<(String, Origin), CoreError> {
    require_set_up(home)?;
    ensure_materialized(home, version)?;
    let disk_path = home.join(rel);
    if let Ok(body) = std::fs::read_to_string(&disk_path) {
        let origin = match content::get(rel) {
            Some(asset) if sha256_hex(&body) == asset.sha256 => Origin::Materialized,
            _ => Origin::Override,
        };
        return Ok((body, origin));
    }
    match content::get(rel) {
        Some(asset) => Ok((asset.body.to_string(), Origin::Embedded)),
        None => Err(CoreError::refusal(
            4,
            format!("error: unknown asset: {rel}"),
        )),
    }
}
