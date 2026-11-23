//! External contracts at the CLI: the lint check (L9) and `vitrify pin
//! list|sync` on the seeded fixtures — `pin-states`, one external contract
//! in each state, and `pin-multi`, pins read through by the reading set and
//! the worker's prompt — against their finding sets and golden outputs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let t = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// A scratch copy of a seeded fixture's project, with a vitrify home of its
/// own beside it.
struct Scratch {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    fixture: PathBuf,
}

fn scratch(fixture: &str) -> Scratch {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let fixture = repo_root().join("fixtures/seeded").join(fixture);
    copy_dir(&fixture.join("project"), &base.join("repo"));
    set_up_home(&base.join("home"));
    Scratch {
        root: base.join("repo"),
        home: base.join("home"),
        fixture,
        _tmp: tmp,
    }
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Scratch {
    fn run(&self, args: &[&str]) -> Ran {
        let out: Output = Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .args(args)
            .current_dir(&self.root)
            .env("VITRIFY_HOME", &self.home)
            .output()
            .unwrap();
        Ran {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    fn ok(&self, args: &[&str]) -> Ran {
        let ran = self.run(args);
        assert_eq!(ran.code, 0, "{args:?}\n{}\n{}", ran.stdout, ran.stderr);
        ran
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn edit(&self, rel: &str, from: &str, to: &str) {
        let text = self.read(rel);
        assert!(text.contains(from), "{rel} has no `{from}`");
        self.write(rel, &text.replace(from, to));
    }

    fn git_init(&self) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "t@test"],
            &["config", "user.name", "t"],
            &["add", "-A"],
            &["commit", "-q", "-m", "seed"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&self.root)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }

    fn lint_set(&self, args: &[&str]) -> Vec<(String, String)> {
        let mut all = vec!["lint", "--json"];
        all.extend(args);
        finding_set(&self.run(&all).stdout)
    }

    fn expected_set(&self) -> Vec<(String, String)> {
        finding_set(&std::fs::read_to_string(self.fixture.join("expected.json")).unwrap())
    }
}

fn finding_set(json: &str) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {json}"));
    let mut set: Vec<(String, String)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["check"].as_str().unwrap().to_string(),
                x["subject"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    set.sort();
    set.dedup();
    set
}

fn golden(dir: &str, name: &str) -> String {
    std::fs::read_to_string(repo_root().join("fixtures/golden").join(dir).join(name)).unwrap()
}

/// The body of a contract file: from its `## Surface` line to its end.
fn body(text: &str) -> &str {
    &text[text.find("\n## Surface").expect("a Surface heading") + 1..]
}

// ------------------------------------------------------------- lint (L9)

#[test]
fn seeded_fixtures_report_exactly_their_own_finding() {
    for fixture in ["pin-states", "pin-multi"] {
        let s = scratch(fixture);
        // the consumer: its stale pin, its missing authority — and nothing
        // else
        let ran = s.run(&["lint", "--sdd", "specs/beta", "--json"]);
        assert_eq!(ran.code, 2, "{fixture}");
        let got = finding_set(&ran.stdout);
        assert_eq!(got, s.expected_set(), "{fixture}");
        assert!(got.iter().all(|(check, _)| check == "L9"), "{fixture}");
        // the producer and the single-file consumer are clean
        for sdd in ["specs/alpha", "specs/gamma.sdd.md"] {
            let ran = s.run(&["lint", "--sdd", sdd]);
            assert_eq!(ran.code, 0, "{fixture} {sdd}: {}", ran.stdout);
            assert_eq!(ran.stdout, "lint clean (L1–L12)\n");
        }
    }
}

/// The lines of `out` that begin with `prefix`.
fn lines_starting<'a>(out: &'a str, prefix: &str) -> Vec<&'a str> {
    out.lines().filter(|l| l.starts_with(prefix)).collect()
}

#[test]
fn lint_names_the_authority_and_the_cause() {
    let s = scratch("pin-states");
    let ran = s.run(&["lint", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 2);
    assert_eq!(ran.stdout.lines().count(), 2, "{}", ran.stdout);
    for prefix in [
        "[L9] C-002: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97)",
        "[L9] C-003: its authority cannot be found: alpha has no contract C-009",
    ] {
        assert_eq!(
            lines_starting(&ran.stdout, prefix).len(),
            1,
            "{prefix}\n{}",
            ran.stdout
        );
    }

    // a contract that names another SDD's contract and has no `Pin:` line
    // is no transcription: the cause says what to write
    s.edit(
        "specs/beta/contracts/C-001-greeting.md",
        "- Pin: alpha C-001 @ ef9dc34b4daf\n",
        "",
    );
    let ran = s.run(&["lint", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 2);
    assert_eq!(
        lines_starting(
            &ran.stdout,
            "[L9] C-001: its Producer line names alpha C-001 and it has no `Pin:` line — write `- Pin: alpha C-001` in its header, then `vitrify pin sync` records the hash"
        )
        .len(),
        1,
        "{}",
        ran.stdout
    );
    let ran = s.run(&["pin", "list", "--sdd", "specs/beta"]);
    assert!(
        ran.stdout
            .starts_with("beta C-001  invalid            pin of alpha C-001"),
        "{}",
        ran.stdout
    );
}

#[test]
fn a_renamed_producer_is_a_finding_never_an_unchecked_transcription() {
    // every pin names `alpha`; the producer's slug becomes `first`. A check
    // that only knew the slugs it could find would file these as
    // transcriptions and call the SDD clean.
    let s = scratch("pin-states");
    s.edit("specs/alpha/sdd.md", "sdd: alpha", "sdd: first");
    assert_eq!(
        s.lint_set(&["--sdd", "specs/beta"]),
        [
            ("L9".to_string(), "C-001".to_string()),
            ("L9".to_string(), "C-002".to_string()),
            ("L9".to_string(), "C-003".to_string()),
        ]
    );
    let ran = s.run(&["pin", "list", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stdout.contains(
            "its authority cannot be found: no SDD in this repository has the slug `alpha`"
        ),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .ends_with("4 external contracts: 3 missing-authority · 1 unchecked\n"),
        "{}",
        ran.stdout
    );
    // a sync has nothing to read a hash from, and says so
    let ran = s.run(&["pin", "sync", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stdout.ends_with("0 synced · 3 not synced\n"),
        "{}",
        ran.stdout
    );
    assert!(!ran.stderr.contains("next:"), "{}", ran.stderr);
}

// --------------------------------------------------------------- pin list

#[test]
fn pin_list_matches_golden() {
    for fixture in ["pin-states", "pin-multi"] {
        let s = scratch(fixture);
        let ran = s.run(&["pin", "list", "--all", "--json"]);
        assert_eq!(ran.code, 2, "{fixture}: a stale contract is not clean");
        assert_eq!(ran.stdout, golden(fixture, "pin-list.json"), "{fixture}");
        // the same inside a git repository, where discovery asks git
        s.git_init();
        let ran = s.run(&["pin", "list", "--all", "--json"]);
        assert_eq!(ran.stdout, golden(fixture, "pin-list.json"), "{fixture}");
    }
    let s = scratch("pin-states");
    let ran = s.run(&["pin", "list", "--all"]);
    assert_eq!(ran.code, 2);
    assert_eq!(ran.stdout, golden("pin-states", "pin-list.txt"));
}

#[test]
fn pin_list_takes_one_sdd_or_all() {
    let s = scratch("pin-states");
    // several SDDs and no flag: refused, the candidates listed as the
    // `--sdd` values that name them
    let ran = s.run(&["pin", "list"]);
    assert_eq!(ran.code, 4);
    assert_eq!(
        ran.stderr,
        "error: multiple SDDs found; pass --sdd <root>:\n  --sdd specs/alpha\n  --sdd specs/beta\n  --sdd specs/gamma.sdd.md\n"
    );
    // --all and --sdd do not combine
    assert_eq!(
        s.run(&["pin", "list", "--all", "--sdd", "specs/beta"]).code,
        2
    );
    // an SDD with no external contract is clean, and says what it has
    let ran = s.ok(&["pin", "list", "--sdd", "specs/alpha"]);
    assert_eq!(ran.stdout, "no external contracts\n");
    let ran = s.ok(&["pin", "list", "--sdd", "specs/gamma.sdd.md"]);
    assert!(
        ran.stdout.ends_with("1 external contract: 1 ok\n"),
        "{}",
        ran.stdout
    );
    // a transcription is listed as unchecked — never as ok — and is no
    // finding: there is nothing in the repository to hold it against
    s.edit(
        "specs/beta/contracts/C-002-farewell.md",
        "@ 446a992352a3",
        "@ 28c9da8acc97",
    );
    std::fs::remove_file(s.root.join("specs/beta/contracts/C-003-ledger.md")).unwrap();
    let ran = s.ok(&["pin", "list", "--sdd", "specs/beta"]);
    assert!(
        ran.stdout
            .contains("beta C-004  unchecked          transcription"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .ends_with("3 external contracts: 2 ok · 1 unchecked\n"),
        "{}",
        ran.stdout
    );

    // no SDD at all: refused, naming where to start
    let empty = tempfile::tempdir().unwrap();
    for args in [&["pin", "list", "--all"][..], &["pin", "list"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_vitrify"))
            .args(args)
            .current_dir(empty.path())
            .env("VITRIFY_HOME", &s.home)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(4));
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "error: no SDD found in repository → start one: vitrify brief authoring (or vitrify new sdd inside specs/)\n"
        );
    }
}

// --------------------------------------------------------------- pin sync

#[test]
fn sync_rewrites_the_stale_pin_and_names_the_units_it_staled() {
    let s = scratch("pin-states");
    // beta's U-001 was built and verified against the pins as they stand
    for stamp in ["built", "verified"] {
        s.ok(&[
            "trace",
            "record",
            "--unit",
            "U-001",
            "--sdd",
            "specs/beta",
            "--stamp",
            stamp,
            "--commit",
            "abc1234",
        ]);
    }
    s.ok(&["trace", "verify", "--unit", "U-001", "--sdd", "specs/beta"]);
    let in_step = s.read("specs/beta/contracts/C-001-greeting.md");
    let before = s.read("specs/beta/contracts/C-002-farewell.md");

    let ran = s.run(&["pin", "sync", "--sdd", "specs/beta"]);
    // the missing authority is still missing: exit 2, and it is named
    assert_eq!(ran.code, 2, "{}\n{}", ran.stdout, ran.stderr);
    assert_eq!(
        ran.stdout,
        "synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md\n\
         beta: - Staleness applied: U-001 → Stale (consume C-002)\n\
         1 synced · 1 not synced\n"
    );
    assert_eq!(
        ran.stderr,
        "not synced: beta C-003: its authority cannot be found: alpha has no contract C-009\n\
         next: write beta's change entry around its line, then run vitrify change apply --sdd specs/beta\n"
    );

    // one line changed: the hash; nothing else of beta's, nothing of the
    // producer's
    assert_eq!(
        s.read("specs/beta/contracts/C-002-farewell.md"),
        before.replace("@ 446a992352a3", "@ 28c9da8acc97")
    );
    assert_eq!(s.read("specs/beta/contracts/C-001-greeting.md"), in_step);
    assert_eq!(
        s.read("specs/alpha/contracts/C-002-farewell.md"),
        std::fs::read_to_string(
            s.fixture
                .join("project/specs/alpha/contracts/C-002-farewell.md")
        )
        .unwrap()
    );

    // the sync is what delivers the producer's change to the consumer's
    // staleness: the unit's recorded trace no longer matches
    let ran = s.run(&["trace", "verify", "--unit", "U-001", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 3);
    assert_eq!(ran.stdout, "U-001: STALE — mismatched keys: C-002\n");
    // … and the pin is no longer a lint finding
    assert_eq!(
        s.lint_set(&["--sdd", "specs/beta", "--built"]),
        [("L9".to_string(), "C-003".to_string())]
    );

    // a second sync writes nothing, and names no next step
    let ran = s.run(&["pin", "sync", "--sdd", "specs/beta", "--json"]);
    assert_eq!(ran.code, 2);
    let v: serde_json::Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(v["synced"].as_array().unwrap().len(), 0);
    assert_eq!(v["unresolved"][0]["contract"], "C-003");
    assert_eq!(v["unresolved"][0]["state"], "missing-authority");
    assert!(!ran.stderr.contains("next:"), "{}", ran.stderr);
}

#[test]
fn sync_reports_as_json_too() {
    let s = scratch("pin-states");
    s.ok(&[
        "trace",
        "record",
        "--unit",
        "U-001",
        "--sdd",
        "specs/beta",
        "--stamp",
        "built",
    ]);
    let ran = s.run(&["pin", "sync", "--all", "--json"]);
    assert_eq!(ran.code, 2);
    let v: serde_json::Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(
        v["synced"],
        serde_json::json!([{
            "sdd": "beta",
            "contract": "C-002",
            "file": "specs/beta/contracts/C-002-farewell.md",
            "authority": {"sdd": "alpha", "contract": "C-002"},
            "hash": "28c9da8acc97"
        }])
    );
    assert_eq!(
        v["staleness"],
        serde_json::json!([{
            "sdd": "beta",
            "stale": [{"contract": "C-002", "units": ["U-001"]}],
            "line": "- Staleness applied: U-001 → Stale (consume C-002)"
        }])
    );
    assert_eq!(
        v["unresolved"],
        serde_json::json!([{
            "sdd": "beta",
            "contract": "C-003",
            "state": "missing-authority",
            "detail": "its authority cannot be found: alpha has no contract C-009"
        }])
    );
}

#[test]
fn a_dry_run_prints_what_a_sync_would_and_writes_nothing() {
    let s = scratch("pin-states");
    for stamp in ["built", "verified"] {
        s.ok(&[
            "trace",
            "record",
            "--unit",
            "U-001",
            "--sdd",
            "specs/beta",
            "--stamp",
            stamp,
            "--commit",
            "abc1234",
        ]);
    }
    let before = tree_of(&s.root);
    let dry = s.run(&["pin", "sync", "--sdd", "specs/beta", "--dry-run"]);
    // the lines a sync prints, the units it would stale included …
    assert_eq!(dry.code, 2, "{}\n{}", dry.stdout, dry.stderr);
    assert_eq!(
        dry.stdout,
        "synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md\n\
         beta: - Staleness applied: U-001 → Stale (consume C-002)\n\
         1 synced · 1 not synced\n"
    );
    // … no next step, since nothing was written
    assert_eq!(
        dry.stderr,
        "not synced: beta C-003: its authority cannot be found: alpha has no contract C-009\n\
         dry run: nothing written\n"
    );
    assert!(tree_of(&s.root) == before, "a dry run wrote");
    let json = s.run(&["pin", "sync", "--sdd", "specs/beta", "--dry-run", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(v["dry_run"], true);
    assert_eq!(
        v["staleness"][0]["line"],
        "- Staleness applied: U-001 → Stale (consume C-002)"
    );
    assert_eq!(v["unresolved"].as_array().unwrap().len(), 1);
    assert!(tree_of(&s.root) == before, "a dry run wrote");

    // the sync itself prints the same
    let ran = s.run(&["pin", "sync", "--sdd", "specs/beta"]);
    assert_eq!(ran.stdout, dry.stdout);
    let v: serde_json::Value = serde_json::from_str(
        &s.run(&["pin", "sync", "--sdd", "specs/beta", "--json"])
            .stdout,
    )
    .unwrap();
    assert_eq!(v["dry_run"], false);
}

#[test]
fn a_dry_run_reads_a_single_files_pin_as_the_sync_would_leave_it() {
    let s = scratch("pin-states");
    s.ok(&[
        "trace",
        "record",
        "--unit",
        "U-001",
        "--sdd",
        "specs/gamma.sdd.md",
        "--stamp",
        "built",
        "--commit",
        "abc1234",
    ]);
    s.edit(
        "specs/alpha/contracts/C-001-greeting.md",
        "- THE SYSTEM SHALL write exactly one line and nothing after it.\n",
        "- THE SYSTEM SHALL write exactly one line and nothing after it.\n\n### Encoding\n\n- UTF-8, no byte-order mark.\n",
    );
    let before = s.read("specs/gamma.sdd.md");
    let dry = s.ok(&["pin", "sync", "--sdd", "specs/gamma.sdd.md", "--dry-run"]);
    assert_eq!(
        dry.stdout,
        "synced gamma C-001 ← alpha C-001  specs/gamma.sdd.md\n\
         gamma: - Staleness applied: U-001 → Stale (consume C-001)\n\
         1 synced\n"
    );
    assert_eq!(dry.stderr, "dry run: nothing written\n");
    assert_eq!(s.read("specs/gamma.sdd.md"), before);
    let ran = s.ok(&["pin", "sync", "--sdd", "specs/gamma.sdd.md"]);
    assert_eq!(ran.stdout, dry.stdout);
    assert_ne!(s.read("specs/gamma.sdd.md"), before);
}

/// Every file under `dir` and its bytes.
fn tree_of(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn sync_rewrites_a_single_file_sdds_pin_in_place() {
    let s = scratch("pin-states");
    // the producer adds a rule; gamma's pin is now behind
    s.edit(
        "specs/alpha/contracts/C-001-greeting.md",
        "- THE SYSTEM SHALL write exactly one line and nothing after it.\n",
        "- THE SYSTEM SHALL write exactly one line and nothing after it.\n\n### Encoding\n\n- UTF-8, no byte-order mark.\n",
    );
    assert_eq!(
        s.lint_set(&["--sdd", "specs/gamma.sdd.md"]),
        [("L9".to_string(), "C-001".to_string())]
    );
    let before = s.read("specs/gamma.sdd.md");
    let ran = s.ok(&["pin", "sync", "--sdd", "specs/gamma.sdd.md"]);
    assert_eq!(
        ran.stdout,
        "synced gamma C-001 ← alpha C-001  specs/gamma.sdd.md\n\
         gamma: - Staleness applied: —\n\
         1 synced\n"
    );
    assert_eq!(
        ran.stderr,
        "next: write gamma's change entry around its line, then run vitrify change apply --sdd specs/gamma.sdd.md\n"
    );
    // only the pin's hash moved; the region still ends where it did
    let text = s.read("specs/gamma.sdd.md");
    let changed: Vec<(&str, &str)> = before
        .lines()
        .zip(text.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(before.lines().count(), text.lines().count());
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].1.starts_with("- Pin: alpha C-001 @ "));
    // the file is still the SDD it was, and its unit reads the new body
    assert_eq!(
        s.ok(&["lint", "--sdd", "specs/gamma.sdd.md"]).stdout,
        "lint clean (L1–L12)\n"
    );
    let ran = s.ok(&["locate", "--units", "--sdd", "specs/gamma.sdd.md"]);
    assert!(ran.stdout.starts_with("U-001    Ready"), "{}", ran.stdout);
    let ran = s.ok(&["readset", "--unit", "U-001", "--sdd", "specs/gamma.sdd.md"]);
    assert!(
        ran.stdout
            .ends_with("specs/alpha/contracts/C-001-greeting.md  ::  Surface onward\n"),
        "{}",
        ran.stdout
    );
}

// ------------------------------------------------------------------- pins

#[test]
fn readset_follows_a_pin_to_its_authoritys_body() {
    let s = scratch("pin-multi");
    let root = s.root.to_str().unwrap();
    for (sdd, name) in [
        ("specs/beta", "readset-beta-U-001.json"),
        ("specs/gamma.sdd.md", "readset-gamma-U-001.json"),
    ] {
        let ran = s.ok(&["readset", "--unit", "U-001", "--sdd", sdd, "--json"]);
        assert_eq!(
            ran.stdout,
            golden("pin-multi", name).replace("{ROOT}", root),
            "{sdd}"
        );
    }
    // the body counts against the budget as the unit's own reading
    let over = s.run(&[
        "readset",
        "--unit",
        "U-001",
        "--sdd",
        "specs/beta",
        "--budget",
        "0",
    ]);
    let total: u64 = over
        .stderr
        .split("totals ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .unwrap()
        .parse()
        .unwrap();
    let size = |rel: &str| s.read(rel).len() as u64;
    let authority = s.read("specs/alpha/contracts/C-001-greeting.md");
    let sdd = s.read("specs/beta/sdd.md");
    let section = |name: &str| {
        let from = sdd.find(&format!("## {name}")).unwrap();
        let rest = &sdd[from + 3..];
        (rest.find("\n## ").map_or(rest.len(), |i| i + 1) + 3) as u64
    };
    assert!(total > size("specs/beta/contracts/C-001-greeting.md") + body(&authority).len() as u64);
    assert_eq!(
        total,
        section("Ground Rules")
            + section("Technology Decisions")
            + size("specs/beta/contracts/C-001-greeting.md")
            + body(&authority).len() as u64
            // the unit's own entry, without the fields vitrify writes
            + size("specs/beta/units/U-001-one.md")
            - "status: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\nbuilt: —\nverified: —\ntrace: —\n"
                .len() as u64
    );
}

#[test]
fn a_pinned_entry_is_traced_as_written() {
    let s = scratch("pin-multi");
    let ran = s.ok(&[
        "trace",
        "record",
        "--unit",
        "U-001",
        "--stamp",
        "built",
        "--sdd",
        "specs/beta",
    ]);
    let got: serde_json::Value = serde_json::from_str(&ran.stdout).unwrap();
    let want: serde_json::Value =
        serde_json::from_str(&golden("pin-multi", "trace-beta-U-001.json")).unwrap();
    assert_eq!(got["recorded"], want["recorded"]);
}

#[test]
fn a_stale_pin_is_never_followed() {
    let s = scratch("pin-multi");
    let ran = s.run(&["readset", "--unit", "U-002", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 4);
    assert_eq!(
        ran.stderr,
        "error: pinned contract C-002 does not resolve: its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97) → vitrify pin sync --all\n"
    );
    assert!(ran.stdout.is_empty());
    // nor is a worker's prompt generated from it
    let ran = s.run(&[
        "brief",
        "build-unit",
        "--unit",
        "U-002",
        "--holder",
        "h1",
        "--sdd",
        "specs/beta",
    ]);
    assert_eq!(ran.code, 4, "{}", ran.stdout);
    assert!(
        ran.stderr
            .contains("pinned contract C-002 does not resolve"),
        "{}",
        ran.stderr
    );
    // lint reports it once — as the pin's own finding, not as a reading
    // set that does not resolve as well
    assert_eq!(
        s.lint_set(&["--sdd", "specs/beta"]),
        [("L9".to_string(), "C-002".to_string())]
    );
}

#[test]
fn a_worker_prompt_inlines_the_pinned_contract_and_the_body_it_stands_for() {
    let s = scratch("pin-multi");
    let ran = s.ok(&[
        "brief",
        "build-unit",
        "--unit",
        "U-001",
        "--holder",
        "h1",
        "--sdd",
        "specs/beta",
    ]);
    let own = s.read("specs/beta/contracts/C-001-greeting.md");
    let authority = s.read("specs/alpha/contracts/C-001-greeting.md");
    let prompt = ran.stdout;
    let entry = format!(
        "### Reading 4 — {}\n\n{}",
        s.root
            .join("specs/beta/contracts/C-001-greeting.md")
            .display(),
        own.trim_end()
    );
    let stands_for = format!(
        "### Reading 5 — {}  ::  Surface onward\n\n{}",
        s.root
            .join("specs/alpha/contracts/C-001-greeting.md")
            .display(),
        body(&authority).trim_end()
    );
    assert!(prompt.contains(&entry), "{prompt}");
    assert!(prompt.ends_with(&format!("{stands_for}\n")), "{prompt}");
    // the producer's own header — its units, its consumers — is not read
    assert!(!prompt.contains("- Producer: U-001"), "{prompt}");
}

#[test]
fn sync_re_pins_and_names_the_units_it_staled() {
    let s = scratch("pin-multi");
    s.ok(&[
        "trace",
        "record",
        "--unit",
        "U-002",
        "--sdd",
        "specs/beta",
        "--stamp",
        "built",
        "--commit",
        "abc1234",
    ]);
    let before = s.read("specs/beta/contracts/C-002-farewell.md");
    let ran = s.ok(&["pin", "sync", "--all"]);
    assert_eq!(
        ran.stdout,
        "synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md\n\
         beta: - Staleness applied: U-002 → Stale (consume C-002)\n\
         1 synced\n"
    );
    assert_eq!(
        ran.stderr,
        "next: write beta's change entry around its line, then run vitrify change apply --sdd specs/beta\n"
    );
    // one line changed: the hash
    assert_eq!(
        s.read("specs/beta/contracts/C-002-farewell.md"),
        before.replace("@ 446a992352a3", "@ 28c9da8acc97")
    );
    assert_eq!(
        s.ok(&["lint", "--all", "--built"]).stdout.lines().count(),
        3
    );
    assert_eq!(
        s.run(&["trace", "verify", "--unit", "U-002", "--sdd", "specs/beta"])
            .stdout,
        "U-002: STALE — mismatched keys: C-002\n"
    );
    // the pin now leads to the body, and the unit can be handed out
    let ran = s.ok(&["readset", "--unit", "U-002", "--sdd", "specs/beta"]);
    assert!(
        ran.stdout
            .ends_with("specs/alpha/contracts/C-002-farewell.md  ::  Surface onward\n"),
        "{}",
        ran.stdout
    );
    assert_eq!(
        s.ok(&["pin", "sync", "--all"]).stdout,
        "nothing to sync (3 in step)\n"
    );

    // a pin written by hand, without its hash: a sync records it
    s.edit(
        "specs/beta/contracts/C-001-greeting.md",
        "- Pin: alpha C-001 @ ef9dc34b4daf\n",
        "- Pin: alpha C-001\n",
    );
    let ran = s.run(&["lint", "--sdd", "specs/beta"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stdout.contains("its pin records no hash yet"),
        "{}",
        ran.stdout
    );
    let ran = s.ok(&["pin", "sync", "--sdd", "specs/beta"]);
    assert!(
        ran.stdout
            .starts_with("synced beta C-001 ← alpha C-001  specs/beta/contracts/C-001-greeting.md"),
        "{}",
        ran.stdout
    );
    assert!(
        s.read("specs/beta/contracts/C-001-greeting.md")
            .ends_with("- Pin: alpha C-001 @ ef9dc34b4daf\n")
    );
}

#[test]
fn a_schema_version_this_vitrify_does_not_read_is_refused() {
    let s = scratch("pin-multi");
    s.edit(
        "specs/beta/sdd.md",
        "schema_version: 2.0",
        "schema_version: 1.1",
    );
    for args in [
        &["lint", "--sdd", "specs/beta"][..],
        &["readset", "--unit", "U-001", "--sdd", "specs/beta"],
        &["locate", "--units", "--sdd", "specs/beta"],
        &["audit", "--sdd", "specs/beta", "--skip-rotten"],
        &["pin", "list", "--sdd", "specs/beta"],
    ] {
        let ran = s.run(args);
        assert_eq!(ran.code, 4, "{args:?}: {}", ran.stdout);
        assert!(
            ran.stderr
                .contains("schema_version 1.1 is not one this vitrify reads (2.0, 2.1 and 2.2)"),
            "{args:?}: {}",
            ran.stderr
        );
    }
    // it is still found and listed, version and all
    let ran = s.ok(&["locate"]);
    assert!(
        ran.stdout
            .contains("specs/beta  [bundle]  beta v1.0.0 Active"),
        "{}",
        ran.stdout
    );
    // and a verb over every SDD reports on the others
    let ran = s.run(&["lint", "--all"]);
    assert_eq!(ran.code, 4);
    assert!(
        ran.stdout.contains("alpha: lint clean (L1–L12)"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("gamma: lint clean (L1–L12)"),
        "{}",
        ran.stdout
    );
    assert!(ran.stderr.contains("beta: "), "{}", ran.stderr);
}

/// `vitrify init` with no harness CLI on PATH: the home is set up — every
/// setting at its default, the content materialized — and no model is
/// called.
fn set_up_home(home: &Path) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vitrify"))
        .arg("init")
        .env("VITRIFY_HOME", home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "vitrify init: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
