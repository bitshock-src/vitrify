# Installation

> Install the vitrify binary, check its requirements, set the machine up with `vitrify init`, prepare a harness CLI for builds, and upgrade or remove vitrify cleanly.

Use this page to install vitrify for the first time, or to upgrade or remove it. vitrify is one Rust binary, `vitrify`, whose runtime dependency set is exactly git; a harness CLI is needed only to build with `vitrify run`. For the shortest path to a working build, follow the [Quick Start](quick-start.md), which includes these steps.

## Requirements

Check these before installing. The [Support Matrix](../reference/support-matrix.md) is the canonical list of supported platforms and harnesses.

| Requirement | Version | Needed for |
| --- | --- | --- |
| macOS or Linux | — (on Windows, inside WSL 2) | everything |
| git | 2.30 or later | every command |
| Rust and cargo | 1.89 or later | installing with `cargo install`; a checkout builds with the toolchain it pins |
| A harness CLI, signed in | Claude Code, Codex, or OpenCode | `vitrify run` and interactive builds only |
| Your project's own tools | whatever its §5 Commands run | the gates, during a build |

## Install the CLI

Install the version these docs describe from crates.io, where vitrify is published as [`vitrify-cli`](https://crates.io/crates/vitrify-cli); the binary it installs is `vitrify`.

```sh
cargo install vitrify-cli --version 0.5.0 --locked
```

Install `vitrify-cli` by that name: the crate named `vitrify` on crates.io is an unrelated project. `--locked` builds with the exact dependency versions the release was published with. cargo places the binary in `~/.cargo/bin`, which must be on your `PATH`. Everything vitrify shares across repositories lives under `~/.vitrify`.

If cargo refuses because the package requires a newer `rustc`, your toolchain is older than 1.89: update it (`rustup update stable`, if rustup manages it) and run the command again.

### Install From Source

To build from a checkout of the repository instead, run these commands from a directory where you keep source code:

```sh
git clone https://github.com/bitshock-src/vitrify.git
cd vitrify
cargo install --path crates/vitrify-cli --locked
```

`--locked` builds with the exact dependency versions in the committed `Cargo.lock`, and rustup builds with the toolchain that `rust-toolchain.toml` pins — see [Build Toolchain](../reference/support-matrix.md#build-toolchain).

## Verify the Installation

Check the binary:

```sh
vitrify --version
```

```text
vitrify 0.5.0
```

Then set the machine up, once. `vitrify init` creates `~/.vitrify/`, and no other command does:

```sh
vitrify init
```

`init` writes `~/.vitrify/config.toml` with every setting at its default, checks git, probes each harness CLI on your `PATH`, materializes the briefs, schemas and templates, probes each harness role, and records what works in `config.toml`, printing one `status: text` line per check. With only Claude Code installed, the output looks like this, where `<version>` is your git version:

```text
ok: vitrify version 0.5.0
ok: created ~/.vitrify/config.toml with every setting at its default
ok: git present (git version <version>)
ok: harness claude: headless = claude -p --output-format stream-json --verbose
untested: harness codex: CLI not on PATH
untested: harness opencode: CLI not on PATH
ok: ~/.vitrify materialized (17 assets, 0 overridden)
ok: harness claude worker: model=opus effort=medium
ok: harness claude retry: model=opus effort=high
ok: harness claude escalation: model=best effort=xhigh
ok: recorded the probe in ~/.vitrify/config.toml
```

The machine is ready when `init` exits 0. A harness that is not installed is `untested`, never an error. A `FAIL` line names what to fix — see [`vitrify init` or `vitrify probe` Reports FAIL](../resources/troubleshooting.md#vitrify-init-or-vitrify-probe-reports-fail). `init` confirms that git runs; it does not check git's version. Until it has run, every other command refuses: `error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init`.

> [!NOTE]
> Each harness probe is a real one-shot model call on your harness account: one for the harness's streaming spelling, and one for each role that adds flags — four per harness with the shipped defaults.

## Prepare a Harness CLI

`vitrify run` drives inference through a harness CLI you already use; vitrify itself calls no model API and stores no credentials. Follow these steps for each harness you want to build with:

1. Install Claude Code, Codex, or OpenCode by its vendor's instructions, and sign in.
2. Make sure a headless session may edit files and run commands in `~/.vitrify/worktrees/`, where workers build. vitrify passes no permission flag of its own; see [Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions).
3. Run `vitrify probe`. It probes again, prints each change to the record of working headless commands in `~/.vitrify/config.toml`, and asks before it writes it — `vitrify probe -y` writes without asking. `vitrify run` uses only what is recorded there.

Re-run `vitrify probe` whenever you install or upgrade a harness CLI.

## Upgrade

Install the new version over the old one, then re-run `probe`:

```sh
cargo install vitrify-cli --version <version> --locked
vitrify probe
```

`<version>` is the release you are installing. If `probe` reports that the machine is not set up, run `vitrify init` instead. `~/.vitrify/config.toml` is yours: an upgrade never changes your settings or role profiles, and `probe` brings its probe record up to date. Unmodified content in `~/.vitrify/` — briefs, templates and schemas — refreshes to the installed version's; files you have edited are kept, and `probe` flags any that were forked from another shipped version. Any release may change commands, outputs and formats: the [Release Notes](../about/release-notes.md) say briefly what each one changed, and `vitrify check --all` says whether the repository is consistent under the installed version.

## Uninstall

Rollback is non-destructive: stop using vitrify, delete `.vitrify/` and the repository's directory under `~/.vitrify/worktrees/` (`git worktree prune` then forgets them); protocol state lives entirely in the spec files, journal, and ledger.

> [!WARNING]
> These commands delete files. `~/.vitrify/` holds your edited configuration and content overrides, and `~/.vitrify/worktrees/` holds the checkouts of any run still going; stop every run and copy anything you want to keep first.

1. In each repository where you used vitrify, remove its scratch and worktrees. `<repo>-<hash>` is the directory a run's header named on its `Worktrees:` line:

   ```sh
   rm -rf .vitrify
   rm -rf ~/.vitrify/worktrees/<repo>-<hash>
   git worktree prune
   ```

2. Remove the binary and its home:

   ```sh
   cargo uninstall vitrify-cli
   rm -rf ~/.vitrify
   ```

Branches of attempts that did not land, `sdd/<sdd>/<unit>/c<seq>`, remain in the repository until you delete them with git.

## Next Steps

- [Quick Start](quick-start.md) — have your coding agent spec a project, then build it with `vitrify run`.
- [Harness CLIs](../cli/harnesses.md) — role profiles, permissions, and per-harness notes.
- [Configuration](../cli/configuration.md) — the settings you can change.
