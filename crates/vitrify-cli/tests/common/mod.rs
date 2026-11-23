//! A two-SDD repository whose units are Verified the way an integration
//! leaves them, for the incremental-build tests.
//!
//! `alpha` (bundle `specs/alpha`, scope `a/`): U-001 the foundation, owning
//! `a/one.txt` and producing contract C-001; U-002 owning `a/two.txt`,
//! depending on U-001. `beta` (bundle `specs/beta`, scope `b/`): U-001
//! owning `b/one.txt`, consuming a pin of alpha C-001 and naming the pin in
//! `depends_on` — so it waits for alpha U-001; U-002 owning `b/two.txt`,
//! depending on nothing. Both at schema 2.1, each with a §5 Test command.
//!
//! Seeded on `main` in two commits, as an integration does it: the code
//! (the anchor every `verified` event records), then the stamps, journals
//! and ledgers.
//!
//! [`Project::with_tests`] builds the same repository with alpha at a given
//! schema, listing `a/tests/` under §6 `tests:`: alpha U-001 also owns the
//! test `a/tests/t1.txt` (generated), and nobody owns `a/tests/pre.txt`
//! (pre-existing).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use vitrify_core::journal::{self, Event};
use vitrify_core::model::{self, UtcTime};

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

pub fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// An SDD's core file; `tests` is §6 text before its `unmanaged:` line.
fn sdd_md(slug: &str, schema: &str, scope: &str, test: &str, tests: &str) -> String {
    format!(
        "---\nsdd: {slug}\nschema_version: {schema}\ntitle: {slug}\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - {scope}\n---\n\n# {slug}\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `{test}`\n- Lint:      `true`\n\n## System Layout & Ownership\n\n{tests}unmanaged: []\n"
    )
}

fn unit_md(uid: &str, owns: &str, deps: &str, consumes: &str, row: &str) -> String {
    format!(
        "---\nunit: {uid}\ntitle: {uid}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: [{deps}]\nparallel_group: \u{2014}\nowns: [{owns}]\nconsumes: [{consumes}]\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n---\n\n# {uid}\n\n## Requirements\n\n- **{uid}.R1** \u{2014} THE SYSTEM SHALL ship `{owns}`.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `{row}` | exit 0 |\n"
    )
}

/// The repository, its vitrify home, and a scratch area.
pub struct Project {
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    /// The commit every `verified` event records.
    pub anchor: String,
}

pub const UNITS: [(&str, &str); 4] = [
    ("specs/alpha", "U-001"),
    ("specs/alpha", "U-002"),
    ("specs/beta", "U-001"),
    ("specs/beta", "U-002"),
];

impl Project {
    pub fn new() -> Project {
        Project::build(None)
    }

    /// The repository of [`Project::new`], with alpha declaring
    /// `schema_version: <schema>` and listing `a/tests/` under §6 `tests:`;
    /// alpha U-001 also owns `a/tests/t1.txt`, and `a/tests/t1.txt` and
    /// `a/tests/pre.txt` are committed with the code.
    pub fn with_tests(schema: &str) -> Project {
        Project::build(Some(schema))
    }

    /// The shared builder: `tests` is alpha's schema when it lists tests.
    fn build(tests: Option<&str>) -> Project {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let home = tmp.path().join("vhome");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(tmp.path().join("t")).unwrap();
        std::fs::write(home.join("config.toml"), vitrify_core::content::CONFIG.body).unwrap();
        let root = root.canonicalize().unwrap();
        let (alpha_schema, alpha_tests, alpha_owns) = match tests {
            None => ("2.1", "", "a/one.txt"),
            Some(schema) => (
                schema,
                "tests:\n  - a/tests/\n",
                "a/one.txt, a/tests/t1.txt",
            ),
        };
        let mut files: Vec<(&str, String)> = vec![
            (
                "specs/alpha/sdd.md",
                sdd_md("alpha", alpha_schema, "a/", "test -f a/one.txt", alpha_tests),
            ),
            (
                "specs/alpha/units/U-001-one.md",
                unit_md("U-001", alpha_owns, "", "", "grep -q one a/one.txt"),
            ),
            (
                "specs/alpha/units/U-002-two.md",
                unit_md("U-002", "a/two.txt", "U-001", "", "grep -q two a/two.txt"),
            ),
            (
                "specs/alpha/contracts/C-001-greeting.md",
                "# C-001 \u{2014} Greeting file\n\n- Producer: U-001\n- Consumers: \u{2014}\n- Stability: Frozen\n\n## Surface\n\n```text\n# a/one.txt\none\n```\n".to_string(),
            ),
            (
                "specs/beta/sdd.md",
                sdd_md("beta", "2.1", "b/", "test -f b/one.txt", ""),
            ),
            (
                "specs/beta/units/U-001-one.md",
                unit_md("U-001", "b/one.txt", "C-001", "C-001", "test -f b/one.txt"),
            ),
            (
                "specs/beta/units/U-002-two.md",
                unit_md("U-002", "b/two.txt", "", "", "test -f b/two.txt"),
            ),
            (
                "specs/beta/contracts/C-001-greeting.md",
                "# C-001 \u{2014} Greeting file (pinned)\n\n- Producer: external \u{2014} alpha, C-001\n- Consumers: U-001\n- Stability: Frozen by its producer\n- Pin: alpha C-001 @ 000000000000\n".to_string(),
            ),
            ("a/one.txt", "one\n".to_string()),
            ("a/two.txt", "two\n".to_string()),
            ("b/one.txt", "b one\n".to_string()),
            ("b/two.txt", "b two\n".to_string()),
        ];
        if tests.is_some() {
            files.push(("a/tests/t1.txt", "t1\n".to_string()));
            files.push(("a/tests/pre.txt", "pre\n".to_string()));
        }
        for (rel, content) in &files {
            write(&root, rel, content);
        }
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "t@test"]);
        git(&root, &["config", "user.name", "t"]);
        let mut p = Project {
            tmp,
            root,
            home,
            anchor: String::new(),
        };
        let (ok, _, err) = p.vitrify(&["pin", "sync", "--sdd", "specs/beta"]);
        assert!(ok, "pin sync: {err}");
        p.commit("code");
        p.anchor = git(&p.root, &["rev-parse", "--short=7", "HEAD"]);
        for (sdd, unit) in UNITS {
            let (ok, _, err) = p.vitrify(&[
                "trace", "record", "--unit", unit, "--stamp", "verified", "--commit", &p.anchor,
                "--sdd", sdd,
            ]);
            assert!(ok, "trace record {sdd} {unit}: {err}");
            p.event(sdd, "verified", Some(unit), "integrator", &p.anchor.clone());
        }
        for sdd in ["specs/alpha", "specs/beta"] {
            let (ok, _, err) = p.vitrify(&["ledger", "regen", "--sdd", sdd]);
            assert!(ok, "ledger regen {sdd}: {err}");
        }
        p.commit("verify");
        p
    }

    /// Run vitrify in the repository.
    pub fn vitrify(&self, args: &[&str]) -> (bool, String, String) {
        self.vitrify_in(&self.root, args)
    }

    pub fn vitrify_in(&self, dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .args(args)
            .current_dir(dir)
            .env("VITRIFY_HOME", &self.home)
            .env("TMPDIR", self.tmp.path().join("t"))
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// Append an event to the journal of the SDD at `sdd`.
    pub fn event(&self, sdd: &str, event: &str, unit: Option<&str>, actor: &str, commit: &str) {
        let handle = model::resolve_sdd(
            &self.root,
            Some(&self.root.join(sdd).to_string_lossy()),
            &self.root,
        )
        .unwrap();
        let (events, _) = journal::read(&handle).unwrap();
        let e = Event {
            seq: journal::next_seq(&events),
            at: UtcTime::now().format(),
            event: event.to_string(),
            unit: unit.map(str::to_string),
            actor: Some(actor.to_string()),
            sdd: "1.0.0".to_string(),
            commit: Some(commit.to_string()),
            ..Default::default()
        };
        journal::append_line(&handle, &journal::render_line(&e)).unwrap();
    }

    /// Append an `audit` event recording a pass of the §5 Test command of
    /// the SDD at `sdd`, anchored at `commit`: its suite anchor.
    pub fn suite_pass(&self, sdd: &str, commit: &str) {
        let handle = model::resolve_sdd(
            &self.root,
            Some(&self.root.join(sdd).to_string_lossy()),
            &self.root,
        )
        .unwrap();
        let (events, _) = journal::read(&handle).unwrap();
        let e = Event {
            seq: journal::next_seq(&events),
            at: UtcTime::now().format(),
            event: "audit".to_string(),
            actor: Some("audit".to_string()),
            findings: Some(0),
            suite: Some("pass".to_string()),
            sdd: "1.0.0".to_string(),
            commit: Some(commit.to_string()),
            ..Default::default()
        };
        journal::append_line(&handle, &journal::render_line(&e)).unwrap();
    }

    /// Commit everything; returns the new head in full.
    pub fn commit(&self, message: &str) -> String {
        git(&self.root, &["add", "-A"]);
        git(
            &self.root,
            &["commit", "-q", "--allow-empty", "-m", message],
        );
        self.head()
    }

    pub fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"])
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    pub fn write(&self, rel: &str, content: &str) {
        write(&self.root, rel, content);
    }
}
