//! Configuration: TOML schema with built-in defaults and one precedence —
//! CLI flags > `VITRIFY_*` env > repo `.vitrify/config.toml` >
//! `~/.vitrify/config.toml` > defaults.
//!
//! `vitrify init` writes `~/.vitrify/config.toml` with every setting at its
//! default; the harness role profiles and the probe record live in the same
//! file, read by the harness and probe code. A repository's
//! `.vitrify/config.toml` may override any of it for that repository —
//! settings, `schema`, role profiles — but the probe record, which describes
//! the machine.

use std::path::Path;

use crate::CoreError;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Repo-level mainline branch override; empty = resolve by the mainline precedence.
    pub mainline: String,
    pub run: RunConfig,
    pub harness: HarnessConfig,
    pub lint: LintConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    pub worker_cap: i64,
    pub lease_minutes: i64,
    pub renew_fraction: f64,
    /// Failed attempts of one unit before the escalation call; twice as many
    /// set the unit aside for the rest of the run.
    pub abort_failed_waves: i64,
    pub worker_timeout_minutes: i64,
    /// Verification-table rows the gate runs at once. 0 — the default —
    /// means as many as this machine has CPU cores to give
    /// ([`RunConfig::rows_at_once`]); 1 runs them one at a time.
    pub gate_jobs: i64,
    /// How often a run with nothing to react to looks at mainline again —
    /// for a spec change, a lease that lapsed, another holder's release.
    pub poll_seconds: i64,
    /// Where the unit worktrees and the integration checkout live. Empty =
    /// outside the working tree, under the vitrify home; a relative path is
    /// taken from the repository root.
    pub worktree_dir: String,
    /// The longest an agent operating a run waits between two
    /// `vitrify run status` checks when its harness cannot notify it
    /// (`vitrify brief run` prints it). 270 s keeps each check inside a
    /// model provider's five-minute prompt-cache lifetime, with half a
    /// minute of margin for generation time.
    pub status_check_seconds: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessConfig {
    /// "" = the first harness the probe recorded, of claude, codex,
    /// opencode. Per-role model and effort are the `[harness.<name>]`
    /// tables beside it, read by the harness module.
    pub preferred: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LintConfig {
    pub readset_budget_bytes: i64,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            mainline: String::new(),
            run: RunConfig {
                worker_cap: 4,
                lease_minutes: 60,
                renew_fraction: 0.25,
                abort_failed_waves: 2,
                worker_timeout_minutes: 120,
                gate_jobs: 0,
                poll_seconds: 30,
                worktree_dir: String::new(),
                status_check_seconds: 270,
            },
            harness: HarnessConfig {
                preferred: String::new(),
            },
            lint: LintConfig {
                readset_budget_bytes: 65536,
            },
        }
    }
}

impl RunConfig {
    /// How many Verification rows run at once: `gate_jobs` when it names a
    /// number, otherwise one per CPU core available to this process — the
    /// rule Rust's test harness uses for its test threads (`cargo test`
    /// runs tests side by side by default; `--test-threads=1` is the
    /// opt-out, as `gate_jobs = 1` is here).
    pub fn rows_at_once(&self) -> usize {
        if self.gate_jobs >= 1 {
            self.gate_jobs as usize
        } else {
            available_parallelism()
        }
    }

    /// Whether the number of rows at once follows the machine rather than
    /// a configured value.
    pub fn rows_follow_cores(&self) -> bool {
        self.gate_jobs < 1
    }
}

/// The CPU cores this process may use (cgroup quotas and affinity masks
/// counted where the platform has them); 1 when that cannot be told.
pub fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

/// Typed partial from the clap layer; every `Some` wins over everything.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub mainline: Option<String>,
    pub worker_cap: Option<i64>,
    pub lease_minutes: Option<i64>,
    pub readset_budget_bytes: Option<i64>,
    pub gate_jobs: Option<i64>,
}

fn apply_toml(cfg: &mut Config, value: &toml::Value, source: &str) -> Result<(), CoreError> {
    let bad = |path: &str| {
        CoreError::refusal(
            2,
            format!("error: invalid config value for `{path}` in {source}"),
        )
    };
    let table = value
        .as_table()
        .ok_or_else(|| CoreError::refusal(2, format!("error: {source} is not a TOML table")))?;
    if let Some(v) = table.get("schema")
        && v.as_integer() != Some(1)
    {
        return Err(CoreError::refusal(
            2,
            format!("error: invalid config value for `schema` in {source}: it must be 1"),
        ));
    }
    if let Some(v) = table.get("mainline") {
        cfg.mainline = v.as_str().ok_or_else(|| bad("mainline"))?.to_string();
    }
    if let Some(run) = table.get("run") {
        let t = run.as_table().ok_or_else(|| bad("run"))?;
        if let Some(v) = t.get("worker_cap") {
            cfg.run.worker_cap = v.as_integer().ok_or_else(|| bad("run.worker_cap"))?;
        }
        if let Some(v) = t.get("lease_minutes") {
            cfg.run.lease_minutes = v.as_integer().ok_or_else(|| bad("run.lease_minutes"))?;
        }
        if let Some(v) = t.get("renew_fraction") {
            cfg.run.renew_fraction = match v {
                toml::Value::Float(f) => *f,
                toml::Value::Integer(i) => *i as f64,
                _ => return Err(bad("run.renew_fraction")),
            };
        }
        if let Some(v) = t.get("abort_failed_waves") {
            cfg.run.abort_failed_waves = v
                .as_integer()
                .ok_or_else(|| bad("run.abort_failed_waves"))?;
        }
        if let Some(v) = t.get("worker_timeout_minutes") {
            cfg.run.worker_timeout_minutes = v
                .as_integer()
                .ok_or_else(|| bad("run.worker_timeout_minutes"))?;
        }
        if let Some(v) = t.get("gate_jobs") {
            cfg.run.gate_jobs = v.as_integer().ok_or_else(|| bad("run.gate_jobs"))?;
        }
        if let Some(v) = t.get("poll_seconds") {
            cfg.run.poll_seconds = v.as_integer().ok_or_else(|| bad("run.poll_seconds"))?;
        }
        if let Some(v) = t.get("status_check_seconds") {
            cfg.run.status_check_seconds = v
                .as_integer()
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("run.status_check_seconds"))?;
        }
        if let Some(v) = t.get("worktree_dir") {
            cfg.run.worktree_dir = v
                .as_str()
                .ok_or_else(|| bad("run.worktree_dir"))?
                .to_string();
        }
    }
    if let Some(h) = table.get("harness") {
        let t = h.as_table().ok_or_else(|| bad("harness"))?;
        if let Some(v) = t.get("preferred") {
            cfg.harness.preferred = v
                .as_str()
                .ok_or_else(|| bad("harness.preferred"))?
                .to_string();
        }
    }
    if let Some(l) = table.get("lint") {
        let t = l.as_table().ok_or_else(|| bad("lint"))?;
        if let Some(v) = t.get("readset_budget_bytes") {
            cfg.lint.readset_budget_bytes = v
                .as_integer()
                .ok_or_else(|| bad("lint.readset_budget_bytes"))?;
        }
    }
    Ok(())
}

fn apply_file(cfg: &mut Config, path: &Path, repo: bool) -> Result<(), CoreError> {
    if !path.exists() {
        return Ok(());
    }
    let text = crate::model::read_text(path)?;
    let value: toml::Table = text.parse().map_err(|e| {
        CoreError::refusal(
            2,
            format!(
                "error: unparseable config file {}: {e} → fix the line it names",
                path.display()
            ),
        )
    })?;
    if repo {
        refuse_probe_record(&value, path)?;
    }
    apply_toml(cfg, &toml::Value::Table(value), &path.display().to_string())
}

/// A repository's config.toml may override everything a repository can
/// decide; the probe record describes the machine, and only `vitrify init`
/// and `vitrify probe` write it.
fn refuse_probe_record(table: &toml::Table, path: &Path) -> Result<(), CoreError> {
    if table.contains_key("probe") {
        return Err(CoreError::refusal(
            2,
            format!(
                "error: {}: `probe` describes the machine and belongs in ~/.vitrify/config.toml \
                 → remove it; vitrify init and vitrify probe write it",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// The parsed configuration files that apply here, in precedence order:
/// `~/.vitrify/config.toml`, then the repository's `.vitrify/config.toml`
/// when there is one — each with the label its errors name. The role
/// profiles are read from these layers; the settings, through [`load`].
pub fn layers(
    repo_root: Option<&Path>,
    home_dir: Option<&Path>,
) -> Result<Vec<(toml::Table, String)>, CoreError> {
    let mut out = Vec::new();
    let mut read = |path: &Path, label: String, repo: bool| -> Result<(), CoreError> {
        if !path.exists() {
            return Ok(());
        }
        let text = crate::model::read_text(path)?;
        let table: toml::Table = text.parse().map_err(|e| {
            CoreError::refusal(2, format!("error: unparseable config file {label}: {e}"))
        })?;
        if repo {
            refuse_probe_record(&table, path)?;
        }
        out.push((table, label));
        Ok(())
    };
    if let Some(home) = home_dir {
        read(
            &home.join("config.toml"),
            "~/.vitrify/config.toml".to_string(),
            false,
        )?;
    }
    if let Some(root) = repo_root {
        let path = root.join(".vitrify/config.toml");
        read(&path, path.display().to_string(), true)?;
    }
    Ok(out)
}

/// Env mapping: `VITRIFY_<SECTION>_<KEY>` upper-snake over the TOML path with
/// `.` → `_`; values parsed as the TOML type; parse failure is exit 2 naming
/// the variable.
fn apply_env(cfg: &mut Config, env: &[(String, String)]) -> Result<(), CoreError> {
    let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let bad_int = |var: &str| {
        CoreError::refusal(
            2,
            format!("error: environment variable {var} is not a valid integer"),
        )
    };
    if let Some(v) = get("VITRIFY_MAINLINE") {
        cfg.mainline = v;
    }
    if let Some(v) = get("VITRIFY_RUN_WORKER_CAP") {
        cfg.run.worker_cap = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_WORKER_CAP"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_LEASE_MINUTES") {
        cfg.run.lease_minutes = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_LEASE_MINUTES"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_RENEW_FRACTION") {
        cfg.run.renew_fraction = v.trim().parse().map_err(|_| {
            CoreError::refusal(
                2,
                "error: environment variable VITRIFY_RUN_RENEW_FRACTION is not a valid float",
            )
        })?;
    }
    if let Some(v) = get("VITRIFY_RUN_ABORT_FAILED_WAVES") {
        cfg.run.abort_failed_waves = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_ABORT_FAILED_WAVES"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_WORKER_TIMEOUT_MINUTES") {
        cfg.run.worker_timeout_minutes = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_WORKER_TIMEOUT_MINUTES"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_GATE_JOBS") {
        cfg.run.gate_jobs = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_GATE_JOBS"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_POLL_SECONDS") {
        cfg.run.poll_seconds = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_RUN_POLL_SECONDS"))?;
    }
    if let Some(v) = get("VITRIFY_RUN_STATUS_CHECK_SECONDS") {
        cfg.run.status_check_seconds =
            v.trim()
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| {
                    CoreError::refusal(
                        2,
                        "error: environment variable VITRIFY_RUN_STATUS_CHECK_SECONDS \
                     (run.status_check_seconds) is not a positive integer",
                    )
                })?;
    }
    if let Some(v) = get("VITRIFY_RUN_WORKTREE_DIR") {
        cfg.run.worktree_dir = v;
    }
    if let Some(v) = get("VITRIFY_HARNESS_PREFERRED") {
        cfg.harness.preferred = v;
    }
    if let Some(v) = get("VITRIFY_LINT_READSET_BUDGET_BYTES") {
        cfg.lint.readset_budget_bytes = v
            .trim()
            .parse()
            .map_err(|_| bad_int("VITRIFY_LINT_READSET_BUDGET_BYTES"))?;
    }
    Ok(())
}

/// Load with the precedence above. `home_dir` is the resolved `~/.vitrify`
/// (`$VITRIFY_HOME` else `$HOME/.vitrify`, resolved by the caller).
pub fn load(
    repo_root: Option<&Path>,
    home_dir: Option<&Path>,
    env: &[(String, String)],
    overrides: &Overrides,
) -> Result<Config, CoreError> {
    let mut cfg = Config::default();
    if let Some(home) = home_dir {
        apply_file(&mut cfg, &home.join("config.toml"), false)?;
    }
    if let Some(root) = repo_root {
        apply_file(&mut cfg, &root.join(".vitrify/config.toml"), true)?;
    }
    apply_env(&mut cfg, env)?;
    if let Some(v) = &overrides.mainline {
        cfg.mainline = v.clone();
    }
    if let Some(v) = overrides.worker_cap {
        cfg.run.worker_cap = v;
    }
    if let Some(v) = overrides.lease_minutes {
        cfg.run.lease_minutes = v;
    }
    if let Some(v) = overrides.readset_budget_bytes {
        cfg.lint.readset_budget_bytes = v;
    }
    if let Some(v) = overrides.gate_jobs {
        cfg.run.gate_jobs = v;
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_with(env: &[(&str, &str)], overrides: &Overrides) -> Config {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        load(None, None, &env, overrides).unwrap()
    }

    #[test]
    fn rows_at_once_follow_the_machine_unless_a_number_is_given() {
        // nothing configured: one row per available core, as `cargo test`
        // runs one test thread per core
        let cfg = load_with(&[], &Overrides::default());
        assert_eq!(cfg.run.gate_jobs, 0);
        assert!(cfg.run.rows_follow_cores());
        assert_eq!(cfg.run.rows_at_once(), available_parallelism());
        assert!(cfg.run.rows_at_once() >= 1);

        // a number is that number, whatever the machine
        let cfg = load_with(&[("VITRIFY_RUN_GATE_JOBS", "3")], &Overrides::default());
        assert!(!cfg.run.rows_follow_cores());
        assert_eq!(cfg.run.rows_at_once(), 3);
        // 1 is the opt-out: one row at a time
        let serial = Overrides {
            gate_jobs: Some(1),
            ..Default::default()
        };
        assert_eq!(
            load_with(&[("VITRIFY_RUN_GATE_JOBS", "3")], &serial)
                .run
                .rows_at_once(),
            1
        );
        // 0 says "follow the machine" out loud — over a lower layer's number
        let auto = Overrides {
            gate_jobs: Some(0),
            ..Default::default()
        };
        let cfg = load_with(&[("VITRIFY_RUN_GATE_JOBS", "3")], &auto);
        assert!(cfg.run.rows_follow_cores());
        assert_eq!(cfg.run.rows_at_once(), available_parallelism());
    }

    #[test]
    fn status_check_seconds_defaults_to_270_and_must_be_positive() {
        assert_eq!(
            load_with(&[], &Overrides::default())
                .run
                .status_check_seconds,
            270
        );
        let cfg = load_with(
            &[("VITRIFY_RUN_STATUS_CHECK_SECONDS", "600")],
            &Overrides::default(),
        );
        assert_eq!(cfg.run.status_check_seconds, 600);
        for bad in ["0", "-5", "soon"] {
            let env = vec![(
                "VITRIFY_RUN_STATUS_CHECK_SECONDS".to_string(),
                bad.to_string(),
            )];
            let err = load(None, None, &env, &Overrides::default()).unwrap_err();
            assert!(
                format!("{err}").contains("run.status_check_seconds"),
                "{err}"
            );
        }
        let dir = std::env::temp_dir().join(format!("vfy-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "[run]\nstatus_check_seconds = 0\n").unwrap();
        let err = load(None, Some(&dir), &[], &Overrides::default()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("run.status_check_seconds"), "{msg}");
        assert!(msg.contains("config.toml"), "{msg}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_shipped_default_is_the_built_in_default() {
        let dir = std::env::temp_dir().join(format!("vfy-cfg-default-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), crate::content::CONFIG.body).unwrap();
        let cfg = load(None, Some(&dir), &[], &Overrides::default()).unwrap();
        assert_eq!(cfg, Config::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn schema_must_be_1() {
        let dir = std::env::temp_dir().join(format!("vfy-cfg-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "schema = 2\n").unwrap();
        let err = load(None, Some(&dir), &[], &Overrides::default()).unwrap_err();
        assert!(format!("{err}").contains("`schema`"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repository_overrides_all_but_the_probe_record() {
        let base = std::env::temp_dir().join(format!("vfy-cfg-repo-{}", std::process::id()));
        let home = base.join("home");
        let repo = base.join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(repo.join(".vitrify")).unwrap();
        std::fs::write(home.join("config.toml"), "[run]\nworker_cap = 6\n").unwrap();
        std::fs::write(
            repo.join(".vitrify/config.toml"),
            "mainline = \"trunk\"\n[run]\nworker_cap = 2\n[harness]\npreferred = \"codex\"\n",
        )
        .unwrap();
        let cfg = load(Some(&repo), Some(&home), &[], &Overrides::default()).unwrap();
        assert_eq!(cfg.mainline, "trunk");
        assert_eq!(cfg.run.worker_cap, 2);
        assert_eq!(cfg.harness.preferred, "codex");

        // its own schema version, and role profile overrides
        std::fs::write(
            repo.join(".vitrify/config.toml"),
            "schema = 1\n[harness.claude.worker]\nmodel = \"sonnet\"\n",
        )
        .unwrap();
        load(Some(&repo), Some(&home), &[], &Overrides::default()).unwrap();
        assert_eq!(layers(Some(&repo), Some(&home)).unwrap().len(), 2);
        // a schema version this vitrify does not read is refused there too
        std::fs::write(repo.join(".vitrify/config.toml"), "schema = 2\n").unwrap();
        let err = load(Some(&repo), Some(&home), &[], &Overrides::default()).unwrap_err();
        assert!(format!("{err}").contains("`schema`"), "{err}");

        // the probe record describes the machine
        std::fs::write(
            repo.join(".vitrify/config.toml"),
            "[probe.claude]\npresent = true\n",
        )
        .unwrap();
        for err in [
            load(Some(&repo), Some(&home), &[], &Overrides::default()).unwrap_err(),
            layers(Some(&repo), Some(&home)).unwrap_err(),
        ] {
            let msg = format!("{err}");
            assert!(msg.contains("`probe` describes the machine"), "{msg}");
            assert_eq!(err.exit_code(), 2);
        }
        std::fs::remove_dir_all(&base).ok();
    }
}
