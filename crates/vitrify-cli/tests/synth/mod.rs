//! A synthetic project: a repository of S SDDs with U units each, written
//! deterministically from a seed, for scheduling measurements and the
//! benchmark that runs `vitrify run` on a large one.
//!
//! The SDDs are schema 2.1 bundles, `s00`, `s01`, … (scope `s00/`, …). Each
//! unit `U-###` owns one file, `<slug>/u###.txt`, and its Verification row
//! is `test -f` on it. Inside an SDD a unit depends on the unit before it
//! (a chain), or, with `fan_percent`, on one of the three before that (a
//! fan out of a recent unit). With `cross_percent` a unit also depends on a
//! unit of an earlier SDD — never a later one, so the unit graph is
//! acyclic — through a pin: the upstream SDD publishes a contract the unit
//! produces (`Producer: U-###`), and the downstream SDD holds a pin
//! contract (`Producer: external — <slug>, C-###`, `Pin: <slug> C-### @
//! <hash>`, the hash `vitrify_core::contract::pin_hash` computes over the
//! authority's body) that its unit consumes and names in `depends_on`.
//!
//! Beside the repository, `write` puts the worker's inputs in a support
//! directory, outside the repository so nothing is left uncommitted in it:
//! `solution/<slug>/u###.txt` (the canned files), `stub-map-<slug>.toml`
//! (so `STUB_REF=<support>/solution`,
//! `STUB_MAP=<support>/stub-map-{sdd}.toml` serve every lane of
//! `crates/stub-worker`), and `durations.txt`, the `STUB_SLEEP_MAP` file
//! (`<sdd> <unit> <ms>` per line).

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use vitrify_core::contract::pin_hash;

/// A tiny seeded PRNG (xorshift64), so every repository reproduces.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// What to generate.
#[derive(Debug, Clone)]
pub struct Spec {
    pub sdds: usize,
    /// Units in each SDD (at most 999).
    pub units: usize,
    pub seed: u64,
    /// The chance, per unit, of one dependency on a unit of an earlier SDD.
    pub cross_percent: u64,
    /// The chance, per unit after the first, of depending on one of the
    /// three units before the previous one instead of on the previous one.
    pub fan_percent: u64,
    /// The inclusive range of a unit's duration, in milliseconds.
    pub millis: (u64, u64),
}

impl Spec {
    pub fn new(sdds: usize, units: usize, seed: u64) -> Spec {
        Spec {
            sdds,
            units,
            seed,
            cross_percent: 25,
            fan_percent: 40,
            millis: (50, 400),
        }
    }
}

/// A unit by its SDD slug and id.
pub type UnitRef = (String, String);

/// One unit of the generated project.
#[derive(Debug, Clone)]
pub struct SynthUnit {
    pub slug: String,
    pub id: String,
    /// The file the unit owns, relative to the repository root.
    pub file: String,
    /// Dependencies in the same SDD, as unit ids.
    pub local: Vec<String>,
    /// Dependencies on units of earlier SDDs.
    pub cross: Vec<UnitRef>,
    pub millis: u64,
}

/// A generated project, in memory.
#[derive(Debug, Clone)]
pub struct Synth {
    pub spec: Spec,
    /// SDD by SDD, unit by unit.
    pub units: Vec<SynthUnit>,
}

pub fn slug_of(s: usize) -> String {
    format!("s{s:02}")
}

pub fn unit_id(k: usize) -> String {
    format!("U-{:03}", k + 1)
}

/// Generate the project the spec describes.
pub fn generate(spec: &Spec) -> Synth {
    assert!(spec.units >= 1 && spec.units <= 999 && spec.sdds >= 1);
    let mut rng = Rng::new(spec.seed);
    let mut units = Vec::new();
    for s in 0..spec.sdds {
        let slug = slug_of(s);
        for k in 0..spec.units {
            let mut local = Vec::new();
            if k > 0 {
                let back = if k > 1 && rng.chance(spec.fan_percent) {
                    2 + rng.below(3.min(k - 1))
                } else {
                    1
                };
                local.push(unit_id(k - back));
            }
            let mut cross = Vec::new();
            if s > 0 && rng.chance(spec.cross_percent) {
                let up = rng.below(s);
                cross.push((slug_of(up), unit_id(rng.below(spec.units))));
            }
            let (lo, hi) = spec.millis;
            units.push(SynthUnit {
                file: format!("{slug}/u{:03}.txt", k + 1),
                slug: slug.clone(),
                id: unit_id(k),
                local,
                cross,
                millis: lo + rng.next() % (hi - lo + 1),
            });
        }
    }
    Synth {
        spec: spec.clone(),
        units,
    }
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

impl Synth {
    pub fn slugs(&self) -> Vec<String> {
        (0..self.spec.sdds).map(slug_of).collect()
    }

    /// Every dependency as `((slug, unit), (slug, unit))`: the first unit
    /// depends on the second. Sorted.
    pub fn edges(&self) -> BTreeSet<(UnitRef, UnitRef)> {
        let mut out = BTreeSet::new();
        for u in &self.units {
            for l in &u.local {
                out.insert(((u.slug.clone(), u.id.clone()), (u.slug.clone(), l.clone())));
            }
            for c in &u.cross {
                out.insert(((u.slug.clone(), u.id.clone()), c.clone()));
            }
        }
        out
    }

    /// The edges that cross SDDs.
    pub fn cross_edges(&self) -> BTreeSet<(UnitRef, UnitRef)> {
        self.edges()
            .into_iter()
            .filter(|(a, b)| a.0 != b.0)
            .collect()
    }

    /// The contract id a unit's authority has in its SDD.
    fn authority(id: &str) -> String {
        format!("C-{}", &id[2..])
    }

    /// The body of the authority a unit produces, and the whole file.
    fn authority_text(u: &SynthUnit) -> String {
        let c = Self::authority(&u.id);
        format!(
            "# {c} — surface of {id}\n\n- Producer: {id}\n- Consumers: —\n- Stability: Frozen\n\n## Surface\n\n```text\n# {file}\n{slug} {id}\n```\n\n## Semantics\n\n- THE SYSTEM SHALL write exactly one line and nothing after it.\n",
            id = u.id,
            file = u.file,
            slug = u.slug
        )
    }

    /// Write the repository into `root` (the SDDs only) and the worker's
    /// inputs into `support`.
    pub fn write(&self, root: &Path, support: &Path) {
        let mut sleep = String::from("# <sdd> <unit> <ms>\n");
        for slug in self.slugs() {
            put(root, &format!("specs/{slug}/sdd.md"), &sdd_md(&slug));
            let mine: Vec<&SynthUnit> = self.units.iter().filter(|u| u.slug == slug).collect();
            // the authorities other SDDs pin
            let targeted: BTreeSet<&str> = self
                .units
                .iter()
                .flat_map(|u| u.cross.iter())
                .filter(|(s, _)| *s == slug)
                .map(|(_, id)| id.as_str())
                .collect();
            for u in &mine {
                if targeted.contains(u.id.as_str()) {
                    put(
                        root,
                        &format!(
                            "specs/{slug}/contracts/{}-surface.md",
                            Self::authority(&u.id)
                        ),
                        &Self::authority_text(u),
                    );
                }
            }
            // one pin contract per distinct upstream authority this SDD uses
            let mut pins: Vec<UnitRef> = Vec::new();
            for u in &mine {
                for c in &u.cross {
                    if !pins.contains(c) {
                        pins.push(c.clone());
                    }
                }
            }
            for (n, (up_slug, up_id)) in pins.iter().enumerate() {
                let upstream = self
                    .units
                    .iter()
                    .find(|x| x.slug == *up_slug && x.id == *up_id)
                    .unwrap();
                let text = Self::authority_text(upstream);
                let body = &text[text.find("## Surface").unwrap()..];
                let authority = Self::authority(up_id);
                let id = Self::pin_id(n);
                let consumers: Vec<&str> = mine
                    .iter()
                    .filter(|u| u.cross.contains(&(up_slug.clone(), up_id.clone())))
                    .map(|u| u.id.as_str())
                    .collect();
                put(
                    root,
                    &format!("specs/{slug}/contracts/{id}-{up_slug}.md"),
                    &format!(
                        "# {id} — {authority} of {up_slug} (pinned)\n\n- Producer: external — {up_slug}, {authority}\n- Consumers: {}\n- Stability: Frozen by its producer\n- Pin: {up_slug} {authority} @ {}\n",
                        consumers.join(", "),
                        pin_hash(body)
                    ),
                );
            }
            let mut map = String::from("[units]\n");
            for u in &mine {
                let mut deps: Vec<String> = u.local.clone();
                let mut consumes: Vec<String> = Vec::new();
                for c in &u.cross {
                    let n = pins.iter().position(|p| p == c).unwrap();
                    deps.push(Self::pin_id(n));
                    consumes.push(Self::pin_id(n));
                }
                put(
                    root,
                    &format!(
                        "specs/{slug}/units/{}-{}.md",
                        u.id,
                        u.file.rsplit('/').next().unwrap().trim_end_matches(".txt")
                    ),
                    &unit_md(u, &deps, &consumes),
                );
                put(
                    support,
                    &format!("solution/{}", u.file),
                    &format!("{} {}\n", u.slug, u.id),
                );
                map.push_str(&format!("{} = [\"{}\"]\n", u.id, u.file));
                sleep.push_str(&format!("{} {} {}\n", u.slug, u.id, u.millis));
            }
            put(support, &format!("stub-map-{slug}.toml"), &map);
        }
        put(support, "durations.txt", &sleep);
    }

    fn pin_id(n: usize) -> String {
        format!("C-1{:03}", n + 1)
    }
}

fn sdd_md(slug: &str) -> String {
    format!(
        "---\nsdd: {slug}\ntitle: synthetic {slug}\nschema_version: 2.1\nversion: 1.0.0\nstatus: Active\nscope_roots:\n  - {slug}/\n---\n\n# {slug}\n\n## Ground Rules\n\nOne implementer per unit; write only owned paths.\n\n## Technology Decisions\n\nPlain files.\n\n### Commands\n\n- Bootstrap: `true`\n- Build:     `true`\n- Test:      `true`\n- Lint:      `true`\n\n## System Layout & Ownership\n\nunmanaged: []\n"
    )
}

fn unit_md(u: &SynthUnit, deps: &[String], consumes: &[String]) -> String {
    format!(
        "---\nunit: {id}\ntitle: {id} ships {file}\nstatus: Ready\nclaimed_by: \u{2014}\nclaim_expires: \u{2014}\nclaim_seq: 0\ndepends_on: [{deps}]\nparallel_group: \u{2014}\nowns: [{file}]\nconsumes: [{consumes}]\nbuilt: \u{2014}\nverified: \u{2014}\ntrace: \u{2014}\n---\n\n# {id} \u{2014} ships `{file}`\n\n## Requirements\n\n- **{id}.R1** \u{2014} THE SYSTEM SHALL ship `{file}`.\n\n## Verification\n\n| Covers | Command | Must satisfy |\n| --- | --- | --- |\n| R1 | `test -f {file}` | exit 0 |\n",
        id = u.id,
        file = u.file,
        deps = deps.join(", "),
        consumes = consumes.join(", ")
    )
}

/// Make `root` a git repository on `main` with everything committed.
pub fn commit_all(root: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "t@test"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "synthetic project"],
    ] {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
