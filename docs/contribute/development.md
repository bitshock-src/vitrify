# Develop vitrify

> Build, test, and change vitrify from source: the toolchain, the workspace, the build and code conventions, the golden corpus, help text, the two test tiers, and publishing a release.

Use this page when you change vitrify itself. To use vitrify, install the released binary instead — see [Installation](../getting-started/installation.md). This page states the conventions the codebase holds to and how a change is verified; [Architecture](../about/architecture.md) explains the components, and [Design Rationale](../about/design-rationale.md) explains why they are built this way.

> [!NOTE]
> Bracketed keys such as `[insta]` cite [Sources](../reference/sources.md). `RA-n` refers to a [rejected alternative](../about/design-rationale.md#rejected-alternatives).

## Source Development

Set up a working tree with these tools.

| Requirement | Version | Needed for |
| --- | --- | --- |
| Rust toolchain | 1.99.1 with `rustfmt` and `clippy`, pinned by `rust-toolchain.toml` | building and testing; `rustup` selects it automatically |
| git | 2.30 or later | every test that drives a repository |
| `python3` | 3.10 or later | the tier-2 battery, which builds the calc example with its own §5 Commands |

Clone, build, and run the tier-1 suite:

```sh
git clone https://github.com/bitshock-src/vitrify.git
cd vitrify
cargo build --workspace --locked
cargo test --workspace --locked
```

### Workspace Layout

Use this table to find where each part of vitrify lives.

| Path | Holds |
| --- | --- |
| `crates/vitrify-core/` | the core library, published as `vitrify-core`: discovery and front matter, masking and trace hashing, ledger, journal, audit, claim planning, the content registry |
| `crates/vitrify-cli/` | the `vitrify` binary, published as `vitrify-cli`: the command surface and its help text (`src/cli.rs`, `src/help/`), the orchestrator, process management, and the git layer |
| `crates/stub-worker/` | the deterministic stand-in worker the tests use in place of a model (unpublished) |
| `crates/vitrify-core/assets/` | the embedded content: briefs, the escalation template, schemas, templates, and `config.toml`, the defaults `vitrify init` writes; inside the crate, so the published `vitrify-core` builds on its own |
| `fixtures/example/` | the calc example: the primary fixture, and the shipped example the quick start builds |
| `fixtures/golden/` | every verb's expected output, for the current `schema_version` |
| `fixtures/seeded/` | seeded finding sets for audit and lint drift cases, and seeded projects for `vitrify run` |
| `.github/workflows/ci.yml` | continuous integration |

## Build Conventions

These rules govern the toolchain, dependencies, and CI.

Rust edition 2024, toolchain pinned by `rust-toolchain.toml` (rustup documents the toolchain file as the per-repo pin, "suitable to check in to source control" [rustup-overrides] [rust-release]). The published crates declare `rust-version = "1.89"`, the oldest `rustc` that builds them (`File::lock` is stable from 1.89), for users who install with their own toolchain; CI's `msrv` job holds it, so raise it only with a reason. Dependencies: `clap` (derive), `serde`/`serde_json`, `toml`, `sha2`, `thiserror` (core) / `anyhow` (bin), `fs-err`; dev: `assert_cmd`/`predicates`, `insta`, `tempfile` — the CLI working group's crate canon [rust-cli-book]; versions verified against the registry [crates-io]; caret requirements with the committed `Cargo.lock` as the exact pin and `--locked` on every gate command — lockfiles "extend the idea of pinning by freezing the entire dependency graph", and the literature recommends lockfiles over hard pinning [lockfiles] [cargo-faq]. No `build.rs` anywhere. Signal handling adds no crate: the four C library calls it needs (`signal`, `raise`, `kill`, `_exit`) are declared in `signals.rs`, and the handler does nothing but store to an atomic. CI: GitHub Actions, `macos-latest` × `ubuntu-latest`, toolchain via `dtolnay/rust-toolchain` [dtolnay-action], fmt + clippy `-D warnings` + build + test, all `--locked` — multi-OS CI on every change, per the Bitcoin Core and fish-port discipline [bitcoin-ci] [fish-port].

## Code Conventions

These rules govern how the code is written.

Code conventions: exit codes map through one `ExitCode` enum, converted to a process exit only in `main`; no panics on user input (`unwrap` only in tests and for `// invariant:` comments); file I/O through `fs-err` with strict UTF-8 (invalid UTF-8 in a spec file is exit 1); normative JSON serialized from structs or ordered pairs, never from maps; UTC only, one time helper; directory listings sorted before use — no OS iteration order reaches an output; subprocesses argv-style, never through a shell (single fixed exception: Verification-table rows via `sh -c`, by design).

## Testing

vitrify is tested against its golden corpus and its help snapshots, in two tiers. Run the narrowest test that covers your change first, then the full suite for each tier your change can affect.

### The Golden Corpus

Enforcement is approval-style golden testing: golden files generated and committed become the specification [approvals], with output disagreement as the bug signal — "a single input is passed to multiple systems that are expected to produce the same output; if the systems disagree … a bug … has been detected" [mckeeman]. The oracle is committed in-repo (the strongest pin, hermetic for CI): `fixtures/example/` (the calc instance; trace hashes are computed over the bytes under its `specs/calc/`, so they are never edited by hand), `fixtures/golden/` (every verb's expected output, `{ROOT}`/`{DATE}`-normalized), and `fixtures/seeded/` (seeded finding sets for audit and lint drift cases). What reaches across SDDs has fixtures of its own, a repository of three SDDs — a producing bundle, a consuming bundle and a consuming single file: `fixtures/seeded/pin-multi/` (pins that hold and pins that do not), with its expected outputs under `fixtures/golden/pin-multi/`. Precedent for committing the oracle in-tree: the fish-shell port kept its reference implementation in-tree to compare throughout [fish-port]; committed snapshots are the insta norm ("they look best under version control") [insta-docs]; for the shape (Rust reimplementation verified by a snapshot harness against approved output): brush [brush]; gitoxide's practice is "compare against git when applicable" [rust-git].

The corpus is a set of regression tests for the current `schema_version`. It is regenerated only by a change that raises `schema_version` — a change to the document format or to the trace recipe — and in that same reviewed change ([Versioning](../reference/formats.md#10-versioning)). Any other difference between the code and the corpus is a bug in the code: one masking-rule drift silently stales every existing unit [mokhov], so a trace hash that moves without a schema change is fixed on vitrify's side.

### Test Tiers

Two test tiers. **Tier-1** (every `cargo test`; offline, hermetic): byte/set equality against the corpus — trace hash maps, ledger bytes, locate/readset JSON, audit finding sets `(check, subject)`, replay results — with snapshot assertions via insta [insta]; the help snapshots and the conformance test ([Help Text](#help-text)); and `vitrify run` itself, end to end on small seeded projects with the stub worker (`run_e2e.rs`, `run_integrate.rs`, `rehearsal_claim.rs`): several SDDs in one repository, interrupts and ignored signals, a run that died, retries and escalation, spec changes mid-run, publishing through an origin, the primary checkout following with local changes aboard, rows side by side, a stale pin refused at preflight and built after its sync, a pinned contract read through its pin and its unit set aside when the producer moves mid-run. **Tier-2** (`cargo test --workspace --locked -- --ignored`; permanent CI stage, no runtime beyond the test binaries and git): the end-to-end battery (`crates/vitrify-cli/tests/e2e.rs`) — a full calc generation pass via `vitrify run` with the stub worker, protocol reset to Complete with the evidence triple, zero spec-body edits, no re-dispatch on a Verified tree, plus the kill-a-worker fencing rehearsal — and the claim race rehearsals (local CAS and bare-origin push arbitration).

### Build the calc Example Without a Model

The tier-2 battery's `e2e.rs` builds the calc example through `vitrify run` with no harness and no tokens: the deterministic stub worker stands in for the implementer, with `fixtures/example/reference/` as its canned solution. The whole circuit — spec → claims → worktrees → gates → fenced serial integration → audit → replay — is proven this way on every CI run. Run it from the root of your checkout:

```sh
cargo test -p vitrify-cli --test e2e -- --ignored
```

It resets the protocol fields on a scratch copy of the example, drives `vitrify run` to **Complete** with the evidence triple, asserts zero spec-body edits in either direction, and rehearses the kill-a-worker fencing case: a lapsed claim's green branch must be rejected at the gate. It passes when cargo reports `test result: ok`. The stub worker copies the reference solution and stamps it Built, so the worker-side steps — observing tests fail first, running the unit gate — are not exercised; only a build through a real harness exercises them, as in the [Quick Start](../getting-started/quick-start.md#build-the-shipped-example).

### Help Text

Each command's `--help` is content the binary ships, and agents read it to call the command correctly ([Design Rationale](../about/design-rationale.md#agent-first-presentation), goal G7). The headings follow kubectl's rule that every command carry a long text on its input and output and an example block, checked by a script [kubectl-conv]; the tests below are that script, because tool documentation drifts from the tool it describes [draft]. The text lives in `crates/vitrify-cli/src/cli.rs` (the command tree, its flags, and the root help) and `crates/vitrify-cli/src/help/` (each verb's long help). Every verb's `--help` carries these headings, in this order:

| Heading | Holds | Present |
| --- | --- | --- |
| `Reads:` | the files and state the command reads | where it applies |
| `Writes:` | what it writes, and whether it commits | where it applies |
| `Prints:` | its stdout lines and `--json` fields, and its stderr hints | where it applies |
| `Exit status:` | each exit code it returns and what it means | always |
| `Config:` | the configuration keys it reads | where it applies |
| `Examples:` | command lines to adapt | always |
| `Procedure:` | the `vitrify brief` task it belongs to | where it applies |

Two tier-1 tests hold the content:

- **Help snapshots.** Insta snapshots hold the root help and every command's `--help` output. A change to help text changes a snapshot; review it with `cargo insta review` and commit the accepted `.snap` files with the change [insta].
- **The conformance test.** Every `vitrify …` command line in `crates/vitrify-core/assets/` (briefs, schemas, templates) and `docs/` is parsed against the command tree the binary builds, in-process. A line that names a command or flag the binary does not have fails the test. Placeholders such as `<root>`, `U-###`, `[--follow]` and `…` are allowed.

### Common Commands

Use this table to pick the commands for a change.

| Change | Run |
| --- | --- |
| any change | `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`, then `cargo test --workspace --locked` |
| the orchestrator, claims, or the git layer | also `cargo test --workspace --locked -- --ignored` (tier 2) |
| the calc build end to end, without a model | `cargo test -p vitrify-cli --test e2e -- --ignored` ([Build the calc Example Without a Model](#build-the-calc-example-without-a-model)) |
| help text | `cargo test -p vitrify-cli --test cli_help`, then `cargo insta review` for the changed snapshots |
| an `insta` snapshot | review it with `cargo insta review` (from the `cargo-insta` tool); regenerate a file under `fixtures/golden/` only in a change that raises `schema_version` |

## Continuous Integration

CI runs on GitHub Actions for every push and pull request: `test` and `tier-2` on `ubuntu-latest` and `macos-latest` with the pinned toolchain, and `package` and `msrv` on `ubuntu-latest`. Use this table to see what each job runs.

| Job | Runs |
| --- | --- |
| `test` | `cargo fetch --locked`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo build --workspace --locked`, `cargo test --workspace --locked` |
| `tier-2` | after `test` passes: `cargo test --workspace --locked -- --ignored` |
| `package` | `cargo package -p vitrify-core -p vitrify-cli --locked`: packages the two published crates and builds each from its tarball, `vitrify-cli` against the packaged `vitrify-core` |
| `msrv` | `cargo +1.89.0 check -p vitrify-cli --locked` with Rust 1.89.0, the published crates' `rust-version` |

## Publish a Release

vitrify is published on crates.io as two crates, released together at one version: `vitrify-core`, the library, and `vitrify-cli`, which installs the `vitrify` binary and requires exactly the `vitrify-core` of its own release. `stub-worker` is never published. Each published crate carries its own `README.md` — its crates.io page, with links that are absolute because crates.io resolves relative links against the crate's directory — and holds only its sources and, for `vitrify-core`, the embedded content; the tests stay in the repository with the fixtures they read. Publishing needs a crates.io API token with publish rights to both crates (`cargo login`). To publish a release:

1. Set the version in the root `Cargo.toml`: `version` under `[workspace.package]`, and the `=` requirement on `vitrify-core` under `[workspace.dependencies]`. Then run `cargo update --workspace`, so `Cargo.lock` records it.
2. Update everything that names the version: the root help snapshot (`cargo insta review`), the install commands and sample outputs in `README.md`, the [Installation](../getting-started/installation.md) and [Quick Start](../getting-started/quick-start.md) pages and the [Command Reference](../cli/command-reference.md), and a new entry in the [Release Notes](../about/release-notes.md).
3. Run both test tiers and every `test` job command ([Common Commands](#common-commands)).
4. Rehearse the publish. It packages both crates in dependency order and builds each from its tarball, then stops short of uploading:

   ```sh
   cargo publish --workspace --dry-run --locked
   ```

5. Commit, tag the commit `v<version>`, and push both.
6. Publish. cargo uploads `vitrify-core`, waits until the registry serves it, then uploads `vitrify-cli`:

   ```sh
   cargo publish --workspace --locked
   ```

A version on crates.io is permanent: it can be yanked (`cargo yank`), never replaced, so a fix ships as a new version.

## Change the Design

Some changes need a design revision rather than code alone. Check these rules before you start.

- Future scope arrives only as a new, reviewed design revision. Adding an inference point requires a design revision, not code alone.
- A change to the document format or to the trace recipe raises `schema_version`, and regenerates the golden corpus in the same reviewed change ([Versioning](../reference/formats.md#10-versioning)). Any other trace-hash drift is a bug, fixed on vitrify's side.
- A change to a command, flag, output or exit code changes its help text, its snapshots and the [Command Reference](../cli/command-reference.md) in the same change; a release may make it with no alias or deprecation period.
- New documentation for agents goes into one of the three places the binary serves — a brief, a schema, or a command's help — and nowhere else; vitrify writes nothing into a repository beyond what its verbs own (R8, R9 in [Design Rationale](../about/design-rationale.md#requirements-and-goals)).
- A rejected option is recorded as a new `RA-n` entry in [Design Rationale](../about/design-rationale.md#rejected-alternatives).

## Keep the Documentation Current

Change the documentation in the same change as the behavior it describes. Use this table to find the page that owns each kind of fact, and change it there only.

| Fact | Owning page |
| --- | --- |
| a command, flag, exit code, or verb output | the command's help text in `crates/vitrify-cli/src/`, and [Command Reference](../cli/command-reference.md) |
| a configuration key or default | [Configuration](../cli/configuration.md) and `crates/vitrify-core/assets/config.toml`; its basis in [Design Rationale](../about/design-rationale.md#thresholds) |
| a harness spelling, role, or default model | [Harness CLIs](../cli/harnesses.md) and `crates/vitrify-core/assets/config.toml` |
| a run output line or run-record field | [Monitor a Run](../user-guides/monitor-a-run.md) |
| a file format or hash rule | [Data Model and Formats Specification](../reference/formats.md) |
| supported platforms and harnesses | [Support Matrix](../reference/support-matrix.md) |
| a term | [Glossary](../resources/glossary.md) |
| what a release changed | [Release Notes](../about/release-notes.md), briefly |
| a design choice or rejected alternative | [Design Rationale](../about/design-rationale.md), citing [Sources](../reference/sources.md) |

The agent-facing procedures and schemas live in `crates/vitrify-core/assets/` and are compiled into the binary. Keep them operative and lean, and record the grounding behind their rules in Design Rationale instead.

## Next Steps

- [Architecture](../about/architecture.md) — find the component your change touches.
- [Data Model and Formats Specification](../reference/formats.md) — check whether your change reaches the document format or the trace recipe, and so raises `schema_version`.
- [Design Rationale](../about/design-rationale.md) — read the closed doors before reopening one.
