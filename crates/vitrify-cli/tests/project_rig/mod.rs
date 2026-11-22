//! A project repository driven by `vitrify run` with the stub worker: a
//! seeded `fixtures/seeded/project-*` copy, a synthetic project, or one a
//! test writes itself. Shared by the project-run progress tests and the
//! benchmark.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

pub fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
}

pub fn stub_bin() -> PathBuf {
    // built once per test process
    static STUB: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    STUB.get_or_init(|| {
        let stub = PathBuf::from(vitrify_bin())
            .parent()
            .unwrap()
            .join("stub-worker");
        assert!(
            Command::new(env!("CARGO"))
                .args(["build", "-p", "stub-worker", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        stub
    })
    .clone()
}

pub fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let (p, t) = (entry.path(), dst.join(entry.file_name()));
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn put(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// `~/.vitrify/config.toml` with a probe record naming the stub worker.
fn write_config(home: &Path) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "{}\n[probe]\nvitrify_version = \"{}\"\ngit = {{ present = true, version = \"2\" }}\n\n[probe.claude]\npresent = true\nheadless = [\"{}\"]\n",
            vitrify_core::content::CONFIG.body,
            env!("CARGO_PKG_VERSION"),
            stub_bin().display()
        ),
    )
    .unwrap();
}

pub struct Rig {
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    /// Where the stub worker reads its canned files and maps from.
    pub support: PathBuf,
}

/// The project of `fixtures/seeded/<fixture>`, not yet committed: a test
/// may change it first, then call [`commit_seed`].
pub fn rig_of_uncommitted(fixture: &str) -> Rig {
    let r = empty_rig();
    let dir = repo_root().join("fixtures/seeded").join(fixture);
    copy_dir(&dir.join("project"), &r.root);
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "project" || name == "README.md" {
            continue;
        }
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &r.support.join(&name));
        } else {
            std::fs::copy(entry.path(), r.support.join(&name)).unwrap();
        }
    }
    r
}

/// The project of `fixtures/seeded/<fixture>`, committed on main.
pub fn rig_of(fixture: &str) -> Rig {
    let r = rig_of_uncommitted(fixture);
    commit_seed(&r.root);
    r
}

pub fn empty_rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, home, support) = (base.join("repo"), base.join("vhome"), base.join("support"));
    for d in [&root, &support, &base.join("t")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_config(&home);
    Rig {
        tmp,
        root,
        home,
        support,
    }
}

pub fn commit_seed(root: &Path) {
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@test"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "seed"]);
}

impl Rig {
    /// The scratch directory, outside the repository and every checkout.
    pub fn scratch(&self) -> PathBuf {
        self.tmp.path().canonicalize().unwrap()
    }

    pub fn command(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
        let mut c = Command::new(vitrify_bin());
        c.args(args)
            .current_dir(&self.root)
            .env("VITRIFY_HOME", &self.home)
            .env("STUB_REF", self.support.join("solution"))
            .env("STUB_MAP", self.support.join("stub-map-{sdd}.toml"))
            .env("VITRIFY_BIN", vitrify_bin())
            .env("STUB_MODE", "build")
            .env("TMPDIR", self.tmp.path().join("t"))
            .env("VITRIFY_RUN_POLL_SECONDS", "1");
        for (k, v) in extra_env {
            c.env(k, v);
        }
        c
    }

    pub fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String, String) {
        let out = self.command(args, extra_env).output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// `vitrify run …` that must end with exit `want`.
    pub fn run_expect(&self, want: i32, args: &[&str], extra_env: &[(&str, &str)]) -> String {
        let (code, stdout, stderr) = self.run(args, extra_env);
        assert_eq!(code, want, "stdout:\n{stdout}\nstderr:\n{stderr}");
        stdout
    }

    pub fn start(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Background {
        let out = self.tmp.path().join(format!(
            "run-{}.out",
            std::fs::read_dir(self.tmp.path()).unwrap().count()
        ));
        let err = out.with_extension("err");
        let child = self
            .command(args, extra_env)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(std::fs::File::create(&err).unwrap())
            .spawn()
            .unwrap();
        Background { child, out, err }
    }

    pub fn run_json(&self) -> serde_json::Value {
        std::fs::read_to_string(self.root.join(".vitrify/run.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    pub fn job_logs(&self) -> PathBuf {
        let job = self.run_json()["job_id"].as_str().unwrap().to_string();
        self.root.join(".vitrify/logs").join(job)
    }

    /// The latest job's `events.jsonl`, one value per line.
    pub fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.job_logs().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn on_main(&self, path: &str) -> String {
        git(&self.root, &["show", &format!("main:{path}")])
    }

    pub fn edit(&self, rel: &str, from: &str, to: &str) {
        let path = self.root.join(rel);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{rel} has no `{from}`");
        std::fs::write(&path, text.replace(from, to)).unwrap();
    }

    pub fn commit(&self, message: &str) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", message]);
    }
}

pub struct Background {
    pub child: Child,
    pub out: PathBuf,
    pub err: PathBuf,
}

impl Background {
    pub fn signal(&self, sig: &str) {
        let ok = Command::new("/bin/kill")
            .args([sig, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {sig}");
    }

    pub fn stdout(&self) -> String {
        std::fs::read_to_string(&self.out).unwrap_or_default()
    }

    pub fn finish(mut self) -> (std::process::ExitStatus, String, String) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (
                    status,
                    std::fs::read_to_string(&self.out).unwrap_or_default(),
                    std::fs::read_to_string(&self.err).unwrap_or_default(),
                );
            }
            assert!(Instant::now() < deadline, "the run did not end");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One `STUB_LOG` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubLine {
    pub event: String,
    pub sdd: String,
    pub unit: String,
    pub seq: i64,
    pub ms: u128,
}

pub fn stub_log(path: &Path) -> Vec<StubLine> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some(StubLine {
                event: f.first()?.to_string(),
                sdd: f.get(1)?.to_string(),
                unit: f.get(2)?.to_string(),
                seq: f.get(3)?.parse().ok()?,
                ms: f.get(4)?.parse().ok()?,
            })
        })
        .collect()
}

/// Worker sessions from a `STUB_LOG`: `(sdd, unit, seq, start, end)`,
/// `end` `None` for a session that never ended.
pub fn sessions(log: &[StubLine]) -> Vec<(String, String, i64, u128, Option<u128>)> {
    log.iter()
        .filter(|l| l.event == "start")
        .map(|s| {
            let end = log
                .iter()
                .find(|e| e.event == "end" && e.sdd == s.sdd && e.unit == s.unit && e.seq == s.seq)
                .map(|e| e.ms);
            (s.sdd.clone(), s.unit.clone(), s.seq, s.ms, end)
        })
        .collect()
}

/// The `[slug] dispatch U-###` lines, as `slug U-###`, in order.
pub fn dispatched(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| {
            let (lane, rest) = l.strip_prefix('[')?.split_once("] dispatch ")?;
            Some(format!("{lane} {}", rest.split_whitespace().next()?))
        })
        .collect()
}
