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
        sha256: "53df333d7fe2fa7c2e0632b626145d337708baa921272bfbc322aa4e84fb2089",
    },
    Asset {
        rel_path: "briefs/build-unit.md",
        body: include_str!("../assets/briefs/build-unit.md"),
        sha256: "acd14cecb4ed92f1a9ea0133d8daeed9fe12aefc2cfaa2a55df383f572406f33",
    },
    Asset {
        rel_path: "briefs/change.md",
        body: include_str!("../assets/briefs/change.md"),
        sha256: "da57a1e8b7b56c1d0c0a03a082bf27ea90f2a03ebfe5e02740e394afff9475c0",
    },
    Asset {
        rel_path: "briefs/escalation.md",
        body: include_str!("../assets/escalation.md"),
        sha256: "8edb7a1def707c71b6a3debcfdf46d13875cfa34e599fba5b9d1360247cd5e4a",
    },
    Asset {
        rel_path: "briefs/integrate.md",
        body: include_str!("../assets/briefs/integrate.md"),
        sha256: "bfe09125cc54ccf2accab5a9003755b3c85ac88132368f598924326f6adca6c0",
    },
    Asset {
        rel_path: "briefs/kickoff.md",
        body: include_str!("../assets/briefs/kickoff.md"),
        sha256: "8015d1a7f43fd8bda4fe77064382f9373fcd3f81b47dcdf918d36355e118f912",
    },
    Asset {
        rel_path: "briefs/onboard.md",
        body: include_str!("../assets/briefs/onboard.md"),
        sha256: "fbd520d08d103a576b62511a32550fea0f6dca57c2527f7564bb4661a6c90e80",
    },
    Asset {
        rel_path: "briefs/run.md",
        body: include_str!("../assets/briefs/run.md"),
        sha256: "bcf8020d89b3eaba175a89a0c35b74ff13af91fd1389f469e9e1b59e20878d8f",
    },
    Asset {
        rel_path: "schemas/sdd-schema.md",
        body: include_str!("../assets/schemas/sdd-schema.md"),
        sha256: "f891d6523b0944a4ea7dd0632b10b7dda3113054582d64334e0a381ccb078507",
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
        sha256: "3ed9c3457b602e1e856c9b42716db29291868cb47499fde132c70680a20e490b",
    },
    Asset {
        rel_path: "templates/sdd-template-bundle/units/U-001-foundation.md",
        body: include_str!("../assets/templates/sdd-template-bundle/units/U-001-foundation.md"),
        sha256: "297454abcc47693d1295f53c033bdb4c988eaf301df14ece8cef7a1f99d9bef1",
    },
    Asset {
        rel_path: "templates/sdd-template-single.md",
        body: include_str!("../assets/templates/sdd-template-single.md"),
        sha256: "59f6ec561c63f4b023839d2b4062008b7132359909071436e5115a5861f57ee1",
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
    sha256: "f7c12e668d91f5d4894b123cea6e89748ba2020c671a4ea2549d592630c6f44c",
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
