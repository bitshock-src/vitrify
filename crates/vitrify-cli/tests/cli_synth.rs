//! The synthetic project generator: a small repository lints clean and
//! `locate --graph --json` shows exactly the dependencies it generated.

use std::collections::BTreeSet;
use std::process::Command;

mod synth;

use synth::{Spec, generate};

fn vitrify(root: &std::path::Path, home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vitrify"))
        .args(args)
        .current_dir(root)
        .env("VITRIFY_HOME", home)
        .output()
        .unwrap()
}

#[test]
fn a_generated_project_lints_clean_and_has_the_generated_edges() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (root, support, home) = (base.join("repo"), base.join("support"), base.join("home"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), vitrify_core::content::CONFIG.body).unwrap();

    let spec = Spec::new(4, 6, 7);
    let project = generate(&spec);
    project.write(&root, &support);
    synth::commit_all(&root);
    let cross = project.cross_edges();
    assert!(!cross.is_empty(), "the seed makes edges across SDDs");
    assert!(
        project.edges().len() > cross.len(),
        "and edges inside an SDD"
    );

    // deterministic from the seed
    let again = generate(&spec);
    assert_eq!(project.edges(), again.edges());
    assert_eq!(
        project.units.iter().map(|u| u.millis).collect::<Vec<_>>(),
        again.units.iter().map(|u| u.millis).collect::<Vec<_>>()
    );
    assert_ne!(
        project.edges(),
        generate(&Spec::new(4, 6, 8)).edges(),
        "another seed, another graph"
    );

    let out = vitrify(&root, &home, &["lint", "--all", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let all = v.as_array().unwrap();
    assert_eq!(all.len(), spec.sdds);
    assert!(all.iter().all(|s| s["clean"] == true), "{text}");

    let out = vitrify(&root, &home, &["locate", "--graph", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let got: BTreeSet<_> = v["unit_edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let end = |k: &str| {
                (
                    e[k]["sdd"].as_str().unwrap().to_string(),
                    e[k]["unit"].as_str().unwrap().to_string(),
                )
            };
            (end("from"), end("to"))
        })
        .collect();
    assert_eq!(got, project.edges());
    assert_eq!(v["order"].as_array().unwrap().len(), project.units.len());
    assert!(v["cycles"].as_array().unwrap().is_empty());

    // the worker's inputs
    for slug in project.slugs() {
        assert!(support.join(format!("stub-map-{slug}.toml")).is_file());
    }
    let u = &project.units[0];
    assert!(support.join("solution").join(&u.file).is_file());
    let sleeps = std::fs::read_to_string(support.join("durations.txt")).unwrap();
    assert!(sleeps.contains(&format!("{} {} {}\n", u.slug, u.id, u.millis)));
    assert_eq!(
        sleeps.lines().filter(|l| !l.starts_with('#')).count(),
        project.units.len()
    );
}

#[test]
fn the_generator_scales_to_dozens_of_sdds_and_hundreds_of_units() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, support) = (tmp.path().join("repo"), tmp.path().join("support"));
    let project = generate(&Spec::new(30, 12, 1));
    project.write(&root, &support);
    assert_eq!(project.units.len(), 360);
    assert!(project.cross_edges().len() >= 30);
}
