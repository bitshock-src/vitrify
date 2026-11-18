//! Harness role profiles: per-role model and reasoning effort for the
//! inference points `vitrify run` drives (workers, escalation), per harness
//! CLI. Pure parse + argv assembly over the `[harness.<name>]` tables of
//! `~/.vitrify/config.toml`, which `vitrify init` writes with the shipped
//! defaults.
//!
//! Each harness table names how its CLI spells the two knobs
//! (`model_flag`, `effort_flag`, with `{model}` / `{effort}` placeholders),
//! and one sub-table per role gives the values. An empty value omits its
//! flag, leaving the harness's own default in force. Values are passed
//! through unvalidated — the harness CLI owns its vocabulary.
//!
//! `retry` is the worker role for a unit whose earlier attempt did not
//! land; a harness that does not name it retries with the `worker` profile.

use crate::CoreError;

/// The roles `vitrify run` dispatches inference for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Worker,
    /// A worker dispatched for a unit whose earlier attempt did not land.
    Retry,
    Escalation,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Worker, Role::Retry, Role::Escalation];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Worker => "worker",
            Role::Retry => "retry",
            Role::Escalation => "escalation",
        }
    }
}

/// One role's settings for one harness.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleProfile {
    pub model: String,
    pub effort: String,
    /// Extra argv appended verbatim after the model/effort flags.
    pub args: Vec<String>,
}

/// One harness's flag spellings and role profiles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarnessProfile {
    pub model_flag: Vec<String>,
    pub effort_flag: Vec<String>,
    pub worker: RoleProfile,
    /// `None` when the file has no `retry` table: retries use `worker`.
    pub retry: Option<RoleProfile>,
    pub escalation: RoleProfile,
}

impl HarnessProfile {
    pub fn role(&self, role: Role) -> &RoleProfile {
        match role {
            Role::Worker => &self.worker,
            Role::Retry => self.retry.as_ref().unwrap_or(&self.worker),
            Role::Escalation => &self.escalation,
        }
    }

    /// Whether `role` has settings of its own (`retry` without a table
    /// borrows the worker's).
    pub fn is_own(&self, role: Role) -> bool {
        !matches!(role, Role::Retry) || self.retry.is_some()
    }

    /// The argv to append after the headless spelling the probe recorded
    /// (and before the prompt) for `role`.
    pub fn role_args(&self, role: Role) -> Vec<String> {
        let r = self.role(role);
        let mut out = Vec::new();
        if !r.model.is_empty() {
            out.extend(substitute(&self.model_flag, "{model}", &r.model));
        }
        if !r.effort.is_empty() {
            out.extend(substitute(&self.effort_flag, "{effort}", &r.effort));
        }
        out.extend(r.args.iter().cloned());
        out
    }

    /// One-line human summary of a role, e.g. `model=sonnet effort=high`.
    pub fn describe(&self, role: Role) -> String {
        let r = self.role(role);
        let show = |v: &str| {
            if v.is_empty() {
                "(harness default)".to_string()
            } else {
                v.to_string()
            }
        };
        let mut s = format!("model={} effort={}", show(&r.model), show(&r.effort));
        if !r.args.is_empty() {
            s.push_str(&format!(" args={}", r.args.join(" ")));
        }
        s
    }
}

fn substitute(template: &[String], key: &str, value: &str) -> Vec<String> {
    template.iter().map(|a| a.replace(key, value)).collect()
}

/// The parsed role profiles: harness name → profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profiles {
    pub harnesses: Vec<(String, HarnessProfile)>,
}

impl Profiles {
    /// The profile for `harness`; a harness the file does not name gets
    /// an empty profile (no extra argv — the harness's own defaults).
    pub fn get(&self, harness: &str) -> HarnessProfile {
        self.harnesses
            .iter()
            .find(|(k, _)| k == harness)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    /// The full argv template for `role`: recorded spelling + role args.
    pub fn argv_for(&self, harness: &str, recorded: &[String], role: Role) -> Vec<String> {
        let mut argv = recorded.to_vec();
        argv.extend(self.get(harness).role_args(role));
        argv
    }
}

/// Parse the role profiles of a configuration file's text: its `harness`
/// table. Strict about shape and key names (a typo'd key is a refusal naming
/// it, exit 2), permissive about values.
pub fn parse(text: &str, source: &str) -> Result<Profiles, CoreError> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| CoreError::refusal(2, format!("error: unparseable {source}: {e}")))?;
    from_config(&table, source)
}

/// The role profiles of a parsed configuration file: every table under
/// `harness` (its `preferred` setting aside). A file with no `harness`
/// table names no profiles.
pub fn from_config(table: &toml::Table, source: &str) -> Result<Profiles, CoreError> {
    parse_with(table, source, true)
}

/// The role profiles of several configuration files, a later one
/// overriding an earlier one key by key — a repository's `.vitrify/config.toml`
/// over `~/.vitrify/config.toml`: `[harness.claude.worker]` with only
/// `model` set changes the worker's model and keeps the rest. Each file's
/// shape and key names are checked on their own, so an error names its file.
pub fn from_layers(layers: &[(toml::Table, String)]) -> Result<Profiles, CoreError> {
    let mut merged = toml::Value::Table(toml::Table::new());
    for (table, source) in layers {
        parse_with(table, source, false)?;
        if let Some(h) = table.get("harness") {
            let mut wrap = toml::Table::new();
            wrap.insert("harness".to_string(), h.clone());
            overlay(&mut merged, &toml::Value::Table(wrap));
        }
    }
    let label = layers
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join(" with ");
    match merged {
        toml::Value::Table(t) => from_config(&t, &label),
        _ => Ok(Profiles::default()),
    }
}

/// `top` over `base`: tables merge key by key; any other value replaces.
fn overlay(base: &mut toml::Value, top: &toml::Value) {
    match (base, top) {
        (toml::Value::Table(b), toml::Value::Table(t)) => {
            for (k, v) in t {
                match b.get_mut(k) {
                    Some(slot) => overlay(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, t) => *b = t.clone(),
    }
}

/// Parse the profiles; `cross_check` adds the rules that look across keys
/// (a model needs a flag to carry it), which hold only for a whole profile.
fn parse_with(table: &toml::Table, source: &str, cross_check: bool) -> Result<Profiles, CoreError> {
    let bad =
        |path: &str, why: &str| CoreError::refusal(2, format!("error: {source}: `{path}` {why}"));
    let Some(harness) = table.get("harness") else {
        return Ok(Profiles::default());
    };
    let harness = harness
        .as_table()
        .ok_or_else(|| bad("harness", "must be a table"))?;

    let mut profiles = Profiles::default();
    for (name, value) in harness {
        if name == "preferred" {
            continue; // a setting, read with the others
        }
        let key = format!("harness.{name}");
        let h = value
            .as_table()
            .ok_or_else(|| bad(&key, "must be a table"))?;
        let mut hp = HarnessProfile::default();
        for (hk, hv) in h {
            let path = format!("{key}.{hk}");
            match hk.as_str() {
                "model_flag" => {
                    hp.model_flag =
                        str_array(hv).ok_or_else(|| bad(&path, "must be an array of strings"))?
                }
                "effort_flag" => {
                    hp.effort_flag =
                        str_array(hv).ok_or_else(|| bad(&path, "must be an array of strings"))?
                }
                "worker" | "retry" | "escalation" => {
                    let rp = parse_role(hv, &path, &bad)?;
                    match hk.as_str() {
                        "worker" => hp.worker = rp,
                        "retry" => hp.retry = Some(rp),
                        _ => hp.escalation = rp,
                    }
                }
                _ => {
                    return Err(bad(
                        &path,
                        "is not a known key (model_flag, effort_flag, worker, retry, escalation)",
                    ));
                }
            }
        }
        for role in Role::ALL {
            if !cross_check {
                break;
            }
            let r = hp.role(role);
            if !r.model.is_empty() && !hp.model_flag.iter().any(|a| a.contains("{model}")) {
                return Err(bad(
                    &format!("{key}.model_flag"),
                    &format!(
                        "must contain {{model}} when {key}.{}.model is set",
                        role.as_str()
                    ),
                ));
            }
            if !r.effort.is_empty() && !hp.effort_flag.iter().any(|a| a.contains("{effort}")) {
                return Err(bad(
                    &format!("{key}.effort_flag"),
                    &format!(
                        "must contain {{effort}} when {key}.{}.effort is set",
                        role.as_str()
                    ),
                ));
            }
        }
        profiles.harnesses.push((name.clone(), hp));
    }
    Ok(profiles)
}

fn parse_role(
    value: &toml::Value,
    path: &str,
    bad: &dyn Fn(&str, &str) -> CoreError,
) -> Result<RoleProfile, CoreError> {
    let t = value
        .as_table()
        .ok_or_else(|| bad(path, "must be a table"))?;
    let mut rp = RoleProfile::default();
    for (k, v) in t {
        let p = format!("{path}.{k}");
        match k.as_str() {
            "model" => {
                rp.model = v
                    .as_str()
                    .ok_or_else(|| bad(&p, "must be a string"))?
                    .trim()
                    .to_string()
            }
            "effort" => {
                rp.effort = v
                    .as_str()
                    .ok_or_else(|| bad(&p, "must be a string"))?
                    .trim()
                    .to_string()
            }
            "args" => {
                rp.args = str_array(v).ok_or_else(|| bad(&p, "must be an array of strings"))?
            }
            _ => return Err(bad(&p, "is not a known key (model, effort, args)")),
        }
    }
    Ok(rp)
}

fn str_array(v: &toml::Value) -> Option<Vec<String>> {
    v.as_array()?
        .iter()
        .map(|x| x.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema = 1

[harness]
preferred = ""

[harness.claude]
model_flag = ["--model", "{model}"]
effort_flag = ["--effort", "{effort}"]

[harness.claude.worker]
model = "sonnet"
effort = "high"

[harness.claude.escalation]
model = "opus"
effort = "max"
args = ["--fallback-model", "sonnet"]

[harness.codex]
model_flag = ["-m", "{model}"]
effort_flag = ["-c", "model_reasoning_effort=\"{effort}\""]

[harness.codex.worker]
model = ""
effort = "medium"
"#;

    fn rec(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn builds_role_argv_after_the_recorded_spelling() {
        let p = parse(SAMPLE, "config.toml").unwrap();
        let recorded = rec(&[
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
        ]);
        assert_eq!(
            p.argv_for("claude", &recorded, Role::Worker),
            rec(&[
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "sonnet",
                "--effort",
                "high"
            ])
        );
        assert_eq!(
            p.argv_for("claude", &recorded, Role::Escalation),
            rec(&[
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "opus",
                "--effort",
                "max",
                "--fallback-model",
                "sonnet"
            ])
        );
    }

    #[test]
    fn empty_values_and_unnamed_roles_or_harnesses_add_nothing() {
        let p = parse(SAMPLE, "config.toml").unwrap();
        let recorded = rec(&["codex", "exec", "--json"]);
        // empty model omitted; effort substituted inside one argv element
        assert_eq!(
            p.argv_for("codex", &recorded, Role::Worker),
            rec(&[
                "codex",
                "exec",
                "--json",
                "-c",
                "model_reasoning_effort=\"medium\""
            ])
        );
        // role table absent
        assert_eq!(p.argv_for("codex", &recorded, Role::Escalation), recorded);
        // harness absent
        let oc = rec(&["opencode", "run"]);
        assert_eq!(p.argv_for("opencode", &oc, Role::Worker), oc);
    }

    #[test]
    fn typos_and_bad_shapes_are_refused_by_name() {
        let e = parse("[harness.claude.worker]\nmodle = \"x\"\n", "c.toml").unwrap_err();
        assert!(
            e.to_string()
                .contains("`harness.claude.worker.modle` is not a known key"),
            "{e}"
        );
        assert_eq!(e.exit_code(), 2);

        let e = parse("[harness.claude]\nmodel_flag = \"--model\"\n", "c.toml").unwrap_err();
        assert!(
            e.to_string()
                .contains("`harness.claude.model_flag` must be an array"),
            "{e}"
        );

        let e = parse("[harness]\nclaude = 3\n", "c.toml").unwrap_err();
        assert!(
            e.to_string().contains("`harness.claude` must be a table"),
            "{e}"
        );

        // a value with no flag to carry it
        let e = parse("[harness.claude.worker]\nmodel = \"opus\"\n", "c.toml").unwrap_err();
        assert!(e.to_string().contains("must contain {model}"), "{e}");

        // settings beside the profiles are not profiles
        let p = parse(
            "[run]\nworker_cap = 2\n[harness]\npreferred = \"codex\"\n",
            "c.toml",
        )
        .unwrap();
        assert!(p.harnesses.is_empty());
    }

    #[test]
    fn retry_borrows_the_worker_profile_until_named() {
        let p = parse(SAMPLE, "config.toml").unwrap();
        let recorded = rec(&["claude", "-p"]);
        // no [harness.claude.retry]: a retry runs exactly as a first attempt does
        assert!(!p.get("claude").is_own(Role::Retry));
        assert_eq!(
            p.argv_for("claude", &recorded, Role::Retry),
            p.argv_for("claude", &recorded, Role::Worker)
        );

        let named =
            format!("{SAMPLE}\n[harness.claude.retry]\nmodel = \"opus\"\neffort = \"high\"\n");
        let p = parse(&named, "config.toml").unwrap();
        assert!(p.get("claude").is_own(Role::Retry));
        assert_eq!(
            p.argv_for("claude", &recorded, Role::Retry),
            rec(&["claude", "-p", "--model", "opus", "--effort", "high"])
        );
        // the first attempt is untouched
        assert_eq!(
            p.argv_for("claude", &recorded, Role::Worker),
            rec(&["claude", "-p", "--model", "sonnet", "--effort", "high"])
        );
    }

    #[test]
    fn a_later_layer_overrides_key_by_key() {
        let home: toml::Table = SAMPLE.parse().unwrap();
        let repo: toml::Table = "[harness.claude.worker]\nmodel = \"haiku\"\n\n[harness.codex.escalation]\nmodel = \"gpt-x\"\n"
            .parse()
            .unwrap();
        let p = from_layers(&[
            (home.clone(), "home".to_string()),
            (repo, "repo".to_string()),
        ])
        .unwrap();
        // the model moved, the effort stayed
        assert_eq!(
            p.get("claude").describe(Role::Worker),
            "model=haiku effort=high"
        );
        // untouched roles keep the home's values; a new role table lands whole
        assert_eq!(
            p.get("claude").describe(Role::Escalation),
            "model=opus effort=max args=--fallback-model sonnet"
        );
        assert_eq!(
            p.get("codex").describe(Role::Escalation),
            "model=gpt-x effort=(harness default)"
        );

        // a bad key names the file it is in
        let bad: toml::Table = "[harness.claude.worker]\nmodle = \"x\"\n".parse().unwrap();
        let e = from_layers(&[(home, "home".to_string()), (bad, "repo".to_string())]).unwrap_err();
        assert!(
            e.to_string()
                .contains("error: repo: `harness.claude.worker.modle`"),
            "{e}"
        );
    }

    #[test]
    fn describe_names_defaults() {
        let p = parse(SAMPLE, "config.toml").unwrap();
        assert_eq!(
            p.get("codex").describe(Role::Worker),
            "model=(harness default) effort=medium"
        );
        assert_eq!(
            p.get("claude").describe(Role::Escalation),
            "model=opus effort=max args=--fallback-model sonnet"
        );
    }
}
