//! Embedded-asset registry: a hand-written
//! `include_str!` table — no embed crate, no `build.rs`. Digests are the
//! SHA-256 of the shipped bytes, written as literals; the completeness test
//! (`tests/content.rs`) walks `assets/` and recomputes every digest.
//!
//! `rel_path` is the asset-relative path as materialized under `~/.vitrify/`
//! : phase briefs and the escalation template under `briefs/`,
//! instance templates under `templates/`, and the normative schema
//! documents under `schemas/`; all sources live under this crate's
//! `assets/`, inside the package, so the published crate builds on its own.
//!
//! [`CONFIG`] is apart from the registry: `vitrify init` writes it once as
//! `~/.vitrify/config.toml`, and from then on the file is the user's —
//! nothing refreshes or restores it.

/// One embedded asset: path (relative to `~/.vitrify/`), body, shipped digest.
#[derive(Debug, Clone, Copy)]
pub struct Asset {
    pub rel_path: &'static str,
    pub body: &'static str,
    pub sha256: &'static str,
}

/// Every embedded asset, sorted by `rel_path`.
pub const ASSETS: &[Asset] = &[
    Asset {
        rel_path: "briefs/authoring.md",
        body: include_str!("../assets/briefs/authoring.md"),
        sha256: "7f3ae01d5e93c2c0e69bae364c894b64e0e9198f80ce8fd2130f4e22fb979fe7",
    },
    Asset {
        rel_path: "briefs/build-unit.md",
        body: include_str!("../assets/briefs/build-unit.md"),
        sha256: "855ed7000e615cf9ad2dd878dd8faabe9e65c966b4a0ab59947df0485a055e9f",
    },
    Asset {
        rel_path: "briefs/change.md",
        body: include_str!("../assets/briefs/change.md"),
        sha256: "07c612e94037ef2409345b2688fc5576a67f768e1f05160d2a926c021e6d5722",
    },
    Asset {
        rel_path: "briefs/escalation.md",
        body: include_str!("../assets/escalation.md"),
        sha256: "8edb7a1def707c71b6a3debcfdf46d13875cfa34e599fba5b9d1360247cd5e4a",
    },
    Asset {
        rel_path: "briefs/integrate.md",
        body: include_str!("../assets/briefs/integrate.md"),
        sha256: "cc8b6893ad255ebfabe3714e27f72951b480a4be439dd1600ab585dd3059c07d",
    },
    Asset {
        rel_path: "briefs/kickoff.md",
        body: include_str!("../assets/briefs/kickoff.md"),
        sha256: "8015d1a7f43fd8bda4fe77064382f9373fcd3f81b47dcdf918d36355e118f912",
    },
    Asset {
        rel_path: "briefs/onboard.md",
        body: include_str!("../assets/briefs/onboard.md"),
        sha256: "222eee3156df548f92738b534b37807faba44e3cfc59a4193f82afea5ebceb81",
    },
    Asset {
        rel_path: "briefs/run.md",
        body: include_str!("../assets/briefs/run.md"),
        sha256: "c7e7feb531bb6f42d77a590ffcfcfe8f178d7d72b26d6912c84acb82b9ee8fde",
    },
    Asset {
        rel_path: "schemas/sdd-schema.md",
        body: include_str!("../assets/schemas/sdd-schema.md"),
        sha256: "1e610b7202f9c3156a385f01a816b98d0b2a0939bb68e65e70ea118228d2f6d9",
    },
    Asset {
        rel_path: "schemas/tdd-schema.md",
        body: include_str!("../assets/schemas/tdd-schema.md"),
        sha256: "851b3f8f6e17213b4222bc8a54f8510ce9d4018277606ac1b42cd93593b11e2e",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/changelog.md",
        body: include_str!("../assets/templates/sdd-template-bundle/changelog.md"),
        sha256: "9b48a3305bb0167b699cfd80732ec9e1e68f096cd33812dd662f89688a11e17d",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/contracts/C-001-example.md",
        body: include_str!("../assets/templates/sdd-template-bundle/contracts/C-001-example.md"),
        sha256: "fd5b28450c6eccb02349b458d87355fe36f8a1036675c23c85941fd671f9398f",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/decisions.md",
        body: include_str!("../assets/templates/sdd-template-bundle/decisions.md"),
        sha256: "c9e9e2899359ac5f1fea14cc11e823063264640aed8da41532c1fff52e0001aa",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/sdd.md",
        body: include_str!("../assets/templates/sdd-template-bundle/sdd.md"),
        sha256: "e415430447eee3ecc00b5b94e6185b7664572e65748c38c78dae5f1ecfee77e9",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/units/U-001-foundation.md",
        body: include_str!("../assets/templates/sdd-template-bundle/units/U-001-foundation.md"),
        sha256: "297454abcc47693d1295f53c033bdb4c988eaf301df14ece8cef7a1f99d9bef1",
    },
    Asset {
        rel_path: "templates/sdd-template-single.md",
        body: include_str!("../assets/templates/sdd-template-single.md"),
        sha256: "4fe9b0ad71a746237931213f8d49b6c4c805ae5fb173f90e41381d466b7457ae",
    },
    Asset {
        rel_path: "templates/tdd-template.md",
        body: include_str!("../assets/templates/tdd-template.md"),
        sha256: "d7207fb97937399aac2f3228413644cfd0151ad72cf74c597ac4c89515fdba0a",
    },
];

/// The default configuration `vitrify init` writes to `~/.vitrify/config.toml`:
/// every setting at its default, and the harness role profiles.
pub const CONFIG: Asset = Asset {
    rel_path: "config.toml",
    body: include_str!("../assets/config.toml"),
    sha256: "83f2770f5dd1d461b65c16dcb9b426944826b90258ce7cfdcd05b572edbcdae1",
};

/// Enumerate the complete embedded set.
pub fn assets() -> &'static [Asset] {
    ASSETS
}

/// Look one asset up by its asset-relative path.
pub fn get(rel_path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.rel_path == rel_path)
}

/// SHA-256 hex digest of arbitrary content (for manifest comparison).
pub fn sha256_hex(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
