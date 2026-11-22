//! Kept progress in project runs, end to end with the stub worker: a null
//! build, an interrupt and what the next run keeps, `--recheck`, pristine
//! checkouts across lanes, and rotten units named with their culprits (or
//! concluded flaky) in the report.

mod project_rig;

use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};

use project_rig::{
    Rig, commit_seed, dispatched, empty_rig, git, put, rig_of, rig_of_uncommitted, sessions,
    stub_log, wait_for,
};

// ------------------------------------------------------------ counting

/// Every Verification row of every unit, and every SDD's §5 Test command,
/// appends a line to `counter` before it runs: `<slug> <unit>` for a row,
/// `<slug> test` for the Test command. The counter is outside every
/// checkout, so no gate's cleaning touches it.
fn instrument(root: &Path, counter: &Path) {
    let counter = counter.display();
    for sdd in std::fs::read_dir(root.join("specs")).unwrap().flatten() {
        let slug = sdd.file_name().to_string_lossy().to_string();
        let sdd_md = sdd.path().join("sdd.md");
        let text = std::fs::read_to_string(&sdd_md).unwrap();
        assert!(text.contains("- Test:      `true`"), "{slug}");
        std::fs::write(
            &sdd_md,
            text.replace(
                "- Test:      `true`",
                &format!("- Test:      `echo '{slug} test' >> {counter}`"),
            ),
        )
        .unwrap();
        for unit in std::fs::read_dir(sdd.path().join("units"))
            .unwrap()
            .flatten()
        {
            let text = std::fs::read_to_string(unit.path()).unwrap();
            let id = text
                .lines()
                .find_map(|l| l.strip_prefix("unit: "))
                .unwrap()
                .to_string();
            let rows = text.replace(
                "| R1 | `",
                &format!("| R1 | `echo '{slug} {id}' >> {counter}; "),
            );
            assert_ne!(rows, text);
            std::fs::write(unit.path(), rows).unwrap();
        }
    }
}

/// How many times each counted command has run: `slug U-###` or
/// `slug test` to its count.
fn counts(counter: &Path) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in std::fs::read_to_string(counter).unwrap_or_default().lines() {
        *out.entry(line.to_string()).or_insert(0) += 1;
    }
    out
}

fn delta(before: &BTreeMap<String, usize>, after: &BTreeMap<String, usize>) -> Vec<String> {
    after
        .iter()
        .filter(|(k, n)| before.get(*k).copied().unwrap_or(0) < **n)
        .map(|(k, _)| k.clone())
        .collect()
}

/// project-pipeline with every row and Test command counted.
fn counted_pipeline() -> (Rig, PathBuf) {
    let r = rig_of_uncommitted("project-pipeline");
    let counter = r.scratch().join("counter");
    instrument(&r.root, &counter);
    commit_seed(&r.root);
    (r, counter)
}

const PIPELINE: [&str; 6] = [
    "api U-001",
    "api U-002",
    "api U-003",
    "core U-001",
    "core U-002",
    "core U-003",
];

/// What each project-pipeline unit depends on, transitively.
fn pipeline_upstream(unit: &str) -> &'static [&'static str] {
    match unit {
        "core U-001" => &[],
        "core U-002" => &["core U-001"],
        "core U-003" => &["core U-001", "core U-002"],
        "api U-001" => &["core U-001"],
        "api U-002" => &["api U-001", "core U-001", "core U-002"],
        "api U-003" => &["api U-001", "api U-002", "core U-001", "core U-002"],
        _ => unreachable!("{unit}"),
    }
}

fn unit_file(r: &Rig, unit: &str) -> String {
    let (slug, id) = unit.split_once(' ').unwrap();
    let dir = r.root.join("specs").join(slug).join("units");
    let name = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|n| n.starts_with(id))
        .unwrap();
    format!("specs/{slug}/units/{name}")
}

fn verified_on_main(r: &Rig, unit: &str) -> bool {
    r.on_main(&unit_file(r, unit))
        .contains("\nstatus: Verified\n")
}

// ------------------------------------------------------------ null build

#[test]
fn a_second_run_after_a_complete_project_run_runs_no_row_and_no_worker() {
    let (r, counter) = counted_pipeline();
    let log = r.scratch().join("stub.log");
    let log_s = log.display().to_string();
    let env = [("STUB_LOG", log_s.as_str())];
    let stdout = r.run_expect(0, &["run", "--cap", "2"], &env);
    assert!(stdout.contains("Complete."), "{stdout}");
    assert_eq!(sessions(&stub_log(&log)).len(), 6);
    let before = counts(&counter);
    for unit in PIPELINE {
        assert!(before.contains_key(unit), "{unit} ran no row: {before:?}");
    }
    let head = git(&r.root, &["rev-parse", "main"]);

    let stdout = r.run_expect(0, &["run", "--cap", "2"], &env);
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    assert!(
        stdout.contains("Plan: regenerate 0 units · adopt 0 built attempts · re-check 0 units"),
        "{stdout}"
    );
    // zero worker sessions, zero rows
    assert_eq!(sessions(&stub_log(&log)).len(), 6, "{stdout}");
    assert!(dispatched(&stdout).is_empty(), "{stdout}");
    let after = counts(&counter);
    let rows: Vec<String> = delta(&before, &after)
        .into_iter()
        .filter(|k| !k.ends_with(" test"))
        .collect();
    assert!(rows.is_empty(), "rows ran again: {rows:?}\n{stdout}");
    // and nothing to publish but what the run recorded about itself
    let changed = git(
        &r.root,
        &["diff", "--name-only", &head, "main", "--", "core", "api"],
    );
    assert!(changed.is_empty(), "{changed}");
}

// ------------------------------------------------- kept across an interrupt

#[test]
fn an_interrupted_project_run_keeps_built_attempts_and_green_rows() {
    let (r, counter) = counted_pipeline();
    // core U-002's gate waits, once, until the test lets it go: the
    // interrupt lands while a Built attempt is in the gate
    let (release, started) = (
        r.scratch().join("release"),
        r.scratch().join("gate-started"),
    );
    r.edit(
        "specs/core/units/U-002-parser.md",
        "| R1 | `echo 'core U-002' >> ",
        &format!(
            "| R1 | `if [ ! -e {} ]; then touch {}; sleep 60; fi; echo 'core U-002' >> ",
            release.display(),
            started.display()
        ),
    );
    r.commit("core U-002's gate waits once");
    let log = r.scratch().join("stub.log");
    let log_s = log.display().to_string();

    let bg = r.start(
        &["run", "--cap", "2"],
        &[("STUB_SLEEP_MS", "1000"), ("STUB_LOG", &log_s)],
    );
    wait_for("core U-002 in its gate", || started.exists());
    bg.signal("-INT");
    let (status, stdout, stderr) = bg.finish();
    assert_eq!(
        status.signal(),
        Some(2),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("[core] integrated U-001"), "{stdout}");
    assert!(!verified_on_main(&r, "core U-002"), "half-gated, published");

    // what the first run left: attempts a worker finished, and units green
    // on main with every unit they depend on green too
    let first = stub_log(&log);
    let built: Vec<String> = sessions(&first)
        .into_iter()
        .filter(|s| s.4.is_some())
        .map(|s| format!("{} {}", s.0, s.1))
        .filter(|u| !verified_on_main(&r, u))
        .collect();
    assert!(built.contains(&"core U-002".to_string()), "{built:?}");
    let settled: Vec<&str> = PIPELINE
        .iter()
        .copied()
        .filter(|u| {
            verified_on_main(&r, u) && pipeline_upstream(u).iter().all(|d| verified_on_main(&r, d))
        })
        .collect();
    assert!(settled.contains(&"core U-001"), "{settled:?}");

    std::fs::write(&release, "").unwrap();
    let before = counts(&counter);
    let stdout = r.run_expect(0, &["run", "--cap", "2"], &[("STUB_LOG", &log_s)]);
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    let second: Vec<String> = sessions(&stub_log(&log)[first.len()..])
        .into_iter()
        .map(|s| format!("{} {}", s.0, s.1))
        .collect();
    for unit in &built {
        assert!(
            !second.contains(unit),
            "a worker built {unit} again: {second:?}\n{stdout}"
        );
        let (slug, id) = unit.split_once(' ').unwrap();
        assert!(
            stdout.contains(&format!("[{slug}] {id} adopted (built attempt c1) → gate")),
            "{unit}: {stdout}"
        );
    }
    let ran = delta(&before, &counts(&counter));
    for unit in &settled {
        assert!(
            !ran.contains(&unit.to_string()),
            "{unit}'s rows ran again: {ran:?}\n{stdout}"
        );
    }
    for unit in PIPELINE {
        assert!(verified_on_main(&r, unit), "{unit}");
    }
}

// ------------------------------------------------------------ --recheck

// A product bug: in a project run, `--recheck` plans "re-check 6 units"
// and then runs no row and no Test command — every lane ends "Up to date".
// Project preflight leaves re-checks to the scheduler loop
// (`run/mod.rs`, "re-checks happen in the loop"), and the loop computes its
// dirty set with `Mode::Changes` whatever `--recheck` says
// (`run/sched.rs`, the head refresh); only the plan uses
// `Mode::Everything`. A single-SDD run (`--sdd`) re-checks everything, as
// `run_e2e.rs` shows.
#[test]
fn recheck_in_a_project_run_reruns_every_row_and_test_and_regenerates_nothing() {
    let (r, counter) = counted_pipeline();
    let log = r.scratch().join("stub.log");
    let log_s = log.display().to_string();
    let env = [("STUB_LOG", log_s.as_str())];
    r.run_expect(0, &["run", "--cap", "2"], &env);
    let head = git(&r.root, &["rev-parse", "main"]);
    let before = counts(&counter);

    let stdout = r.run_expect(0, &["run", "--recheck", "--cap", "2"], &env);
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    assert!(
        stdout.contains("Plan: regenerate 0 units · adopt 0 built attempts · re-check 6 units"),
        "{stdout}"
    );
    let ran = delta(&before, &counts(&counter));
    for unit in PIPELINE {
        assert!(ran.contains(&unit.to_string()), "{unit}: {ran:?}\n{stdout}");
    }
    for slug in ["api", "core"] {
        assert!(
            ran.contains(&format!("{slug} test")),
            "{slug}'s §5 Test: {ran:?}\n{stdout}"
        );
    }
    // regenerates nothing: no worker, no file of any unit changed
    assert_eq!(sessions(&stub_log(&log)).len(), 6, "{stdout}");
    assert!(dispatched(&stdout).is_empty(), "{stdout}");
    let changed = git(
        &r.root,
        &["diff", "--name-only", &head, "main", "--", "core", "api"],
    );
    assert!(changed.is_empty(), "{changed}");
    // the re-checks are recorded
    for slug in ["api", "core"] {
        let journal = r.on_main(&format!("specs/{slug}/journal.ndjson"));
        assert_eq!(
            journal
                .lines()
                .filter(
                    |l| l.contains(r#""event":"verified""#) && l.contains(r#""actor":"recheck""#)
                )
                .count(),
            3,
            "{journal}"
        );
    }
}

// ------------------------------------------------------------ pristine

/// project-pristine, plus: `dirty` U-002 after U-001 in the same lane,
/// asserting its gate never sees the log U-001's gate wrote; and `clean`
/// U-002, waiting on dirty U-001 through a pin, gated in its own lane once
/// that log has been written, asserting the same.
fn pristine_rig() -> Rig {
    let r = rig_of_uncommitted("project-pristine");
    put(
        &r.root,
        "specs/dirty/contracts/C-001-one.md",
        "# C-001 — dirty one\n\n- Producer: U-001\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\n```text\n# dirty/one.txt\ndirty one\n```\n",
    );
    put(
        &r.root,
        "specs/clean/contracts/C-001-dirty-one.md",
        "# C-001 — dirty one (pinned)\n\n- Producer: external — dirty, C-001\n- Consumers: U-002\n- Stability: Frozen by its producer\n- Pin: dirty C-001 @ 000000000000\n",
    );
    let after = |slug: &str, owns: &str, deps: &str, consumes: &str| {
        format!(
            "---\nunit: U-002\ntitle: U-002 ships {owns}\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [{deps}]\nparallel_group: —\nowns: [{owns}]\nconsumes: [{consumes}]\nbuilt: —\nverified: —\ntrace: —\n---\n\n# U-002 — ships `{owns}`\n\n## Requirements\n\n- **U-002.R1** — THE SYSTEM SHALL ship `{owns}` after {slug} U-001.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f {owns} && test -f dirty/one.txt` | exit 0 |\n| R1 | `test ! -e dirty/gate.log` | exit 0 |\n"
        )
    };
    put(
        &r.root,
        "specs/dirty/units/U-002-two.md",
        &after("dirty", "dirty/two.txt", "U-001", ""),
    );
    put(
        &r.root,
        "specs/clean/units/U-002-two.md",
        &after("dirty", "clean/two.txt", "C-001", "C-001"),
    );
    put(&r.support, "solution/dirty/two.txt", "dirty two\n");
    put(&r.support, "solution/clean/two.txt", "clean two\n");
    for slug in ["dirty", "clean"] {
        let map = r.support.join(format!("stub-map-{slug}.toml"));
        let text = std::fs::read_to_string(&map).unwrap();
        std::fs::write(&map, format!("{text}U-002 = [\"{slug}/two.txt\"]\n")).unwrap();
    }
    git(&r.root, &["init", "-q", "-b", "main"]);
    let (code, out, err) = r.run(&["pin", "sync", "--sdd", "specs/clean"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    commit_seed(&r.root);
    r
}

#[test]
fn an_ignored_file_one_gate_writes_is_absent_at_the_next_gate_in_any_lane() {
    let r = pristine_rig();
    let stdout = r.run_expect(0, &["run", "--cap", "2"], &[]);
    // dirty U-001's gate wrote the log before either U-002 was gated
    let landed = stdout
        .lines()
        .position(|l| l.starts_with("[dirty] integrated U-001"))
        .unwrap_or_else(|| panic!("{stdout}"));
    for lane in ["dirty", "clean"] {
        let at = stdout
            .lines()
            .position(|l| l.starts_with(&format!("[{lane}] integrated U-002 (c1)")))
            .unwrap_or_else(|| panic!("{lane} U-002 at its first gate:\n{stdout}"));
        assert!(landed < at, "{stdout}");
    }
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    // every row again, in each lane's checkout, after every gate wrote it
    let stdout = r.run_expect(0, &["run", "--recheck"], &[]);
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    assert!(!r.root.join("dirty/gate.log").exists());
}

// ------------------------------------------------------------ culprits

/// Two SDDs. `core` (scope `core/`): U-001 → U-002 → U-003, a chain owning
/// `core/a.txt`, `b.txt`, `c.txt`, U-003 producing C-001; and U-004,
/// independent, owning `core/d.txt`. `api` (scope `api/`): U-001 owning
/// `api/out.txt`, depending on core C-001 through a pin, so its re-checks
/// are dirtied by every unit of the chain and never by U-004. Its second
/// row fails once `core/<target>.txt` reads `v2`.
fn culprit_rig(target: &str) -> Rig {
    let r = empty_rig();
    let sdd = |slug: &str| {
        format!(
            "---\nsdd: {slug}\ntitle: {slug}\nschema_version: 2.1\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - {slug}/\n---\n\n# {slug}\n\n## Ground Rules\n\nOne implementer per unit; write only owned paths.\n\n## Technology Decisions\n\nPlain files; POSIX sh.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\nunmanaged: []\n"
        )
    };
    let unit = |id: &str, owns: &str, deps: &str, consumes: &str, rows: &[String]| {
        let rows: String = rows
            .iter()
            .map(|r| format!("| R1 | `{r}` | exit 0 |\n"))
            .collect();
        format!(
            "---\nunit: {id}\ntitle: {id} ships {owns}\nstatus: Ready\nclaimed_by: —\nclaim_expires: —\nclaim_seq: 0\ndepends_on: [{deps}]\nparallel_group: —\nowns: [{owns}]\nconsumes: [{consumes}]\nbuilt: —\nverified: —\ntrace: —\n---\n\n# {id} — ships `{owns}`\n\n## Requirements\n\n- **{id}.R1** — THE SYSTEM SHALL ship `{owns}` with the frozen content.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n{rows}"
        )
    };
    put(&r.root, "specs/core/sdd.md", &sdd("core"));
    put(&r.root, "specs/api/sdd.md", &sdd("api"));
    let mut map = String::from("[units]\n");
    for (id, file, deps) in [
        ("U-001", "a", ""),
        ("U-002", "b", "U-001"),
        ("U-003", "c", "U-002"),
        ("U-004", "d", ""),
    ] {
        let owns = format!("core/{file}.txt");
        put(
            &r.root,
            &format!("specs/core/units/{id}-{file}.md"),
            &unit(id, &owns, deps, "", &[format!("test -f {owns}")]),
        );
        put(&r.support, &format!("solution/{owns}"), "v1\n");
        put(
            &r.support,
            &format!("solution-2/{owns}"),
            &format!("{file} again\n"),
        );
        map.push_str(&format!("{id} = [\"{owns}\"]\n"));
    }
    put(&r.support, &format!("solution-2/core/{target}.txt"), "v2\n");
    put(&r.support, "stub-map-core.toml", &map);
    put(
        &r.root,
        "specs/core/contracts/C-001-c.md",
        "# C-001 — c surface\n\n- Producer: U-003\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\n```text\n# core/c.txt\nv1\n```\n",
    );
    put(
        &r.root,
        "specs/api/contracts/C-001-c.md",
        "# C-001 — c surface (pinned)\n\n- Producer: external — core, C-001\n- Consumers: U-001\n- Stability: Frozen by its producer\n- Pin: core C-001 @ 000000000000\n",
    );
    put(
        &r.root,
        "specs/api/units/U-001-out.md",
        &unit(
            "U-001",
            "api/out.txt",
            "C-001",
            "C-001",
            &[
                "test -f api/out.txt".to_string(),
                format!("! grep -qsx v2 core/{target}.txt"),
            ],
        ),
    );
    for tree in ["solution", "solution-2"] {
        put(&r.support, &format!("{tree}/api/out.txt"), "api out\n");
    }
    put(
        &r.support,
        "stub-map-api.toml",
        "[units]\nU-001 = [\"api/out.txt\"]\n",
    );
    git(&r.root, &["init", "-q", "-b", "main"]);
    let (code, out, err) = r.run(&["pin", "sync", "--sdd", "specs/api"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    commit_seed(&r.root);
    r
}

/// Build the culprit project, then change the text of every core unit and
/// mark them Stale, so the next run regenerates all four from `solution-2`:
/// three merges on api U-001's dependency chain, and U-004's beside it.
/// Returns that run's exit code and output.
fn rebuild_core(r: &Rig) -> (i32, String, String) {
    r.run_expect(0, &["run", "--cap", "2"], &[]);
    r.edit("specs/core/sdd.md", "version: 1.0.0", "version: 1.1.0");
    for (id, file) in [
        ("U-001", "a"),
        ("U-002", "b"),
        ("U-003", "c"),
        ("U-004", "d"),
    ] {
        r.edit(
            &format!("specs/core/units/{id}-{file}.md"),
            "with the frozen content.",
            "with the frozen content, again.",
        );
    }
    r.commit("core CH-001: every unit again");
    let (code, out, err) = r.run(&["audit", "--apply-stale", "--sdd", "specs/core"], &[]);
    let stale = std::fs::read_to_string(r.root.join("specs/core/units/U-004-d.md")).unwrap();
    assert!(stale.contains("status: Stale"), "{code}\n{out}{err}");
    r.commit("core CH-001: staleness applied");
    // the chain takes a while; U-004 lands at once
    let durations = r.scratch().join("durations.txt");
    std::fs::write(
        &durations,
        "core U-001 600\ncore U-002 600\ncore U-003 600\ncore U-004 0\n",
    )
    .unwrap();
    let (reference, durations) = (
        r.support.join("solution-2").display().to_string(),
        durations.display().to_string(),
    );
    r.run(
        &["run", "--cap", "2"],
        &[("STUB_REF", &reference), ("STUB_SLEEP_MAP", &durations)],
    )
}

/// The first-parent commit on main that changed `path` last, abbreviated.
fn changed_last(r: &Rig, path: &str) -> String {
    git(
        &r.root,
        &[
            "log",
            "--first-parent",
            "-1",
            "--format=%h",
            "main",
            "--",
            path,
        ],
    )
}

fn assert_culprit_named(r: &Rig, stdout: &str, culprit: &str, upstream: &str) {
    assert!(
        stdout.contains("[api] U-001 rotten: row 2 fails on mainline at "),
        "{stdout}"
    );
    let found =
        format!("[api] U-001 row 2 broke at {culprit} ({upstream} integrated) — culprit found");
    assert!(stdout.contains(&found), "want `{found}`:\n{stdout}");
    assert!(stdout.contains("\nBlocked.\n"), "{stdout}");
    let blocker = format!("  api      Blocked      U-001 rotten: row 2 broke at {culprit}");
    assert!(stdout.contains(&blocker), "want `{blocker}`:\n{stdout}");
    assert!(stdout.contains("  core     Complete"), "{stdout}");
    let report = std::fs::read_to_string(r.job_logs().join("report.txt")).unwrap();
    assert!(report.contains(&blocker), "{report}");
    let culprits: Vec<serde_json::Value> = r
        .events()
        .into_iter()
        .filter(|e| e["event"] == "culprit")
        .collect();
    assert_eq!(culprits.len(), 1, "{culprits:?}");
    assert_eq!(culprits[0]["flaky"], false);
}

#[test]
fn a_downstream_row_broken_among_three_upstream_merges_names_its_culprit() {
    let r = culprit_rig("b");
    let (code, stdout, stderr) = rebuild_core(&r);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // the chain's three merges, all in api U-001's range
    for id in ["U-001", "U-002", "U-003", "U-004"] {
        assert!(
            stdout.contains(&format!("[core] integrated {id} (c2)")),
            "{stdout}"
        );
    }
    let culprit = changed_last(&r, "core/b.txt");
    for other in ["core/a.txt", "core/c.txt"] {
        assert_ne!(changed_last(&r, other), culprit, "one merge each");
    }
    assert_culprit_named(&r, &stdout, &culprit, "core U-002");
    assert!(dispatched(&stdout).iter().all(|u| u.starts_with("core ")));
}

#[test]
fn a_culprit_outside_the_declared_dependencies_is_still_named() {
    // core U-004 is no dependency of api U-001's, but its row reads U-004's
    // file: the re-check the chain's merges cause finds it broken by U-004
    let r = culprit_rig("d");
    let (code, stdout, stderr) = rebuild_core(&r);
    assert_eq!(code, 2, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let culprit = changed_last(&r, "core/d.txt");
    for other in ["core/a.txt", "core/b.txt", "core/c.txt"] {
        assert_ne!(changed_last(&r, other), culprit, "one merge each");
    }
    assert_culprit_named(&r, &stdout, &culprit, "core U-004");
}

#[test]
fn a_flaky_row_is_concluded_a_flake_and_blamed_on_no_commit() {
    let r = rig_of("project-flaky");
    let counts = r.scratch().join("flaky");
    std::fs::create_dir_all(&counts).unwrap();
    let counts_s = counts.display().to_string();
    let stdout = r.run_expect(
        0,
        &["run", "--cap", "2"],
        &[("FLAKY_DIR", &counts_s), ("FLAKY_FAILS", "0")],
    );
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    let ran: usize = std::fs::read_to_string(counts.join("count"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // a change outside every SDD's scope dirties every Verified unit; the
    // row then fails on its next runs — the re-check, and its run alone —
    // and passes from then on, whatever the commit
    put(&r.root, "NOTES.txt", "read by everything\n");
    r.commit("notes");
    let fails = (ran + 2).to_string();
    // (two gate jobs: the row's failure is confirmed alone, on any machine)
    let (code, stdout, stderr) = r.run(
        &["run", "--cap", "2", "--gate-jobs", "2"],
        &[("FLAKY_DIR", &counts_s), ("FLAKY_FAILS", &fails)],
    );
    assert!(
        stdout.contains("[flaky] U-001 rotten: row 2 fails on mainline at "),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(
            "[flaky] U-001 row 2 is flaky: no commit since its last check broke it — fix the row (`sh flaky/check.sh`); the unit is re-checked again"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains(" broke at "), "{stdout}");
    let culprits: Vec<serde_json::Value> = r
        .events()
        .into_iter()
        .filter(|e| e["event"] == "culprit")
        .collect();
    assert_eq!(culprits.len(), 1, "{culprits:?}");
    assert_eq!(culprits[0]["flaky"], true);
    // not held against the unit: it is re-checked and the run completes
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("\nComplete.\n"), "{stdout}");
    assert!(!stdout.contains("rotten: flaky U-001"), "{stdout}");
}
