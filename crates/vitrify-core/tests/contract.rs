//! External contracts: header lines, the body, the authority a `Producer:`
//! line names, pins and transcriptions examined against it — in both shapes
//! and across them — and what a sync writes.

use std::path::{Path, PathBuf};

use vitrify_core::contract::{self, External, Kind, Naming, Peers, Pin, State};
use vitrify_core::model::{self, Instance, Schema, SddHandle};
use vitrify_core::readset;
use vitrify_core::trace;

// ------------------------------------------------------------ scaffolding

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        Repo { _tmp: tmp, root }
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    fn peers(&self) -> Peers<'_> {
        Peers::new(|| model::discover(&self.root))
    }

    fn sdd(&self, rel: &str) -> SddHandle {
        model::resolve_sdd(&self.root, Some(rel), &self.root).unwrap()
    }

    fn inst(&self, rel: &str) -> Instance {
        Instance::load(self.sdd(rel)).unwrap()
    }

    fn examine(&self, rel: &str) -> Vec<External> {
        contract::examine(&self.inst(rel), &self.peers()).unwrap()
    }

    /// The one external contract `id` of the SDD at `rel`.
    fn one(&self, rel: &str, id: &str) -> External {
        self.examine(rel)
            .into_iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("{id} is not external in {rel}"))
    }

    /// Apply every fix the SDD at `rel` has.
    fn sync(&self, rel: &str) -> usize {
        let fixes: Vec<contract::Fix> = self
            .examine(rel)
            .into_iter()
            .filter_map(|e| e.fix)
            .collect();
        contract::apply(&fixes).unwrap();
        fixes.len()
    }
}

fn core(slug: &str, extra: &str) -> String {
    let schema = if extra.contains("schema_version") {
        ""
    } else {
        "schema_version: 2.0\n"
    };
    format!(
        "---\nsdd: {slug}\n{schema}{extra}version: 1.0.0\nstatus: Draft\nscope_roots:\n  - {slug}/\n---\n\n# {slug}\n\n## Ground Rules\n\nG.\n\n## Technology Decisions\n\nT.\n"
    )
}

fn unit_file(uid: &str, consumes: &str) -> String {
    format!(
        "---\nunit: {uid}\nstatus: Ready\nconsumes: [{consumes}]\ntrace: —\n---\n\n# {uid}\n\n## Requirements\n\n- **{uid}.R1** — THE SYSTEM SHALL exist.\n"
    )
}

/// A bundle `specs/<slug>/` with the given contract files.
fn bundle(repo: &Repo, slug: &str, extra: &str, contracts: &[(&str, &str)]) {
    repo.write(&format!("specs/{slug}/sdd.md"), &core(slug, extra));
    for (name, text) in contracts {
        repo.write(&format!("specs/{slug}/contracts/{name}"), text);
    }
}

/// A single-file SDD `specs/<slug>.sdd.md` whose Interface Contracts
/// section is `contracts`, followed by one unit consuming `consumes`.
fn single(repo: &Repo, slug: &str, extra: &str, contracts: &str, consumes: &str) {
    let text = format!(
        "{}\n## Interface Contracts\n\n{contracts}## Work Units\n\n### U-001 — the unit\n\n```yaml\nunit: U-001\nstatus: Ready\nconsumes: [{consumes}]\ntrace: —\n```\n\n#### Requirements\n\n- **U-001.R1** — THE SYSTEM SHALL exist.\n",
        core(slug, extra)
    );
    repo.write(&format!("specs/{slug}.sdd.md"), &text);
}

const BODY: &str = "## Surface\n\n```python\n# Module: pool.api\n## not a heading: it is code\ndef open(root): ...\n```\n\n## Semantics\n\n- THE SYSTEM SHALL open.\n\n### Errors\n\n- `root-missing` (4).\n";

fn authority(id: &str) -> String {
    format!(
        "# {id} — Pool API\n\n- Producer: U-001\n- Consumers: U-002\n- Stability: Frozen\n\n{BODY}"
    )
}

/// An external contract with no `Pin:` line, and `body` after its header.
fn unpinned(id: &str, producer: &str, body: &str) -> String {
    format!(
        "# {id} — Pool API\n\n- Producer: external — {producer}\n- Consumers: U-001\n- Stability: Frozen by its producer\n\n{body}"
    )
}

/// A body a single-file SDD can hold: its code has no line beginning `##`
/// or `###` — the single-file parsers end a section, or a region, at one.
const PLAIN: &str = "## Surface\n\n```python\n# Module: pool.api\ndef open(root): ...\n```\n\n## Semantics\n\n- THE SYSTEM SHALL open.\n\n### Errors\n\n- `root-missing` (4).\n";

/// `body` (bundle form, `## Surface` …) as a region of a single-file SDD
/// holds it: every heading outside fenced code two levels down.
fn deeper(body: &str) -> String {
    let mut fence: Option<String> = None;
    let mut out = String::new();
    for raw in body.split_inclusive('\n') {
        let marker: String = raw.chars().take_while(|c| *c == '`' || *c == '~').collect();
        match &fence {
            Some(open) => {
                if marker.len() >= open.len() && marker.starts_with(&open[..1]) {
                    fence = None;
                }
                out.push_str(raw);
            }
            None if marker.len() >= 3 => {
                fence = Some(marker);
                out.push_str(raw);
            }
            None if raw.starts_with('#') => {
                out.push_str("##");
                out.push_str(raw);
            }
            None => out.push_str(raw),
        }
    }
    out
}

// ----------------------------------------------------------- header lines

#[test]
fn header_fields_are_the_bullets_between_title_and_first_section() {
    let text = "# C-012 — Pool API (pinned)\n\n- Producer: external — pool, C-010\n- Consumers: U-005, U-006,\n  U-007\n- Stability: Frozen\n  - a nested note\n- Pin: pool C-010 @ 4f2a91c03d7e\n\n## Surface\n\n- Producer: U-999 (prose in the body, not the header)\n- Notes: body only\n";
    assert_eq!(
        contract::field(text, "Producer").as_deref(),
        Some("external — pool, C-010")
    );
    // an indented continuation line belongs to the value; a nested bullet
    // does not
    assert_eq!(
        contract::field(text, "Consumers").as_deref(),
        Some("U-005, U-006, U-007")
    );
    assert_eq!(
        contract::field(text, "Stability").as_deref(),
        Some("Frozen")
    );
    assert_eq!(
        contract::field(text, "Pin").as_deref(),
        Some("pool C-010 @ 4f2a91c03d7e")
    );
    // the header ends at the first section heading
    assert_eq!(contract::field(text, "Notes"), None);
    // a key is matched whole
    assert_eq!(contract::field(text, "Pinned"), None);
    assert_eq!(contract::field("- Pinned: no\n", "Pin"), None);
    // a single-file region reads the same way, and a bullet is optional
    let region = "### C-001 — x\n\nProducer: U-001\n* Consumers: U-002\n\n#### Surface\n";
    assert_eq!(
        contract::field(region, "Producer").as_deref(),
        Some("U-001")
    );
    assert_eq!(
        contract::field(region, "Consumers").as_deref(),
        Some("U-002")
    );
}

#[test]
fn unit_ids_stand_as_words() {
    assert_eq!(
        contract::unit_ids("U-001, U-002 and U-001 again; XU-003 and U-4x are not ids"),
        ["U-001", "U-002"]
    );
    assert!(contract::unit_ids("—").is_empty());
}

#[test]
fn external_is_the_word_at_the_start_of_the_producer_line() {
    for yes in [
        "external — pool, C-010",
        "external",
        "  external: the host clock",
        "external (steward)",
    ] {
        assert!(contract::is_external(yes), "{yes}");
    }
    for no in [
        "U-001",
        "externally visible unit U-001",
        "external-api U-002",
        "",
    ] {
        assert!(!contract::is_external(no), "{no}");
    }
}

// --------------------------------------------- what a Producer line names

#[test]
fn producer_lines_name_their_authority_by_slug_then_contract_id() {
    let slugs = ["acme-pool", "acme-m1", "acme-m2", "acme-writers", "pool"];
    let named = |line: &str| contract::naming(line, &slugs);
    let authority = |slug: &str, id: &str| Naming::Authority {
        slug: slug.to_string(),
        contract: id.to_string(),
    };
    // the forms Producer lines take in a repository of many bundles
    for (line, slug, id) in [
        (
            "external — acme-pool, C-010 `acme/specs/pool/contracts/C-010-x06-pool-api-admit.md` (X-06)",
            "acme-pool",
            "C-010",
        ),
        (
            "external — acme-pool C-010, acme/specs/pool/contracts/C-010-x06-pool-api-admit.md (X-06)",
            "acme-pool",
            "C-010",
        ),
        (
            "external — acme-pool transcription, C-018 `acme/specs/pool/contracts/C-018-e1.md` (E-1)",
            "acme-pool",
            "C-018",
        ),
        (
            "external — acme-m1 (owner), C-007 `acme/specs/m1/contracts/C-007-e2-facts.md` (E-2)",
            "acme-m1",
            "C-007",
        ),
        // the first word is a package with no SDD: the slug nearest the id
        (
            "external — schema-gen, owner copy acme-m2 C-004, acme/specs/m2/contracts/C-004-e2.md (E-2)",
            "acme-m2",
            "C-004",
        ),
        // words, and other names, between the slug and the id
        (
            "external — acme-pool (owner of E-2; tool packages judge, schema-gen), C-020 `acme/specs/pool/contracts/C-020-e2-bundle-core.md` (E-2)",
            "acme-pool",
            "C-020",
        ),
        // a slug is matched whole: `pool` is not `acme-pool`
        ("external — pool, C-003", "pool", "C-003"),
        // the id may come first in a path; the pair that has a slug wins
        (
            "external — `specs/x/contracts/C-010-api.md`, that is acme-writers C-010",
            "acme-writers",
            "C-010",
        ),
    ] {
        assert_eq!(named(line), authority(slug, id), "{line}");
    }

    // no contract id: a transcription of something that has no SDD
    for line in [
        "external — tool package report-gen, `report totals` (`tools/report/src/report/totals.py`, as of 2026-11-06); [report-tdd] §3.2.11, §3.1.4 (E-9)",
        // `EC-13` holds no contract id
        "external — format: [pool-tdd] §3.3.1 step 3 (D-14; [pool-tdd] revision 1.6, EC-13) (E-3)",
        "external",
    ] {
        assert_eq!(named(line), Naming::Nothing, "{line}");
    }

    // a contract id and no SDD of the repository before it: an authority
    // that cannot be found — never "unchecked"
    assert_eq!(
        named("external — billing, C-004 `specs/billing/contracts/C-004-api.md`"),
        Naming::NoSdd {
            contract: "C-004".to_string()
        }
    );
    assert_eq!(
        contract::naming("external — acme-pool, C-010", &[]),
        Naming::NoSdd {
            contract: "C-010".to_string()
        }
    );
}

#[test]
fn pin_lines_read_slug_id_and_hash() {
    let pin = |slug: &str, id: &str, hash: Option<&str>| {
        Some(Pin {
            slug: slug.to_string(),
            contract: id.to_string(),
            hash: hash.map(str::to_string),
        })
    };
    assert_eq!(
        contract::parse_pin("pool C-010 @ 4f2a91c03d7e"),
        pin("pool", "C-010", Some("4f2a91c03d7e"))
    );
    assert_eq!(
        contract::parse_pin("acme-pool, C-010 @ sha256:4F2A91C03D7E"),
        pin("acme-pool", "C-010", Some("4f2a91c03d7e"))
    );
    // no hash yet: a sync records it
    assert_eq!(
        contract::parse_pin("pool C-010"),
        pin("pool", "C-010", None)
    );
    assert_eq!(
        contract::parse_pin("pool C-010 @ —"),
        pin("pool", "C-010", None)
    );
    for bad in [
        "C-010 @ 4f2a91c03d7e",
        "pool @ 4f2a",
        "pool C-010 @ not-hex",
        "pool C-10x",
        "",
    ] {
        assert_eq!(contract::parse_pin(bad), None, "{bad}");
    }
}

// ------------------------------------------------------------------- body

#[test]
fn the_body_starts_at_the_first_surface_heading_outside_code() {
    let text = "# C-001 — x\n\n- Producer: U-001\n\n```\n## Surface\n```\n\n## Surface\n\ntext\n";
    let (at, level) = contract::surface(text).unwrap();
    assert_eq!(level, 2);
    assert_eq!(&text[at..], "## Surface\n\ntext\n");
    // a region's is two levels down
    assert_eq!(
        contract::surface("### C-001 — x\n\n#### Surface\n").map(|(_, l)| l),
        Some(4)
    );
    // the title must be exactly `Surface`
    assert_eq!(
        contract::surface("# C-001\n\n## Surfaces\n\n## Surface area\n"),
        None
    );
    assert_eq!(contract::surface("# C-001\n\nno sections\n"), None);
}

// ------------------------------------------------- pins and transcriptions

#[test]
fn an_external_contract_without_a_pin_is_a_transcription_or_a_pin_to_write() {
    let repo = Repo::new();
    bundle(&repo, "pool", "", &[("C-010-api.md", &authority("C-010"))]);
    bundle(
        &repo,
        "heldout",
        "",
        &[
            // a source no SDD owns: the body is the consumer's own text
            (
                "C-001-clock.md",
                &unpinned("C-001", "the host clock, POSIX `date`", BODY),
            ),
            // names another SDD's contract, and carries a body of its own
            ("C-002-pool.md", &unpinned("C-002", "pool, C-010", BODY)),
            // names it, and has no body yet
            ("C-003-pool.md", &unpinned("C-003", "pool, C-010", "")),
            // names a contract, and no SDD of the repository before it
            ("C-004-billing.md", &unpinned("C-004", "billing, C-004", "")),
        ],
    );

    let t = repo.one("specs/heldout", "C-001");
    assert_eq!((t.kind, t.state), (Kind::Transcription, State::Unchecked));
    assert!(t.authority.is_none() && t.fix.is_none());
    assert!(!State::Unchecked.is_finding() && !State::Ok.is_finding());

    let with_body = repo.one("specs/heldout", "C-002");
    assert_eq!(
        (with_body.kind, with_body.state),
        (Kind::Pin, State::Invalid)
    );
    assert!(with_body.fix.is_none(), "the author writes the Pin line");
    assert_eq!(
        with_body.detail,
        "its Producer line names pool C-010 and it has no `Pin:` line — write `- Pin: pool C-010` in place of its body, then `vitrify pin sync` records the hash"
    );
    let a = with_body.authority.unwrap();
    assert_eq!((a.slug.as_str(), a.contract.as_str()), ("pool", "C-010"));
    assert_eq!(
        a.file.unwrap(),
        repo.root.join("specs/pool/contracts/C-010-api.md")
    );
    assert_eq!(a.body_bytes, Some(BODY.len() as u64));
    let bare = repo.one("specs/heldout", "C-003");
    assert_eq!(bare.state, State::Invalid);
    assert!(
        bare.detail
            .contains("write `- Pin: pool C-010` in its header"),
        "{}",
        bare.detail
    );
    let nowhere = repo.one("specs/heldout", "C-004");
    assert_eq!(
        (nowhere.kind, nowhere.state),
        (Kind::Pin, State::MissingAuthority)
    );
    assert!(
        nowhere
            .detail
            .contains("names C-004, and no SDD of this repository"),
        "{}",
        nowhere.detail
    );
    // nothing here is a sync's to write
    assert_eq!(repo.sync("specs/heldout"), 0);

    // the Pin line written as the cause says: a sync records the hash
    repo.write(
        "specs/heldout/contracts/C-003-pool.md",
        &unpinned("C-003", "pool, C-010", "")
            .replace("producer\n\n", "producer\n- Pin: pool C-010\n"),
    );
    assert_eq!(repo.one("specs/heldout", "C-003").state, State::Stale);
    assert_eq!(repo.sync("specs/heldout"), 1);
    let pin = repo.one("specs/heldout", "C-003");
    assert_eq!((pin.kind, pin.state), (Kind::Pin, State::Ok));
    assert!(
        repo.read("specs/heldout/contracts/C-003-pool.md")
            .ends_with(&format!(
                "- Pin: pool C-010 @ {}\n",
                contract::pin_hash(BODY)
            ))
    );

    // the producer's own contract is not external
    assert!(repo.examine("specs/pool").is_empty());
}

#[test]
fn consumers_lines_are_held_to_the_units_consumes() {
    let repo = Repo::new();
    bundle(
        &repo,
        "x",
        "",
        &[
            // the producer consumes its own contract: not a consumer
            (
                "C-001-a.md",
                "# C-001\n\n- Producer: U-001\n- Consumers: U-002\n",
            ),
            // names a unit that does not consume it, omits one that does
            (
                "C-002-b.md",
                "# C-002\n\n- Producer: U-001\n- Consumers: U-002, U-009\n",
            ),
            // nobody consumes it, and it says so
            (
                "C-003-c.md",
                "# C-003\n\n- Producer: U-002\n- Consumers: —\n",
            ),
            // an external producer's line names no unit of this SDD
            (
                "C-004-d.md",
                "# C-004\n\n- Producer: external — pool U-001's API, C-010\n- Consumers: U-001\n",
            ),
            // no Consumers line at all, and a consumer
            ("C-005-e.md", "# C-005\n\n- Producer: U-001\n"),
        ],
    );
    repo.write(
        "specs/x/units/U-001-a.md",
        &unit_file("U-001", "C-001, C-004"),
    );
    repo.write(
        "specs/x/units/U-002-b.md",
        &unit_file("U-002", "C-001, C-005"),
    );
    repo.write("specs/x/units/U-003-c.md", &unit_file("U-003", "C-002"));
    let inst = repo.inst("specs/x");
    // per contract: (named and not consuming, consuming and not named)
    type Mismatch = Option<(Vec<String>, Vec<String>)>;
    let got: Vec<(String, Mismatch)> = contract::list(&inst.handle())
        .unwrap()
        .iter()
        .map(|c| (c.id.clone(), contract::consumer_mismatch(c, &inst.units)))
        .collect();
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        got,
        [
            ("C-001".to_string(), None),
            (
                "C-002".to_string(),
                Some((ids(&["U-002", "U-009"]), ids(&["U-003"])))
            ),
            ("C-003".to_string(), None),
            ("C-004".to_string(), None),
            ("C-005".to_string(), Some((ids(&[]), ids(&["U-002"])))),
        ]
    );
}

// ------------------------------------------------------------------- pins

fn pinned(id: &str, producer: &str, pin: &str) -> String {
    format!(
        "# {id} — Pool API (pinned)\n\n- Producer: {producer}\n- Consumers: U-001\n- Stability: Frozen by its producer\n- Pin: {pin}\n"
    )
}

#[test]
fn schema_versions_read_and_refuse() {
    let meta = |block: &str| model::Meta(vitrify_core::yamlish::parse(block));
    for block in [
        "schema_version: 2.0\n",
        "schema_version: 2\n",
        "schema_version: \"2.0.0\"\n",
        "schema_version: 2.0   # pins\n",
    ] {
        assert_eq!(model::schema_of(&meta(block)), Ok(Schema::V2_0), "{block}");
    }
    for block in [
        "schema_version: 2.1\n",
        "schema_version: \"2.1.0\"\n",
        "schema_version: 2.1.3\n",
    ] {
        assert_eq!(model::schema_of(&meta(block)), Ok(Schema::V2_1), "{block}");
    }
    assert_eq!(
        model::schema_of(&meta("sdd: x\n")),
        Err(model::SchemaRefusal::Missing)
    );
    assert_eq!(
        model::schema_of(&meta("schema_version: —\n")),
        Err(model::SchemaRefusal::Missing)
    );
    for block in [
        "schema_version: 1.0\n",
        "schema_version: 1.1\n",
        "schema_version: 2.2\n",
        "schema_version: 3.0\n",
        "schema_version: 2.1.0.1\n",
        "schema_version: one\n",
    ] {
        assert!(
            matches!(
                model::schema_of(&meta(block)),
                Err(model::SchemaRefusal::Unread(_))
            ),
            "{block}"
        );
    }

    // an SDD of a version this vitrify does not read is refused, naming
    // the versions it reads
    let repo = Repo::new();
    bundle(&repo, "other", "schema_version: 1.1\n", &[]);
    let err = Instance::load(repo.sdd("specs/other")).unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert!(
        err.to_string()
            .contains("schema_version 1.1 is not one this vitrify reads (2.0 and 2.1)"),
        "{err}"
    );
    bundle(&repo, "bare", "schema_version: —\n", &[]);
    let err = Instance::load(repo.sdd("specs/bare")).unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert!(
        err.to_string()
            .contains("schema_version is missing; this vitrify reads 2.0 and 2.1"),
        "{err}"
    );
}

#[test]
fn a_pin_stands_for_the_body_it_hashes_and_is_followed_only_while_it_matches() {
    let repo = Repo::new();
    bundle(&repo, "pool", "", &[("C-010-api.md", &authority("C-010"))]);
    let hash = contract::pin_hash(BODY);
    assert_eq!(hash, trace::hash12(BODY), "the one hashing implementation");
    bundle(
        &repo,
        "heldout",
        "",
        &[
            (
                "C-012-pool-api.md",
                &pinned(
                    "C-012",
                    "external — pool, C-010",
                    &format!("pool C-010 @ {hash}"),
                ),
            ),
            (
                "C-013-pool-api.md",
                &pinned(
                    "C-013",
                    "external — pool, C-010",
                    "pool C-010 @ 000000000000",
                ),
            ),
            // written by hand, no hash yet
            (
                "C-014-pool-api.md",
                &pinned("C-014", "external — pool, C-010", "pool C-010"),
            ),
        ],
    );
    repo.write(
        "specs/heldout/units/U-001-a.md",
        &unit_file("U-001", "C-012"),
    );
    repo.write(
        "specs/heldout/units/U-002-b.md",
        &unit_file("U-002", "C-013"),
    );

    let ok = repo.one("specs/heldout", "C-012");
    assert_eq!((ok.kind, ok.state), (Kind::Pin, State::Ok));
    assert_eq!(ok.hash.as_deref(), Some(hash.as_str()));
    let stale = repo.one("specs/heldout", "C-013");
    assert_eq!(stale.state, State::Stale);
    assert!(
        stale.detail.contains(&format!(
            "its pin 000000000000 is not the hash of pool C-010's body (now {hash})"
        )),
        "{}",
        stale.detail
    );
    let unset = repo.one("specs/heldout", "C-014");
    assert_eq!(unset.state, State::Stale);
    assert!(
        unset.detail.contains("records no hash yet"),
        "{}",
        unset.detail
    );

    // the reading set: the pinned contract's own entry, then the body it
    // stands for — read where it lives, carried as the text that was hashed
    let inst = repo.inst("specs/heldout");
    let peers = repo.peers();
    let items = readset::resolve(&inst, "U-001", &peers).unwrap();
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Ground Rules",
            "Technology Decisions",
            "U-001",
            "C-012",
            "C-012 → pool C-010"
        ]
    );
    let body = items.last().unwrap();
    assert_eq!(
        body.path,
        repo.root.join("specs/pool/contracts/C-010-api.md")
    );
    assert_eq!(body.region.as_deref(), Some("Surface onward"));
    assert_eq!(body.bytes, BODY.len() as u64);
    assert_eq!(readset::text_of(&inst, body).unwrap(), BODY);
    // the entry itself is read as any contract is
    let own = &items[3];
    assert!(own.text.is_none() && own.region.is_none());
    assert!(
        readset::text_of(&inst, own)
            .unwrap()
            .contains("- Pin: pool C-010 @")
    );

    // a pin that no longer matches is not followed: a unit is never handed
    // text its trace does not cover
    let err = readset::resolve(&inst, "U-002", &peers).unwrap_err();
    assert_eq!(err.exit_code(), 4);
    let text = err.to_string();
    assert!(
        text.starts_with("error: pinned contract C-013 does not resolve: its pin"),
        "{text}"
    );
    assert!(text.ends_with(" → vitrify pin sync --all"), "{text}");

    // the trace recipe is what it always was: the consumer's own file
    let (map, problems) = trace::compute(&inst, inst.unit("U-002").unwrap()).unwrap();
    assert!(problems.is_empty());
    let own_text = repo.read("specs/heldout/contracts/C-013-pool-api.md");
    assert_eq!(map[1], ("C-013".to_string(), trace::hash12(&own_text)));

    // a sync records the hash; the file — and so the trace — changes with it
    assert_eq!(repo.sync("specs/heldout"), 2);
    let synced = repo.read("specs/heldout/contracts/C-013-pool-api.md");
    assert!(
        synced.ends_with(&format!("- Pin: pool C-010 @ {hash}\n")),
        "{synced}"
    );
    assert_eq!(synced.matches('\n').count(), own_text.matches('\n').count());
    assert!(
        repo.read("specs/heldout/contracts/C-014-pool-api.md")
            .contains(&format!("@ {hash}\n"))
    );
    let inst = repo.inst("specs/heldout");
    assert_ne!(
        trace::compute(&inst, inst.unit("U-002").unwrap())
            .unwrap()
            .0[1],
        map[1]
    );
    assert!(readset::resolve(&inst, "U-002", &repo.peers()).is_ok());

    // the authority moves: every pin on it is stale at once
    repo.write(
        "specs/pool/contracts/C-010-api.md",
        &authority("C-010").replace("SHALL open", "SHALL open, once"),
    );
    for id in ["C-012", "C-013", "C-014"] {
        assert_eq!(repo.one("specs/heldout", id).state, State::Stale, "{id}");
    }
    // a change to the authority's header alone moves nothing
    repo.write(
        "specs/pool/contracts/C-010-api.md",
        &authority("C-010").replace("- Consumers: U-002", "- Consumers: U-002, U-003"),
    );
    for id in ["C-012", "C-013", "C-014"] {
        assert_eq!(repo.one("specs/heldout", id).state, State::Ok, "{id}");
    }
}

#[test]
fn pins_say_one_thing() {
    let repo = Repo::new();
    bundle(&repo, "pool", "", &[("C-010-api.md", &authority("C-010"))]);
    let hash = contract::pin_hash(BODY);
    let good = format!("pool C-010 @ {hash}");
    bundle(
        &repo,
        "new",
        "",
        &[
            (
                "C-001-ok.md",
                &pinned("C-001", "external — pool, C-010", &good),
            ),
            (
                "C-002-shape.md",
                &pinned("C-002", "external — pool, C-010", "pool @ nothing"),
            ),
            ("C-003-local.md", &pinned("C-003", "U-001", &good)),
            (
                "C-004-both.md",
                &format!(
                    "{}\n{BODY}",
                    pinned("C-004", "external — pool, C-010", &good)
                ),
            ),
            (
                "C-005-two.md",
                &pinned("C-005", "external — pool, C-011", &good),
            ),
            (
                "C-006-gone.md",
                &pinned("C-006", "external", "pool C-099 @ 000000000000"),
            ),
            (
                "C-007-nosdd.md",
                &pinned("C-007", "external", "billing C-001 @ 000000000000"),
            ),
            // a pin on a pin: there is no body there to stand for
            (
                "C-008-chain.md",
                &pinned("C-008", "external", "new C-001 @ 000000000000"),
            ),
            (
                "C-009-twins.md",
                &pinned("C-009", "external", "twin-a C-001 @ 000000000000"),
            ),
            (
                "C-010-self.md",
                &pinned("C-010", "external", "new C-010 @ 000000000000"),
            ),
        ],
    );
    // two SDDs sharing a slug
    bundle(&repo, "twin-a", "", &[("C-001-x.md", &authority("C-001"))]);
    repo.write("specs/twin-b/sdd.md", &core("twin-a", ""));
    let state = |id: &str| {
        let e = repo.one("specs/new", id);
        (e.state, e.detail, e.fix.is_some())
    };
    assert_eq!(state("C-001").0, State::Ok);
    let (s, why, fix) = state("C-002");
    assert_eq!((s, fix), (State::Invalid, false));
    assert!(
        why.contains("does not read `<slug> C-### @ <hash>`"),
        "{why}"
    );
    let (s, why, _) = state("C-003");
    assert_eq!(s, State::Invalid);
    assert!(why.contains("must begin `external`"), "{why}");
    let (s, why, _) = state("C-004");
    assert_eq!(s, State::Invalid);
    assert!(why.contains("still carries a body"), "{why}");
    let (s, why, _) = state("C-005");
    assert_eq!(s, State::Invalid);
    assert!(
        why.contains("Producer line names pool C-011, its pin pool C-010"),
        "{why}"
    );
    let (s, why, fix) = state("C-006");
    assert_eq!((s, fix), (State::MissingAuthority, false));
    assert!(why.ends_with("pool has no contract C-099"), "{why}");
    let (s, why, _) = state("C-007");
    assert_eq!(s, State::MissingAuthority);
    assert!(
        why.contains("no SDD in this repository has the slug `billing`"),
        "{why}"
    );
    let (s, why, _) = state("C-008");
    assert_eq!(s, State::Invalid);
    assert!(why.contains("new C-001 has no `Surface` heading"), "{why}");
    let (s, why, _) = state("C-009");
    assert_eq!(s, State::Invalid);
    assert!(why.contains("used by several SDDs"), "{why}");
    let (s, why, _) = state("C-010");
    assert_eq!(s, State::Invalid);
    assert!(why.contains("names itself"), "{why}");
    for s in [State::Stale, State::MissingAuthority, State::Invalid] {
        assert!(s.is_finding());
    }
}

#[test]
fn a_single_file_sdd_pins_and_is_pinned() {
    let repo = Repo::new();
    // the authority is a region, with blank lines after it that are not its
    single(
        &repo,
        "tiny",
        "",
        &format!(
            "### C-001 — Pool API\n\n- Producer: U-001\n- Consumers: —\n\n{}\n\n\n",
            deeper(PLAIN)
        ),
        "",
    );
    let served = deeper(PLAIN);
    let hash = contract::pin_hash(&served);
    single(
        &repo,
        "peer",
        "",
        "### C-001 — Pool API (pinned)\n\n- Producer: external — tiny, C-001\n- Consumers: U-001\n- Pin: tiny C-001 @ 000000000000\n\n### C-002 — Local\n\n- Producer: U-001\n- Consumers: —\n\n#### Surface\n\nlocal\n\n",
        "C-001",
    );
    let rel = "specs/peer.sdd.md";
    assert_eq!(repo.one(rel, "C-001").state, State::Stale);
    assert_eq!(repo.sync(rel), 1);
    let text = repo.read(rel);
    assert!(
        text.contains(&format!("- Pin: tiny C-001 @ {hash}\n\n### C-002 — Local")),
        "{text}"
    );
    assert_eq!(repo.one(rel, "C-001").state, State::Ok);

    let inst = repo.inst(rel);
    let items = readset::resolve(&inst, "U-001", &repo.peers()).unwrap();
    let body = items.last().unwrap();
    assert_eq!(body.path, repo.root.join("specs/tiny.sdd.md"));
    assert_eq!(
        body.region.as_deref(),
        Some("Interface Contracts / C-001 / Surface onward")
    );
    // served as it lies in its file, without the blank lines after it
    assert_eq!(readset::text_of(&inst, body).unwrap(), served);
    assert_eq!(
        items[3].region.as_deref(),
        Some("Interface Contracts / C-001")
    );
}

#[test]
fn an_sdd_outside_discovery_still_finds_itself() {
    // `--sdd` may name an SDD discovery does not find; its own contracts
    // are still there to be pinned
    let repo = Repo::new();
    repo.write("elsewhere/kept/sdd.md", &core("kept", ""));
    repo.write("elsewhere/kept/contracts/C-001-a.md", &authority("C-001"));
    repo.write(
        "elsewhere/kept/contracts/C-002-b.md",
        &pinned(
            "C-002",
            "external — kept, C-001",
            &format!("kept C-001 @ {}", contract::pin_hash(BODY)),
        ),
    );
    let inst = repo.inst("elsewhere/kept");
    let none = Peers::of(Vec::new());
    let got = contract::examine(&inst, &none).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].state, State::Ok);
    assert!(Path::new(&got[0].file).ends_with("C-002-b.md"));
}

// ------------------------------------------------------------ convergence

/// A small deterministic generator: the same bodies on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % below
    }
}

/// A contract body in bundle form (`## Surface` …): headings, prose, blank
/// lines, fenced code with `#` lines in it, and any ending — one newline,
/// several, or none. `single_safe` keeps `##`/`###` lines out of the code,
/// which is what a single-file SDD can hold.
fn random_body(rng: &mut Lcg, single_safe: bool) -> String {
    let mut out = String::from("## Surface\n");
    for n in 0..rng.next(12) {
        match rng.next(7) {
            0 => out.push('\n'),
            1 => out.push_str(&format!("line {n} of prose, with a # in it\n")),
            2 => out.push_str("## Semantics\n"),
            3 => out.push_str(&format!("### Detail {n}\n")),
            4 => {
                let fence = if rng.next(2) == 0 { "```" } else { "~~~~" };
                out.push_str(&format!("{fence}text\n# a comment in code\n"));
                if !single_safe {
                    out.push_str("## SYNOPSIS\n### not a heading\n");
                }
                out.push_str(&format!("  indented {n}\n{fence}\n"));
            }
            5 => out.push_str(&format!("- THE SYSTEM SHALL do thing {n}.\n")),
            _ => out.push_str("    # indented code, not a heading\n"),
        }
    }
    match rng.next(4) {
        0 => {
            out.pop(); // no newline at the end of the file
        }
        1 => out.push('\n'),
        2 => out.push_str("\n\n"),
        _ => {}
    }
    out
}

#[test]
fn a_pin_follows_any_body_and_serves_what_it_hashed() {
    let mut rng = Lcg(0xfeed);
    for round in 0..40 {
        let body = random_body(&mut rng, true);
        let repo = Repo::new();
        bundle(
            &repo,
            "afile",
            "",
            &[(
                "C-001-a.md",
                &format!("# C-001 — a\n\n- Producer: U-001\n- Consumers: —\n\n{body}"),
            )],
        );
        single(
            &repo,
            "aregion",
            "",
            &format!(
                "### C-001 — a\n\n- Producer: U-001\n- Consumers: —\n\n{}\n\n",
                deeper(body.trim_end())
            ),
            "",
        );
        bundle(
            &repo,
            "pins",
            "",
            &[
                (
                    "C-001-f.md",
                    &pinned("C-001", "external — afile, C-001", "afile C-001"),
                ),
                (
                    "C-002-r.md",
                    &pinned("C-002", "external", "aregion C-001 @ 000000000000"),
                ),
            ],
        );
        repo.write(
            "specs/pins/units/U-001-a.md",
            &unit_file("U-001", "C-001, C-002"),
        );
        assert_eq!(repo.sync("specs/pins"), 2, "round {round}");
        assert_eq!(repo.sync("specs/pins"), 0, "round {round}");
        let inst = repo.inst("specs/pins");
        let items = readset::resolve(&inst, "U-001", &repo.peers()).unwrap();
        assert_eq!(items.len(), 7, "round {round}");
        // what is served is what the recorded hash is the hash of
        for (served, file) in [(&items[4], "C-001-f.md"), (&items[6], "C-002-r.md")] {
            let text = readset::text_of(&inst, served).unwrap();
            let entry = repo.read(&format!("specs/pins/contracts/{file}"));
            assert!(
                entry
                    .trim_end()
                    .ends_with(&format!("@ {}", contract::pin_hash(&text))),
                "round {round} {file}: {entry}"
            );
            assert_eq!(served.bytes, text.len() as u64);
        }
        // a file's body is served byte for byte
        assert_eq!(
            readset::text_of(&inst, &items[4]).unwrap(),
            body,
            "round {round}"
        );
    }
}
