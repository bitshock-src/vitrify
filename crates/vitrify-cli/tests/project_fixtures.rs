//! The seeded `project-*` fixtures lint clean (each SDD, under `--sdd`), and
//! the stub worker's `{sdd}` map, `STUB_SLEEP_*` and `STUB_LOG` switches work
//! on a small run over `run-multi`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn vitrify_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vitrify")
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

fn git(dir: &Path, args: &[&str]) {
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
}

/// A seeded project's `project/` tree committed on main in a temp repository.
fn repo_of(fixture: &str, tmp: &Path) -> PathBuf {
    let root = tmp.join("repo");
    copy_dir(
        &repo_root()
            .join("fixtures/seeded")
            .join(fixture)
            .join("project"),
        &root,
    );
    let root = root.canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@test"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    root
}

fn write_config(home: &Path, worker: &Path) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "{}\n[probe]\nvitrify_version = \"{}\"\ngit = {{ present = true, version = \"2\" }}\n\n[probe.claude]\npresent = true\nheadless = [\"{}\"]\n",
            vitrify_core::content::CONFIG.body,
            env!("CARGO_PKG_VERSION"),
            worker.display()
        ),
    )
    .unwrap();
}

fn sdd_dirs(root: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = std::fs::read_dir(root.join("specs"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("sdd.md").exists())
        .map(|e| format!("specs/{}", e.file_name().to_string_lossy()))
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn the_seeded_projects_lint_clean_per_sdd() {
    // the two seeded on purpose are checked by the tests of what they seed
    let projects = [
        "project-pipeline",
        "project-interface-only",
        "project-mutual",
        "project-cascade",
        "project-flaky",
        "project-pristine",
    ];
    for fixture in projects {
        let tmp = tempfile::tempdir().unwrap();
        let root = repo_of(fixture, tmp.path());
        let home = tmp.path().join("vhome");
        write_config(&home, Path::new("stub-worker"));
        let sdds = sdd_dirs(&root);
        assert!(!sdds.is_empty(), "{fixture}: no SDDs");
        for sdd in sdds {
            let out = Command::new(vitrify_bin())
                .args(["lint", "--sdd", &sdd])
                .current_dir(&root)
                .env("VITRIFY_HOME", &home)
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.status.success(),
                "{fixture} {sdd} does not lint clean:\n{text}"
            );
        }
    }
}

#[test]
fn the_stub_switches_map_per_sdd_sleep_and_log() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo_of("run-multi", tmp.path());
    let vit = PathBuf::from(vitrify_bin());
    let stub = vit.parent().unwrap().join("stub-worker");
    assert!(
        Command::new(env!("CARGO"))
            .args(["build", "-p", "stub-worker", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    let home = tmp.path().join("vhome");
    write_config(&home, &stub);
    std::fs::create_dir_all(tmp.path().join("t")).unwrap();
    let fixture = repo_root().join("fixtures/seeded/run-multi");
    let log = tmp.path().join("stub.log");
    let sleeps = tmp.path().join("sleeps");
    std::fs::write(&sleeps, "# per unit\nU-001 20\nbeta U-002 150\n").unwrap();

    for sdd in ["alpha", "beta"] {
        let out = Command::new(vitrify_bin())
            .args(["run", "--sdd", &format!("specs/{sdd}"), "--cap", "1"])
            .current_dir(&root)
            .env("VITRIFY_HOME", &home)
            .env("VITRIFY_BIN", vitrify_bin())
            .env("TMPDIR", tmp.path().join("t"))
            .env("STUB_MODE", "build")
            .env("STUB_REF", fixture.join("solution"))
            .env("STUB_MAP", fixture.join("stub-map-{sdd}.toml"))
            .env("STUB_SLEEP_MS", "10")
            .env("STUB_SLEEP_MAP", &sleeps)
            .env("STUB_LOG", &log)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{sdd}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // one start and one end per unit, in order, carrying slug, unit and seq
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    let shape: Vec<String> = rows
        .iter()
        .map(|r| format!("{} {} {} {}", r[0], r[1], r[2], r[3]))
        .collect();
    assert_eq!(
        shape,
        [
            "start alpha U-001 1",
            "end alpha U-001 1",
            "start alpha U-002 1",
            "end alpha U-002 1",
            "start beta U-001 1",
            "end beta U-001 1",
            "start beta U-002 1",
            "end beta U-002 1",
        ]
    );
    let ms = |i: usize| rows[i][4].parse::<u128>().unwrap();
    assert!(ms(1) - ms(0) >= 20, "U-001 sleeps its mapped 20 ms");
    assert!(ms(3) - ms(2) >= 10, "alpha U-002 sleeps the default 10 ms");
    assert!(ms(7) - ms(6) >= 150, "beta U-002 sleeps its own 150 ms");
}
