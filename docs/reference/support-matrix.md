# Support Matrix

> Supported vitrify platforms, harness CLIs, SDD schema versions, and runtime dependencies.

Use this page as the single source of support claims; every other page that mentions platforms or harnesses defers to it. **Supported** means vitrify is designed for and documented on that combination. vitrify publishes no prebuilt binaries: you build it with cargo, from [crates.io](https://crates.io/crates/vitrify-cli) or from source, on the platform where you run it. Continuous integration builds and tests every change on GitHub Actions' `macos-latest` and `ubuntu-latest` runners; CI coverage alone is not a wider commitment. What each release changed is in the [Release Notes](../about/release-notes.md).

## Platforms

Use this table to check your operating system.

| Platform | Status | Notes |
| --- | --- | --- |
| macOS | Supported | Covered by CI. |
| Linux | Supported | Covered by CI. |
| Windows | Through WSL 2 | Windows runs the Linux build inside WSL 2 — the harness vendors' own documented path. Native Windows is not supported, and WSL 2 is not covered by CI. |

Workers and gate commands run as Unix process groups, and `vitrify run` handles Unix signals itself.

## Runtime Dependencies

The runtime dependency set is exactly git. Use this table to see what each dependency is needed for.

| Dependency | Version | Needed for | Notes |
| --- | --- | --- | --- |
| git | 2.30 or later | every command | vitrify's correctness rests on git's own compare-and-swap and worktree semantics. `vitrify init` and `vitrify probe` check that git runs, not its version. |
| A harness CLI | see [Harness CLIs](#harness-clis) | `vitrify run` and interactive builds | Optional for every other command. |
| Your project's tools | whatever its §5 Commands need | the gates and audit check 4 | For example, `python3` 3.10 or later for the calc example. |

## Harness CLIs

vitrify drives exactly three harness CLIs, through their headless modes. Use this table to see each, the streaming headless command `vitrify init` and `vitrify probe` probe for, and the versions the shipped role defaults were researched against.

| Harness | Executable | Streaming spelling | Defaults researched against |
| --- | --- | --- | --- |
| Claude Code | `claude` | `claude -p --output-format stream-json --verbose` | Claude Code 2.1.319 |
| Codex | `codex` | `codex exec --json` | Codex CLI 0.167.0 |
| OpenCode | `opencode` | `opencode run --format json` | OpenCode 1.19.5 and 2.2.5 |

Each harness has one spelling. A harness CLI that does not accept it fails `vitrify init` and `vitrify probe`, which name the spelling it needs. Other versions of these CLIs are probed the same way: the probe tries the spelling and each role's flags, so a version that rejects them fails there rather than mid-run. vitrify supports no other harness and no direct model-API backend.

## SDD Schema Versions

Use this table to check which `schema_version` values vitrify reads. When the version changes is set out in [Versioning](formats.md#10-versioning).

| `schema_version` | Status |
| --- | --- |
| `2.0` (also written `2` or `2.0.x`) | Supported; the one version this vitrify reads. |
| absent, or any other value | Refused with exit 4; the message names the version this vitrify reads. `vitrify locate` still lists the SDD. |

## Build Toolchain

Use this table to check the Rust toolchain: installing from crates.io needs the minimum version, and a checkout builds with the pinned one.

| Component | Version |
| --- | --- |
| Rust, installing from crates.io | 1.89 or later — the published crates' `rust-version`, held by CI |
| Rust, in a checkout | 1.99.1, pinned by `rust-toolchain.toml`, with `rustfmt` and `clippy`; edition 2024 |
| Dependencies | the exact versions in `Cargo.lock` — committed, and published with the crate — used with `--locked` |

## Related Information

- [Installation](../getting-started/installation.md) — installing on a supported platform.
- [Harness CLIs](../cli/harnesses.md) — preparing each harness.
- [Release Notes](../about/release-notes.md) — what changed in each release.
