# Release Notes

> See briefly what changed in each vitrify release, newest first.

Use this page to see what a release changed. Each entry gives the version, its date, one sentence and the changes; the repository's git history is the complete record. vitrify is at 0.y.z, so any release may change any command, output or format — [Versioning](../reference/formats.md#10-versioning) gives the rule.

## Release 0.5.0

2026-11-17

0.5.0 sets a machine up in one place: `vitrify init` writes all of vitrify's configuration to `~/.vitrify/config.toml`, and no other command creates `~/.vitrify/`.

- New: `vitrify init` writes `~/.vitrify/config.toml` with every setting at its default and the harness role profiles, materializes the content, and records a first probe. It refuses a `config.toml` that exists.
- New: `vitrify probe` probes git and the harness CLIs again and updates the probe record in `config.toml`, printing each change and asking first; `-y` (`--assume-yes`) applies without asking.
- Removed: 0.4.0's machine-probe command and its record file, which `vitrify init` and `vitrify probe` replace. The role profiles are the `[harness.<name>]` tables of `config.toml`; `~/.vitrify/harness.toml` is not read. `VITRIFY_PROBE_TIMEOUT_SECS` sets the probe timeout.
- Every command but `vitrify init` exits 4 on a machine `vitrify init` has not set up, and creates nothing.
- A repository's `.vitrify/config.toml` may override any setting and the role profiles, key by key, and declares its own `schema`; one that carries the probe record is refused.
- `vitrify brief init` is `vitrify brief onboard`. The onboard, authoring, kickoff and run procedures leave machine setup to the human.
- Published on crates.io: `cargo install vitrify-cli` installs the `vitrify` binary, with Rust 1.89 or later.

## Release 0.4.0

2026-11-17

0.4.0 puts everything a coding agent needs to operate vitrify into the installed binary.

- `vitrify --help` routes by task, and every command's help carries its outputs, exit codes and examples.
- `vitrify brief` lists the procedures, adding `change` and `run`.
- New: `vitrify check` and `vitrify change apply`.
- Schema 2.0 is required and is the only version read. A contract another SDD owns is consumed through a pin, and `vitrify mirror` is `vitrify pin`.
- A run keeps its final report, which `vitrify run status` prints.
- Removed: `journal replay --verify`, `trace record --json`, `trace record` without `--stamp`, buffered harness spellings.

## Release 0.3.1

2026-11-16

0.3.1 makes a run visible while it runs.

- `vitrify run status` shows a run unit by unit; `--follow` watches it to its end, `--json` prints it as data.
- The integrator records its step, batch and running commands in `gate.json`, which `run status` shows.
- A run appends one JSON object per event to `.vitrify/logs/<job>/events.jsonl`.
- The run record gains `harness`, and each worker `retry` and `detail`.

## Release 0.3.0

2026-11-15

0.3.0 makes several SDDs in one repository a checked configuration.

- External contracts are held to their authorities by lint L9 and listed and synced by the `mirror` verb.
- `lint`, `audit`, `ledger regen`, `ledger progress` and `questions` take `--all`.
- `vitrify questions` lists every question with the units it blocks.
- `vitrify lint --built` and `vitrify ledger regen --check` are added.
- Lint checks L4 and L8 are stricter, and L9–L11 are added.
- Schema 1.1 adds pinned references.

## Release 0.2.0

2026-11-13

0.2.0 fixes the defects found in a large multi-SDD build.

- Work branches carry the SDD: `sdd/<sdd>/<unit>/c<seq>`.
- Unit worktrees and the integration checkout live outside the working tree.
- Dispatch is continuous, and Verification rows run side by side.
- The gate holds the whole suite to "once green, stays green", and rejects work outside a unit's write-set.
- Each gate command and worker gets a `TMPDIR` of its own.
- `--wait`, the `retry` role and signal handling are added.

## Related Topics

- [Installation](../getting-started/installation.md#upgrade) — how to install another release.
- [Data Model and Formats Specification](../reference/formats.md#10-versioning) — the versioning rule.
- [Support Matrix](../reference/support-matrix.md) — supported platforms and harnesses.
