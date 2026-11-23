# Command Reference

> Look up every vitrify command — its syntax, options, output, and exit codes — and the conventions all commands share.

Use this page to find a command, then follow its guide for the task around it. `vitrify <command> --help` is complete for the version you have installed — what the command reads, writes and prints, its exit status and examples; this page adds the reasons and the cross-links. `vitrify --help` routes each task to its procedure, and `vitrify brief` lists the procedures.

Commands come in two groups. [User Commands](#user-commands) are what you run to set up the machine, author the spec, start the build and inspect its state. [Agent Commands](#agent-commands) are the protocol verbs that workers, the integrator and kickoff sessions run as they follow their briefs. The [conventions](#conventions) below apply to both.

## Command Index

Use this table to find a command and the guide that covers it.

| Command | Purpose | Guide |
| --- | --- | --- |
| [`vitrify init`](#vitrify-init) | set this machine up: `~/.vitrify/config.toml` with every default, the content, a first probe | [Installation](../getting-started/installation.md), [Configuration](configuration.md) |
| [`vitrify probe`](#vitrify-probe) | probe git and the harness CLIs again; update the probe record in `config.toml`, asking first | [Harness CLIs](harnesses.md) |
| [`vitrify new`](#vitrify-new) | materialize a TDD or SDD instance template | [Author a Spec](../user-guides/author-a-spec.md) |
| [`vitrify schema show`](#vitrify-schema-show) | print the normative SDD or TDD schema | [Author a Spec](../user-guides/author-a-spec.md) |
| [`vitrify lint`](#vitrify-lint) | authoring checks L1–L12 | [Author a Spec](../user-guides/author-a-spec.md) |
| [`vitrify check`](#vitrify-check) | every read-only check over one SDD or all: is the repository consistent | [Change a Spec](../user-guides/change-a-spec.md) |
| [`vitrify pin list`, `pin sync`](#vitrify-pin) | list the external contracts and their states; bring stale pins back in step | [Work With Several SDDs](../user-guides/multiple-sdds.md) |
| [`vitrify audit`](#vitrify-audit) | the seven drift checks | [Monitor a Run](../user-guides/monitor-a-run.md#run-health-checks) |
| [`vitrify change apply`](#vitrify-change-apply) | a change entry's bookkeeping in one step: re-stamp or mark Stale, journal, ledger | [Change a Spec](../user-guides/change-a-spec.md) |
| [`vitrify journal replay`](#vitrify-journal-replay) | check the journal against front matter | [Monitor a Run](../user-guides/monitor-a-run.md#run-health-checks) |
| [`vitrify locate`](#vitrify-locate) | find SDDs; list units and claimability; show the SDD graph | [Monitor a Run](../user-guides/monitor-a-run.md#check-progress-and-state) |
| [`vitrify run`](#vitrify-run) | the deterministic orchestrator | [Run a Build](../user-guides/run-a-build.md) |
| [`vitrify run status`](#vitrify-run-status) | show what a run is doing, unit by unit (read-only) | [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status) |
| [`vitrify ledger progress`](#vitrify-ledger-progress) | print the one-line progress summary | [Monitor a Run](../user-guides/monitor-a-run.md#check-progress-and-state) |
| [`vitrify questions`](#vitrify-questions) | list the open questions and the units they block | [Run a Build](../user-guides/run-a-build.md#resolve-a-blocked-build) |
| [`vitrify brief`](#vitrify-brief) | list the procedures, print one, or generate a worker prompt | [Run a Build](../user-guides/run-a-build.md#3-or-build-interactively) |
| [`vitrify claim`, `renew`, `release`](#vitrify-claim-renew-release) | atomic lease transactions | [Build State](../about/concepts/build-state.md#claims-and-leases) |
| [`vitrify readset`](#vitrify-readset) | a unit's ordered minimal reading set; byte-budget check | [Author a Spec](../user-guides/author-a-spec.md) |
| [`vitrify trace`](#vitrify-trace) | record and verify input-trace hashes; stamp units | [Change a Spec](../user-guides/change-a-spec.md) |
| [`vitrify journal append`](#vitrify-journal-append) | append one event to the journal | [Change a Spec](../user-guides/change-a-spec.md) |
| [`vitrify ledger regen`](#vitrify-ledger-regen) | regenerate the derived ledger | [Change a Spec](../user-guides/change-a-spec.md) |

`vitrify --help` lists the commands, and `vitrify --version` prints the installed version.

## Conventions

These rules apply to every command.

- **Headless:** every verb but `vitrify probe` is non-interactive — `probe` asks on stderr before it changes `~/.vitrify/config.toml`, and `-y` answers yes — and none prints color, a pager or animation, whatever its streams are connected to. stdout is data — JSON with `--json` on read verbs, one JSON object on state-changing success — and stderr is diagnostics, including the `next:` lines that name the command to run next. vitrify never runs a guessed command.
- **Repository root:** `git rev-parse --show-toplevel`, else the current directory.
- **SDD resolution:** `--sdd <root>` takes a bundle directory, its `sdd.md`, or a single-file `*.sdd.md`, relative to where the verb runs — the primary checkout or any worktree of it. Without it, vitrify discovers SDDs in a fixed order — bundles `specs/*/sdd.md`, single files `specs/*.sdd.md`, then a repo-wide search over the files git would commit, so git's ignore rules decide (outside a git repository: every file except under `.git` and `.vitrify`); only files whose front matter has an `sdd` key count. A verb that acts on one SDD exits 4 when none is found — `error: no SDD found in repository → start one: vitrify brief authoring (or vitrify new sdd inside specs/)` — or when several are found and `--sdd` is absent, listing the candidates as `--sdd` values (`--sdd specs/alpha`, …).
- **Every SDD at once:** `lint`, `check`, `audit`, `change apply`, `journal replay`, `ledger regen`, `ledger progress`, `pin` and `questions` take `--all` in place of `--sdd`: the verb runs over every SDD discovery finds, in that order, and each line it prints begins with the SDD's slug (`calc: lint clean …`); `--json` output becomes one array, or carries an `sdd` field per row. One SDD's findings — or one SDD that cannot be read, which is reported under its name on stderr — do not stop the rest; the exit code is the worst of theirs.
- **Schema version:** every SDD declares `schema_version`, and this vitrify reads `2.0` and `2.1`; one that declares another version, or none, is refused with exit 4, naming both, and is still listed by `locate`. See [Versioning](../reference/formats.md#10-versioning).
- **Who commits:** `claim`, `renew`, `release` and `run` publish their own mainline commits. `trace record`, `journal append`, `ledger regen`, `audit`'s fixing flags, `pin sync` and `change apply` only edit the working tree — commit the result yourself.
- **The machine:** `vitrify init` sets `~/.vitrify/` (or `$VITRIFY_HOME`) up once per machine, and is the only command that creates it. Every other command reads `~/.vitrify/config.toml`, and the repository's `.vitrify/config.toml` when there is one, before it does anything else — an error in either stops it, exit 2 — and exits 4 until `~/.vitrify/config.toml` exists: `error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init`.
- **Content:** briefs, schemas and templates are this binary's version. `brief`, `new` and `schema` read `~/.vitrify/` disk-first, restoring a deleted file and refreshing an unmodified one whenever this binary's copy differs — see [Override Embedded Content](configuration.md#override-embedded-content).

## Exit Codes

Use this table to read the exit codes every command shares.

| Code | Meaning |
| --- | --- |
| 0 | asserted state holds (recorded, clean, listed, published) |
| 1 | unexpected error (git failure, I/O) |
| 2 | usage error, or a named check failed (lint, audit or `check` findings, an `init` or `probe` `FAIL`, `probe` changes not applied, `run` preflight failure or Blocked, a refused `change apply`) |
| 3 | trace or projection mismatch (`trace verify`, `journal replay`, `ledger regen --check`, and `check` when one of those is the worst) |
| 4 | not found / precondition failed (no SDD, several SDDs and no `--sdd`, an SDD of a schema version this vitrify does not read, no run recorded, no journal, a machine `vitrify init` has not set up) |
| 5 | lost publish race after retries |
| 6 | lease already expired (renew refused — claim again) |

A `vitrify run` that is interrupted cleans up and then ends by the signal it received, as any interrupted program does — there is no exit code for it.

## User Commands

What you run to set up the machine, and what you — or the coding agent working for you — run to author and change the spec, start the build, and inspect its state. A kickoff session runs `lint`, `locate --units`, `audit`, `journal replay` and `ledger progress`, and the integrator runs `audit --skip-rotten` after its gate.

### `vitrify init`

```text
vitrify init [--json]
```

Sets this machine up, once — the only command that creates `~/.vitrify/`; run it in your own terminal. Refuses with exit 2 when `~/.vitrify/config.toml` exists: from then on the file is yours, and [`vitrify probe`](#vitrify-probe) brings its probe record up to date. In order:

1. **Configuration** — writes `~/.vitrify/config.toml` with every setting at its default and the shipped [role profiles](harnesses.md#configure-role-profiles), from the binary's [`crates/vitrify-core/assets/config.toml`](../../crates/vitrify-core/assets/config.toml).
2. **git** — `git --version`; missing git is a `FAIL`.
3. **Harness CLIs** — for each of `claude`, `codex` and `opencode`, tries its one streaming headless spelling — `claude -p --output-format stream-json --verbose`, `codex exec --json`, `opencode run --format json` — and records it when it exits 0 with non-empty stdout. A CLI not on PATH is `untested`, never an error; a CLI that is present but does not accept its spelling is a `FAIL` that names the spelling it needs. Each probe is a real one-shot model call, run in its own process group and killed after 60 s (`VITRIFY_PROBE_TIMEOUT_SECS`). See [Harness CLIs](harnesses.md#probe-harnesses-with-init-and-probe).
4. **Content** — materializes the briefs, schemas and templates under `~/.vitrify/`.
5. **Roles** — for each harness with a recorded spelling, one line per role (`worker`, `retry` when config.toml names one, `escalation`) giving the model, effort and extra args `run` will pass. A role that adds flags gets its own probe — the recorded spelling plus those flags — and a role probe that fails is a `FAIL`.
6. **Record** — writes what it found to the `[probe]` tables of `config.toml`, without asking: the file is new.

Output is one `status: text` line per check (`ok`, `FAIL`, `untested`, `note`), ending `ok: recorded the probe in ~/.vitrify/config.toml`; `--json` emits `{green, checks, harnesses}`. Exit 0 when nothing failed, 2 on any `FAIL`.

### `vitrify probe`

```text
vitrify probe [-y | --assume-yes] [--json]
```

Probes the machine again — git, each harness CLI and each role, as [`vitrify init`](#vitrify-init) does — and brings the `[probe]` tables of `~/.vitrify/config.toml` up to date; it changes nothing else in the file. Run it after installing or upgrading vitrify or a harness CLI, or after editing a role profile. Refuses with exit 4 when `vitrify init` has not set the machine up.

It prints its check lines, then one `change: <key>: <recorded> → <found>` line per difference — `change: probe.claude.version: claude 2.1.318 → claude 2.1.319` — and asks on stderr before it writes:

```text
apply 1 change to ~/.vitrify/config.toml? [y/N]
```

`y` or `yes` applies the changes and prints `ok: updated ~/.vitrify/config.toml (1 change)`; any other answer, or none — stdin closed, as under an agent — changes nothing and prints `note: ~/.vitrify/config.toml: nothing changed`. `-y` (`--assume-yes`) applies without asking. With nothing to change it asks nothing and prints `ok: ~/.vitrify/config.toml: no changes`. It also reports the home's content: overridden files as `note` lines, and those forked from another shipped version as stale.

`--json` emits `{green, checks, harnesses, changes: [{key, from, to}], applied}`. Exit 0 when the record is up to date and nothing failed; 2 on any `FAIL`, when the changes were not applied, or when `config.toml` does not parse; 4 when the machine is not set up.

### `vitrify new`

```text
vitrify new tdd [NAME]
vitrify new sdd [--bundle | --single] [NAME]
```

Materializes an instance template from `~/.vitrify/templates/` into the **current directory**, refusing to overwrite an existing target (exit 2). Prints the created path on stdout, and on stderr the next step: `next: fill <path> per vitrify schema show, then run vitrify lint --sdd <path>` for an SDD, `next: fill <path> per vitrify schema show --tdd` for a TDD. A materialized SDD declares `schema_version: 2.1`.

| Form | Creates | Default `NAME` |
| --- | --- | --- |
| `new tdd NAME` | `NAME.md` | `tdd` |
| `new sdd NAME` (bundle, the default) | `NAME/` with `sdd.md`, `units/U-001-foundation.md`, `contracts/C-001-example.md`, `changelog.md`, `decisions.md` | `scope` |
| `new sdd --single NAME` | `NAME.sdd.md` | `scope` |

Run it inside `specs/` for SDDs — discovery looks there first.

### `vitrify schema show`

```text
vitrify schema show [--tdd]
```

Prints the normative SDD schema, or with `--tdd` the TDD schema — the documents every instance is written to. Reads `~/.vitrify/schemas/`, so an override on disk wins.

### `vitrify lint`

```text
vitrify lint [--sdd <root> | --all] [--built] [--json]
```

Mechanical authoring checks; `lint` must exit 0 before a Draft SDD goes Active. L1–L8 hold the SDD to itself; L9–L12 hold it to what it names outside itself — other SDDs' contracts, its changelog, its TDD, the units of other SDDs it waits on. Rationale: [Design Rationale](../about/design-rationale.md#audit-and-lint).

| Check | Finding |
| --- | --- |
| L1 | a `depends_on` entry is neither a unit of the SDD nor a pin of it (schema 2.1): an unknown id, an internal contract (name the unit instead), a transcription (it has no authority), or an external contract with no `Pin:` line; a pin in `depends_on` the unit does not also list in `consumes`; any pin in `depends_on` of a schema 2.0 SDD; or the SDD's own dependency graph has a cycle (printed) |
| L2 | two units' `owns` globs can match a common path, or a file under `scope_roots` that git would commit is neither owned nor listed under `unmanaged:` in System Layout & Ownership |
| L3 | a unit depends on another unit in its own `parallel_group` |
| L4 | a unit consumes a contract that does not resolve; a contract names a nonexistent producer or consumer; or a contract's `Consumers:` line is not exactly the units that list it in `consumes` (its producer apart, which may consume its own contract) |
| L5 | a unit's reading set exceeds `lint.readset_budget_bytes` (default 65536) or does not resolve; the unit's own entry is measured without the fields vitrify writes (`status`, the lease fields, the stamps, `trace`) |
| L6 | a requirement `U-###.R<n>` in the unit's text is missing from every Verification row's first column (`R<n>`, `U-###.R<n>`, a range `R1–R4`, or `all`) |
| L7 | a unit is past Ready, or has `claimed_by`, `claim_expires` or a `trace` set — state that must not exist before the build |
| L8 | a Ready unit's text contains `OPEN(Q-###)`; or any unit's `OPEN(Q-###)` marker names a question with no entry in the Decision Log |
| L9 | an [external contract](../reference/formats.md#8-external-contracts) is not `ok`: a pin's hash is not the hash of its authority's body (`stale`); the authority it names cannot be found (`missing-authority`); or the declaration cannot be acted on (`invalid`: a pin that still carries a body, a contract that names another SDD's contract but has no `Pin:` line, an authority with no `Surface` heading, a slug two SDDs share) |
| L10 | `version` is not the version the last change entry goes to, or `updated` not its date; or change entries are out of id order (an SDD without change entries is not checked); or a single-file SDD keeps an entry outside `## Changelog`, where it is not read |
| L11 | `tdd:` names no file; or — where the TDD carries the metadata table of vitrify's template — an Active SDD's `tdd_version` does not name its Revision, or its Status is not `Approved` (an SDD without `tdd:` is not checked) |
| L12 | across SDDs: a pin in `depends_on` whose authority cannot be found, or whose `Producer:` line names no unit or ids that are not units of its SDD; a cycle in the repository's unit graph that runs through pins — across SDDs, or within one SDD through a pin of its own contract — printed as the path that closes it (`api U-001 → core U-002 → api U-001`) under every SDD it passes through; or a unit that only waits behind such a cycle |

Output is `lint clean (L1–L12)`, or one line per finding that names its cause and its remediation — the edit or the command that resolves it:

```text
[L4] C-001: <cause> → <remediation>
```

`--json` emits `{clean, findings: [{check, subject, cause, remediation}]}`. Exit 0 clean, 2 on any finding. A stale pin's remediation is `vitrify pin sync --sdd <root>`.

`--built` lints an SDD that is being built, or has been: every check except L7, whose findings are that SDD's normal state. It is the set `vitrify run` preflights with, so a partly built Active SDD can resume — and L5 cannot trip on a unit vitrify itself has built: stamping changes none of the numbers it measures. `--all` lints every SDD of the repository, each line prefixed with the SDD's slug (`calc: [L4] C-001: <cause> → <remediation>`; `--json`: an array of `{sdd, root, clean, findings}`).

L9 and L12 read other SDDs: they find them by discovery, in the checkout the verb runs in. A transcription — an external contract whose `Producer:` line names no contract, because no SDD owns its source — is not a finding and is not clean either: [`pin list`](#vitrify-pin) shows it as `unchecked`.

### `vitrify check`

```text
vitrify check [--sdd <root> | --all] [--skip-rotten] [--json]
```

Says in one call whether an SDD — or, with `--all`, the whole repository — is consistent. For each SDD it runs the read-only checks vitrify has:

| Source | Check | Runs |
| --- | --- | --- |
| `lint` | [`vitrify lint`](#vitrify-lint) | with `--built` once any unit has build state (a status beyond Draft or Ready, a claim, a trace) or a journal exists; otherwise all of L1–L12 |
| `audit` | [`vitrify audit`](#vitrify-audit) | every check; check 4 re-runs Verified units' Verification rows unless `--skip-rotten` |
| `journal` | [`vitrify journal replay`](#vitrify-journal-replay) | where a journal exists |
| `ledger` | [`vitrify ledger regen --check`](#vitrify-ledger-regen) | where the SDD has a ledger or a journal |
| `pin` | [`vitrify pin list`](#vitrify-pin) | the states of its external contracts |

A finding two sources would both report is reported once: a pin's state under `pin`, not again as lint L9, and a journal mismatch under `journal`, not again as audit check 6. Open questions are listed as notes, never as findings: lint L8 is the check that fails on them. `check` writes nothing and runs no §5 command — `vitrify run --dry-run` is the check that does. It is the gate that ends every change: run it after `vitrify change apply`, and before committing.

In a git repository, `check` also lists each SDD's **dirty units**: Verified units that a run would re-check, because a path they are checked against changed in a commit since their last recorded check (their anchor, the commit of their latest `verified` journal event). Those paths are the unit's own files, the files of every unit it depends on — in its SDD, or in another through a pin in `depends_on` — its SDD's unmanaged files, and paths outside every SDD's `scope_roots` (or, where the SDD declares `shared_inputs`, the outside paths those match). Spec files and protocol state (journals, ledgers, decision and change logs, managed unit fields) never make a unit dirty, and neither does a file regenerated into the same bytes. A unit is also dirty when its anchor is a commit git cannot resolve — rewritten history, a shallow clone — or when nothing records a check of it. Dirty units are not findings: they change neither `clean` nor the exit status, and `check` runs nothing to re-check them; `vitrify run` does. Only committed changes count, up to `HEAD`. When an SDD of the repository cannot be read, or the unit graph has a cycle, stderr says `note: dirty units not computed: …` and none are listed.

It prints one line per finding, prefixed with the SDD's slug and the source, each with its remediation; then the notes; then one line per dirty unit, with the changed paths that made it dirty and their owners; then a count:

```text
calc: [lint L4] C-001: <cause> → <remediation>
calc: [audit 5] U-002: <cause> → <remediation>
calc: [journal] U-003: <cause> → <remediation>
calc: [ledger] ledger.md: <cause> → vitrify ledger regen --sdd specs/calc
calc: [pin] C-012: <cause> → vitrify pin sync --sdd specs/calc
calc: note: Q-007 is open (blocks U-002)
calc: dirty U-004: src/core/eval.py (unit calc U-001), tools/gen.sh (outside every scope)
calc: dirty U-005: anchor 3f2a9c1 cannot be resolved (rewritten history, or a shallow clone)
check: 5 findings in 1 of 3 SDDs; 2 units dirty
```

An owner is `unit <slug> U-###`, `unmanaged in <slug>` or `outside every scope`. A clean result ends `check clean: 1 SDD`, or `check clean: 3 SDDs` with `--all`. Under `--all`, an SDD that cannot be read is reported on stderr under its name, and the count ends `; 1 not checked`. Dirty units add `; N units dirty` to the count. `--json` emits one object per SDD — a single object without `--all`, an array with it:

```text
{sdd, root, clean, findings: [{source, check, subject, cause, remediation}], notes: [string],
 dirty: [{unit, anchor, why, paths: [{path, owner}]}]}
```

`source` is `lint`, `audit`, `journal`, `ledger` or `pin`; `check` is the check's own name — `"L4"`, `"5"` — or `null` where the source has none. In `dirty`, `why` is `changed` (with the changed `paths`), `no-anchor` (no recorded check) or `unresolved-anchor`; `anchor` is the recorded commit, `null` when there is none.

**Exit status.** The worst of the component checks, by the rule `--all` uses (an error, 1, wins; otherwise the highest): 0 clean; 2 a lint, audit or pin finding; 3 a journal or ledger mismatch; 4 a refusal (no SDD, several and no `--sdd`, a schema version this vitrify does not read).

### `vitrify pin`

```text
vitrify pin list [--sdd <root> | --all] [--json]
vitrify pin sync [--sdd <root> | --all] [--dry-run] [--json]
```

Works on *external contracts* — contracts whose `Producer:` line begins `external`. Each is one of two kinds; what they are, and how each is written, is in [External Contracts](../reference/formats.md#8-external-contracts):

| Kind | What it is | Held against |
| --- | --- | --- |
| `pin` | how an SDD consumes a contract another SDD owns: the header carries `Pin: <sdd-slug> <C-###> @ <hash12>`, and there is no body | the hash of its authority's body |
| `transcription` | the consumer's own transcription of a source no SDD owns: its `Producer:` line names no contract | nothing — it is listed `unchecked` |

**`list`** prints every external contract of the SDD, what it stands for, and its state:

| State | Means |
| --- | --- |
| `ok` | a pin whose hash is the hash of its authority's body |
| `stale` | the pin's hash is not the hash of its authority's body — `sync` rewrites it |
| `missing-authority` | it names an SDD, or a contract, that is not in the repository |
| `invalid` | what it declares cannot be acted on, and no sync puts it right: a pin that still carries a body; a contract whose `Producer:` names another SDD's contract but that has no `Pin:` line (write the `Pin:` line — `vitrify pin sync` then fixes the hash); an authority with no `Surface` heading; a slug two SDDs share |
| `unchecked` | a transcription: nothing in the repository to hold it against |

```text
beta C-001   ok                 pin of alpha C-001  specs/beta/contracts/C-001-greeting.md
beta C-002   stale              pin of alpha C-002  specs/beta/contracts/C-002-farewell.md — its pin 446a992352a3 is not the hash of alpha C-002's body (now 28c9da8acc97)
beta C-003   missing-authority  pin of alpha C-009  specs/beta/contracts/C-003-ledger.md — its authority cannot be found: alpha has no contract C-009
beta C-004   unchecked          transcription       specs/beta/contracts/C-004-clock.md
gamma C-001  ok                 pin of alpha C-001  specs/gamma.sdd.md
5 external contracts: 2 ok · 1 stale · 1 missing-authority · 1 unchecked
```

`--json` emits `{clean, contracts: [{sdd, contract, file, kind, state, authority, bytes, consumers, detail}], counts}`: `kind` is `pin` or `transcription`; `authority` is `{sdd, contract, file, bytes}`, or `null` for a transcription; `bytes` is the size of the SDD's own entry and `authority.bytes` that of the body it stands for; `consumers` are the units that list it in `consumes`. Paths are relative to the repository root. Exit 0 when nothing is `stale`, `missing-authority` or `invalid` — the states lint L9 reports — and 2 otherwise. An `unchecked` contract never fails the verb, and is never counted as `ok`.

**`sync`** rewrites every `stale` pin's hash in the working tree to the hash of its authority's body. Then it prints, per consumer SDD, the Built and Verified units whose recorded trace the rewrite moved, as the line its change entry needs, and the count:

```text
synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md
beta: - Staleness applied: U-001 → Stale (consume C-002)
1 synced · 1 not synced
```

On stderr it then names the next step, once per consumer SDD:

```text
next: write beta's change entry around its line, then run vitrify change apply --sdd specs/beta
```

What it cannot put right — a missing authority, an invalid declaration — is named on stderr (`not synced: …`) and makes the exit code 2; otherwise 0. `sync` writes contract files and nothing else — no status, no journal event, no version: the change entry is the author's, [`vitrify change apply`](#vitrify-change-apply) applies its marks, and the commit is the author's ([Change a Spec](../user-guides/change-a-spec.md)). `--dry-run` prints the same lines, the staleness lines included, and on stderr `dry run: nothing written` in place of the next steps; it writes nothing. `--json` emits `{synced, staleness, unresolved, dry_run}`.

### `vitrify audit`

```text
vitrify audit [--sdd <root> | --all] [--json] [--skip-rotten] [--fix-leases] [--apply-stale] [--journal]
```

Runs the seven drift checks against the working tree. Rationale: [Design Rationale](../about/design-rationale.md#audit-and-lint).

| # | Check | Finding |
| --- | --- | --- |
| 1 | orphans | a file under `scope_roots` that git would commit is neither owned nor `unmanaged` |
| 2 | ghosts | a Built or Verified unit's `owns` glob matches no file |
| 3 | contract references | a unit consumes a nonexistent contract, or a contract names a nonexistent unit |
| 4 | rotten Verified | a Verified unit's Verification command now exits non-zero (re-run with `sh -c` from the repo root; each distinct command once, `run.gate_jobs` at a time, each with a `TMPDIR` of its own while it runs) |
| 5 | trace mismatch | a Built or Verified unit's recorded trace differs from current content, or it has none |
| 6 | projection consistency | journal replay disagrees with front matter; the journal is out of order; units are past Ready with no journal; a Stale unit has neither a declaring event nor a trace mismatch |
| 7 | lapsed leases | a Claimed unit's lease has expired |

Report-only by default. Each finding prints as `[check N] subject: cause → remediation`, or the run prints `audit clean (7/7 checks)`; `--json` emits `{clean, findings: [{check, subject, cause, remediation}]}`. Exit 0 clean, 2 with findings.

- `--skip-rotten` skips check 4, which executes commands — use it only right after a gate has run them.
- `--fix-leases` returns each lapsed lease to Ready: clears the holder and expiry, keeps `claim_seq`, appends a `release` event with actor `audit`, and regenerates the ledger.
- `--apply-stale` marks every unit with a check-4 or check-5 finding `Stale`, appends a `stale` event carrying the cause, and regenerates the ledger.
- `--journal` appends one `audit` event recording the finding count.

These flags edit the working tree only — commit the result. Findings are reported as found, before any fix, so a fixing run still exits 2; re-run to confirm it is clean. `--all` audits every SDD of the repository (`--json`: an array of `{sdd, root, clean, findings}`), flags and all.

The audit sees one SDD. A pin that is behind its authority is no finding here — the SDD's own files are consistent with each other, which is all its traces can say; that check is lint's (L9), and `pin list`'s.

`--apply-stale` is how a rotten Verified unit is marked, and what kickoff's loop runs. The marks a change entry calls for are [`vitrify change apply`](#vitrify-change-apply)'s.

### `vitrify change apply`

```text
vitrify change apply [--sdd <root> | --all] [--dry-run] [--commit <sha>] [--json]
```

Does a change entry's bookkeeping in one step. The edit, the entry and its version bump are the author's ([Change a Spec](../user-guides/change-a-spec.md)); this verb records what the entry stales. For each Active SDD whose last change entry has no `change` event in its journal yet:

1. It reads the entry's id and version transition, as lint L10 does. It refuses (exit 2) when the front matter's `version` or `updated` does not match the entry; when one of the SDD's pins is stale, since the sync would move its traces again: `error: calc: C-012's pin is stale → vitrify pin sync --sdd specs/calc first`; and when a single-file SDD keeps an entry outside its `## Changelog` section, where no entry is read — a `## CH-###` heading ends that section.
2. It takes the units whose recorded trace no longer matches — [audit](#vitrify-audit) check 5.
3. **PATCH:** it re-records those traces — the editorial re-stamp: the trace map is rewritten, status and stamps are kept — and appends `change --entry CH-###`.
4. **MINOR or MAJOR:** it appends `change --entry CH-### --stale <units>`, then marks each unit Stale with its own `stale` event and cause, as `audit --apply-stale` does.
5. It regenerates the ledger.

Use this table to see what it does with each SDD.

| The SDD | Result |
| --- | --- |
| Active, last entry not yet recorded | applied as above |
| Active, no journal yet (its activation entry) | gets its `change` event, which creates the journal |
| Active, last entry already recorded | skipped, so running the verb again is safe |
| Active, no change entry | nothing to apply |
| Active, an entry under which no unit moved | gets its plain `change` event; nothing is marked or re-recorded |
| Active, units moved but the entry's heading gives no `<from> → <to>` | refused (exit 2): the bump decides what to do with them |
| Active, a single file with an entry outside `## Changelog` | refused (exit 2): the entry would not be read — write it as `### CH-###` under `## Changelog` |
| Draft or Deprecated | nothing to apply (`action` is `draft`): it has no build to keep in step |

It prints one block per SDD, every line prefixed with the SDD's slug, each applied entry followed by the `- Staleness applied:` line the entry needs:

```text
calc: CH-004 1.2.0 → 1.3.0 (minor): U-002, U-004 marked Stale
calc: - Staleness applied: U-002, U-004 → Stale
```

```text
calc: CH-005 1.3.0 → 1.3.1 (patch): traces of U-001 re-recorded
calc: - Staleness applied: —
```

Where there is nothing to do, the line says why: `calc: CH-005 already recorded — nothing to apply`, `calc: Draft — nothing to apply`, `calc: no change entry — nothing to apply`. After an applied entry, stderr names the next step:

```text
next: copy the Staleness applied line into CH-004 (specs/calc/changelog.md), then run vitrify check
```

`--dry-run` prints the same lines, writes nothing, and ends with `dry run: nothing written` on stderr. `--commit` names the code commit recorded on the events it appends. The verb edits the working tree only — the commit is the author's. It changes no format, event kind or journal field: it composes what [`trace record`](#vitrify-trace), [`journal append`](#vitrify-journal-append), `audit --apply-stale` and [`ledger regen`](#vitrify-ledger-regen) do.

`--json` emits one object per SDD — an array with `--all`:

```text
{sdd, root, entry, from, to, bump, action, restamped, stale, staleness_line, dry_run}
```

`bump` is `patch`, `minor`, `major` or `null`; `action` is `applied`, `already-recorded`, `draft` or `no-entry`. A refused SDD has no object: its `error:` lines are on stderr.

**Exit status.** 0 when an entry was applied or there was nothing to do; 2 when an SDD is refused; 4 when no SDD is found, or several and no `--sdd`; 1 for an error.

### `vitrify journal replay`

```text
vitrify journal replay [--sdd <root> | --all] [--json]
```

Checks the journal — the append-only event log, `journal.ndjson` in a bundle or the fenced block under `## Journal` in a single file — against the units' front matter. It rebuilds each unit's expected state from the events (`claim` → Claimed with its `claim_seq`, `release` → Ready, `built`, `verified`, `stale` and `retired` → that status, `change` → Stale for each listed unit; `renew` and `audit` change nothing), then compares every unit's status and `claim_seq`; units with no events must be Draft or Ready. It also flags non-increasing `seq` values and unknown events.

Output is `journal consistent (N events)` (exit 0), or one line per problem (exit 3): `order: …` for ordering findings, `mismatch U.field: journal projects …, front matter says …` for state. No journal, or an empty one, exits 4. `--json` emits `{events, consistent, order_findings, mismatches, projection}`, where `projection` maps each unit to the state the events project.

`--all` replays every SDD of the repository, each line prefixed with the SDD's slug (`calc: journal consistent (12 events)`). An SDD with no journal prints `calc: no journal yet` and does not fail `--all`; the exit code is the worst of the rest. With `--json` it emits an array of the same objects, each led by `sdd` and `root`; an SDD with no journal has `events: 0`, `consistent: null` and empty lists. Events are written by [`journal append`](#vitrify-journal-append); the format is in [Data Model and Formats Specification](../reference/formats.md#6-journal).

### `vitrify locate`

```text
vitrify locate [--json]
vitrify locate --units [--sdd <root>] [--json]
vitrify locate --graph [--json]
```

Without `--units`, lists every SDD in the repository in discovery order — root, shape, slug, version, status and unit count (`--json` adds `title` and `scope_roots`). Exits 4 when none is found.

With `--units`, lists one SDD's units — status, parallel group, `claim_seq` and the computed **claimable** flag. A unit is claimable when every unit it depends on is Built or Verified — a unit of its own SDD, or, through a pin in `depends_on`, every unit on the `Producer:` line of the pin's authority in another SDD — and the unit is Ready, Stale, or Claimed with a lapsed lease. `--json` adds each dependency's status (for a pin: `Verified` when every unit behind it is, `Built` when each is Built or Verified, otherwise the status of the first that is not), the holder, the lease expiry and `lease_live`.

With `--graph`, prints the SDD graph the units' edges imply: for each SDD, sorted by slug, each SDD it depends on — `api → core  via C-001, C-002` — with the pins and the unit edges behind it (`  api U-001 → core U-001  via C-001`), or `(depends on no other SDD)`. Pins may run both ways between two SDDs; only the unit graph must be acyclic, and when it is not, each cycle follows as `cycle: <path that closes it>`, then each unit that waits behind one as `waiting behind a cycle: <unit>`. `--json` emits `{sdds, sdd_edges: [{from, to, pins, units}], unit_edges: [{from, to, via}], order, cycles, waiting}`, each unit as `{sdd, unit}`; `order` is the unit graph's build order (empty with a cycle). An SDD whose schema version this vitrify does not read is left out, named on stderr.

### `vitrify run`

```text
vitrify run [--dry-run] [--cap N] [--gate-jobs N] [--wait] [--recheck] [--sdd <root> [--upstream]] [--mainline BRANCH]
```

The deterministic orchestrator: builds until Complete or Blocked. In a repository of several SDDs, a plain `vitrify run` is a **project run**: every Active SDD is a lane, and one scheduler builds them all (see **Project runs** below). `--sdd` builds one SDD as a single lane. What a run does at each stage — preflight, recovery, dispatch, integration, retries, spec changes mid-run, interrupts, and termination — is described in [Runs](../about/concepts/runs.md), and the procedure is [Run a Build](../user-guides/run-a-build.md). Use this table to read the options.

| Option | Behavior |
| --- | --- |
| `--dry-run` | Run the preflight, print the job header (without the `Watch:` line: a dry run records no job; a project run prints its whole plan, lane by lane) and any claim it would recover, and exit 0. The last line is the command that starts the run it checked — the flags given, without `--dry-run`: `Start: vitrify run --sdd specs/calc`. A dry run publishes nothing and moves no file: lapsed leases, and a primary checkout that is behind, are reported, not put right. |
| `--cap N` | Override `run.worker_cap`. |
| `--gate-jobs N` | Override `run.gate_jobs`: Verification rows run at once (`0` = one per CPU core, `1` = one at a time). |
| `--wait` | When only blocked units remain, keep running and resume when the spec changes, instead of ending Blocked. |
| `--recheck` | Trust no recorded check: every Verified unit's Verification rows and the §5 Test command run again. It rebuilds nothing, and a built attempt is still adopted — it passes the full gate either way. |
| `--sdd <root>` | The SDD to build, as a single lane. Without it, a repository of one SDD builds that one, and a repository of several makes a project run. |
| `--upstream` | With `--sdd`: build the SDDs it depends on too (through pins, transitively), each a lane of one project run; the SDD named is a lane whatever its status, the ones it depends on only when they are Active. Without `--sdd` it is refused: `preflight failed: --upstream adds the SDDs an SDD depends on, and names none → pass --sdd <root> with it` (exit 2). When the named SDD depends on none, the run is a run of one SDD. |
| `--mainline BRANCH` | Override mainline resolution (see [`claim`](#vitrify-claim-renew-release)). |

**Preflight.** Run refuses to start if any gate fails, printing each as `preflight failed: …` on stderr and exiting 2: the SDD resolves, is Active, has no uncommitted changes under its root, and has a slug no other SDD shares; with an `origin`, the local mainline holds no commit `origin` lacks; `vitrify lint --built` passes; the audit is clean (lapsed leases alone are released and published instead; it runs no Verification row); the §5 Bootstrap, Build and Lint commands exit 0 in the SDD's own integration checkout, made anew at the mainline head; `~/.vitrify/config.toml` records a working harness and its role profiles parse; and the worktree root is usable. Each line ends with its fix — a lint or audit finding with its remediation (`preflight failed: lint [L9] C-002: … → vitrify pin sync --sdd specs/beta`), `→ vitrify probe`, `→ commit or stash the changes under specs/calc`. On a machine `vitrify init` has not set up, `run` exits 4 before any preflight. [Runs](../about/concepts/runs.md#preflight) gives each check and its fix in full. A single-file SDD forces the cap of its lane to 1. In a project run only the project's checks refuse the run; a lane's own failures block that lane (see **Project runs**).

**Re-checks.** A Verified unit's recorded check holds until a path it is checked against changes: its own files, those of every unit it depends on (across SDDs through pins), its SDD's unmanaged files, and paths outside every SDD's scope (or, where the SDD declares `shared_inputs`, the outside paths those match). Only units such a change reached run their rows again, with `--recheck` every Verified unit. In a run of one SDD that happens at the start of the run, in the SDD's own checkout; in a project run, as a batch in the lane's checkout once nothing the units depend on has work pending. It happens again in each integration gate, beside the merged units' rows and the §5 Test command (an SDD without a Test command re-runs every Verified unit's rows at each gate), and in the head check that ends the run, which runs the rows of the units a change reached and the Test command unless its recorded pass still holds. The passes of a start-of-run or batch re-check are recorded on mainline as one commit, `sdd: re-check U-001, U-002 at <sha>` (`sdd: re-check the suite at <sha>` when only the Test command ran), carrying a `verified` event by `recheck` and a fresh stamp per unit and `suite: "pass"` when the Test command passed; the gate records its own the same way in its integration commit. So a run with nothing to build or re-check runs no row at all. A row that fails at the start or in a batch makes its unit rotten; a row that fails in the head check is an audit finding.

**Kept work.** A unit whose last attempt was built but never landed, and whose inputs have not moved since, is adopted: claimed again and sent to the gate with no worker session (`[<slug>] U-002 adopted (built attempt c3) → gate`). Only an attempt a run set aside is adopted — the run that made it as it ended, or the next run recovering a run that died — because only then is its worker known to be gone; an attempt whose lease merely lapsed may still have a worker writing, and goes to a worker of its own. An attempt set aside as `rejected-gate`, `rejected-write-set`, `ownership-conflict`, `blocked` or `released`, or one that changed paths outside the unit's write-set, is not adopted: it would be judged the same way again, and its files go to the next worker as a draft. A unit that depends on another SDD's units, through a pin, is claimed only once they are built; its blocker names them. When a Verified unit's row fails at a gate, it is run again on the gate's base: a row that passes there was broken by the batch, which is rejected; a row that fails there too makes its unit **rotten** — reported with the commit that broke it, as a culprit search finds it, never rebuilt — and the units that depend on it wait while the rest of the SDD keeps building. The search probes the commits mainline held since the unit's last recorded check, in a probe checkout of the lane, and ends in a culprit (`[api] U-002 row 2 broke at a1b2c3d (core U-004 integrated) — culprit found`), a flake (`… is flaky: no commit since its last check broke it — fix the row; the unit is re-checked again`: the unit is released from rotten), or `inconclusive`.

**Output.** stdout carries the job header, one line per event, and exactly one final report — `Complete.` or `Blocked.`; stderr carries worker reports, gate failures, notes, and warnings. A run of one SDD — the only SDD of the repository, or the one `--sdd` names — prints the single-SDD shape: the header (`SDD build job — <sdd> @ <version>  [<job>]`, `Done-condition`, `Mainline`, `Plan`, `Workers`, `Roles`, `Worktrees`, `Parameters`), then its event lines unlabelled, except the adopted, rotten and culprit lines, which name the lane as `[<slug>]`. Before anything is dispatched, the header names the command that watches the run, after its `Parameters:` line:

```text
Watch: vitrify run status --job <job> [--follow] · once it ends, the same command prints the report · procedure: vitrify brief run
```

`<job>` is the run's job id. The final report of a run of one SDD — the status line, the progress line, each blocker (a rotten unit with its culprit), the journal tail; its replay line reads `replay: exit 0 (consistent)` — is kept: [`vitrify run status`](#vitrify-run-status) prints it once the run has ended. A project run prints the shape described under **Project runs**. [Monitor a Run](../user-guides/monitor-a-run.md#read-what-the-run-prints) lists every line. An agent that starts or watches a run follows `vitrify brief run`.

**Files.** `.vitrify/run.json` — a snapshot of the latest run, written when the run starts and at every change, never authoritative; its `report` is the final report's lines, `null` until it is printed — and, per job, `.vitrify/logs/<job>/`: a copy of that record; `project.json`, what the run will do and the job's lanes (one per SDD it builds) and their states; `events.jsonl`, one line per event of the run, appended as it happens, a lane's events naming it (`lane`); `report.txt`, the final report. Each lane's own records are in `.vitrify/logs/<job>/<slug>/`: `gate.json`, what its integrator is doing now; per worker, `worker-<holder>.out`, `.err` and `.prompt.md`; and `escalation-<n>.json`. Unit worktrees live under `run.worktree_dir`, by default `~/.vitrify/worktrees/<repo>-<hash>/`, and so do each SDD's integration checkout `mainline-<slug>` (made anew at the start of every run, and reset with no untracked or ignored file before every use) with its lock, its base checkout `base-<slug>` and its culprit probe checkout `probe-<slug>`. `.vitrify/` is git-ignored and nothing is ever built in it.

**Project runs.** A run is a project run when a plain `vitrify run` finds several SDDs, or `--upstream` selects several. Before anything runs, the project preflight reads every SDD: an SDD at schema 2.0 whose units consume pins refuses the run with exit 4, naming it; an SDD that cannot be read, no Active SDD, L1 and L12 problems over the unit graph — every dependency cycle printed as the path that closes it, and the units that only wait behind one — and shared or empty slugs refuse it with exit 2, as do a missing harness, an unusable worktree root, and a local mainline ahead of origin. Each lane then runs its own preflight: the SDD Active and committed, lint, the audit and the journal replay clean, and its §5 Bootstrap, Build and Lint commands passing in its own checkout. A lane whose preflight fails is **blocked** (`[<slug>] preflight failed: …` on stderr), its units — and every unit that depends on them, in any SDD — wait, and the other lanes go on. An SDD that becomes Active on mainline while a plain project run goes joins it as a lane, after its own preflight (`[<slug>] joins the run: its SDD is Active`).

Each pass of the scheduler orders every lane's ready units by their critical-path priority over the whole repository's unit graph — the number of unfinished units on the longest chain from the unit to the end, or with `run.priority = "history"` the time past sessions took — then by slug and unit id, and dispatches while workers are below the cap — the units one pass dispatches are claimed in one mainline commit, each with its own fencing token and journal event; built attempts are adopted straight to the gate. A lane that loses a publish race to a commit that changed nothing its gate checked or read replays its merges on the new head and publishes without gating again. `run.lane_cap` bounds the lanes that hold checkouts on disk at once: a lane's checkout is made, and its §5 Bootstrap, Build and Lint run, when it is first admitted, the lane with the highest-priority work first; a lane with nothing left to do gives its checkouts back. Each lane's commands wait for the gate pool at its priority. When a lane integrates, the Verified units of other lanes that depend on what changed are re-checked — once nothing they depend on has work pending — as one batch with no worker, recorded as one re-check commit (`sdd: re-check U-001, U-002 at <sha>`). A blocked, set-aside or rotten unit holds up only the units that depend on it.

A lane is in one of six states: `idle` (units remain, but none is claimable, in flight or due a re-check), `build` (a unit is claimable, adoptable or in flight), `recheck` (dirty Verified units are due, or a batch is running), `up to date` (every unit is Verified and none is due a re-check), `complete` (its done-condition holds when the run ends), `blocked` (its preflight failed, or unfinished, set-aside or rotten units remain when the run ends). The header reads:

```text
Project build job — 3 SDDs: 2 to build · 1 up to date  [<job>]
Plan: regenerate 4 units · adopt 1 built attempt · re-check 2 units (core U-002 changed)
Critical path: core U-001 → core U-002 → api U-002 → api U-003 (4 units)
Capacity: workers 4 · gate jobs 8 · lanes unlimited
Watch: vitrify run status --job <job> --follow
```

Each event line names its lane (`[api] dispatch U-002 (c1) → w3`, `[api] U-002 adopted (built attempt c3) → gate`, `[api] re-check U-001`, `[api] re-checked U-001 at a1b2c3d`, `[api] U-007 rotten: row 2 fails on mainline at a1b2c3d — finding the culprit`). The final report is `Complete.` — every lane Complete or Up to date — or `Blocked.`, then one line per SDD, sorted by slug, with its state and why, further blockers indented below as `    <slug> <blocker>`; it has no progress line or journal tail:

```text
Blocked.
  api      Blocked      U-007 rotten: row 2 broke at a1b2c3d (core U-004 integrated) — culprit found
  core     Complete     12/12 Verified · built 3 · §5 Test passing
  docs     Up to date   nothing changed since 9f3c2e1
```

A Complete line reads `<V>/<total> Verified`, then `built <n>` (workers' units and adopted attempts that landed), `re-checked <n>`, and `§5 Test passing` where the SDD has one, ending `, no model calls` when it re-checked and called no model. A lane is Up to date when it is complete and the run built, adopted and re-checked nothing in it; `<commit>` is that of its latest journal event. A blocker is the unit's reason: an open `Q-###`, a failing gate command, its last worker outcome, `waits on <unit> (rotten)`, `waits on api U-007 and core U-002, outside this run (SDDs api, core not selected; --upstream builds them)`, or the dependency chain. `--dry-run` prints the whole plan of the run, lane by lane, and is the same for the same repository state and configuration. In a project run `events.jsonl` also gets a `pass` line each time the slots change. `.vitrify/logs/<job>/project.json` holds `job_id`, `started_at`, `ended_at` (`null` while the run lives), `capacity` (`workers`, `gate_jobs`, `lanes`), each lane's `slug`, `root`, `records` directory, `state` and counts (`units`, `verified`, `regenerate`, `adopt`, `recheck`), the `regenerate`, `adopt` and `recheck` units it set out to do (a re-check's `why` and `changed_units`), and the `critical_path` with `critical_path_ran`, the units of it as they landed; the lanes' states are kept current while the run goes.

**Exit status.** 0 for `Complete.`; 2 for `Blocked.`, a preflight failure, or an `abort` verdict; 1 for an error, whose message is the last thing on stderr; 4 for a project run with an SDD at schema 2.0 whose units consume pins. An interrupted run cleans up and then ends by the signal it received.

### `vitrify run status`

```text
vitrify run status [--json] [--job JOB] [--quiet-after S]
vitrify run status --follow [--json] [--since-start] [--job JOB] [--interval S] [--quiet-after S]
```

Shows what a run is doing, unit by unit. It is read-only — it writes no file, takes no lock, and runs git only with `--no-optional-locks` — so point it at a live run from a second terminal. Everything it shows comes from what vitrify records itself: `run.json`, `gate.json`, `events.jsonl`, the worker logs, process liveness, and each worker worktree's git state and front matter. It never parses a harness's output. [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status) shows the output. Use this table to read the options.

| Option | Behavior |
| --- | --- |
| `--follow` | Keep watching: print the view once, then one line per change, until the run ends. |
| `--json` | One JSON object; with `--follow`, one event per line. |
| `--since-start` | With `--follow` only: first replay what the run recorded since it started. |
| `--job JOB` | The run to show, by job ID. Default: the latest, from `.vitrify/run.json`. |
| `--interval S` | Seconds between looks while following (default 2, at least 1). |
| `--quiet-after S` | Seconds without worker output before a worker is flagged quiet (default 300, at least 1). |

The run is read from its own copy, `.vitrify/logs/<job>/run.json`. A run's state is `alive` (its process lives and its lock file exists), `died` (no recorded end, and its process is gone) or `ended`. The follower follows one job; it never switches to a newer run.

**Lanes.** A run of several SDDs has one lane per SDD, listed in `.vitrify/logs/<job>/project.json`; each lane's records — its gate board, its workers' logs — are in `.vitrify/logs/<job>/<slug>/`. With more than one lane, the view prints, in place of the progress line, a table with a line per lane: `LANE`, `STATE` (`idle`, `build`, `recheck`, `up to date`, `complete` or `blocked`, as the run recorded it), `PROGRESS` (`Verified/total`, then the attempts in flight and the units waiting, when not zero) and `WAITS ON`, which for an idle or blocked lane names the units of other lanes that hold its units back (at most three, then `+N more`). A run of one SDD prints the progress line. Both shapes of `project.json` are read: the lanes with their `slug`, `sdd` and `state` alone, and the project record's, whose lanes carry `root` and their counts; the counts come from the lane's SDD in the primary checkout, and from the record when it cannot be read. A job whose records are directly under `.vitrify/logs/<job>/` is shown as one lane. `--follow` prints a line, labelled `[<slug>]`, when a lane changes state, and labels the lines of each lane's work the same way; with `--json`, the follower's events name their lane in a `lane` field.

**Report.** Once the run has ended with its final report, the view prints it under `Report:` — the status line, the progress line, each blocker and the journal tail, exactly as the run printed them. The same lines are in `.vitrify/logs/<job>/report.txt`. So `vitrify run status` answers both while a run goes and after it ends; `vitrify brief run` says what to do about each blocker and warning.

**Stages.** Each attempt in progress has a stage, derived from its `outcome` (first match wins):

| `outcome` | Condition | Stage |
| --- | --- | --- |
| `running` | the worker's process group is gone | `exited` |
| | the worktree could not be probed | `unknown` (shown as `?`) |
| | the unit's front matter in its worktree says `Built` | `stamped` |
| | some file was changed since the worker started | `editing` |
| | otherwise | `reading` |
| `integrating` | the run is not alive | `integrating` |
| | the unit is in the gate's current batch | `gate` |
| | otherwise | `queued` |
| any other | — | the outcome itself |

Files a retry carried over from an earlier attempt and the worker has not touched count as `carried over`, not as editing.

**Warnings.** Printed as `! <text>` lines, and listed in the JSON `warnings` array: a running worker with an empty log after `--quiet-after`; a worker with no output for `--quiet-after`; a worker that has used 90% of `worker_timeout_minutes`; and, for a run that died, that fact plus a `kill -9 -- -<pid>` line for each worker process group still running.

**JSON.** `--json` prints one object with these fields, in order: `job_id`, `pid`, `sdd`, `harness`, `started_at`, `at` (when the snapshot was taken), `state` (`alive`, `died` or `ended`), `terminated` (`null`, or `{state, at, detail}`), `report` (the final report's lines, or `null` until the run prints one), `progress` (the `Progress:` line, or `null`), `params` (`{cap, gate_jobs, lease_minutes, worker_timeout_minutes}`), `workers`, `gate` (`null`, or the contents of `gate.json`), `warnings` (strings), `logs` (`.vitrify/logs/<job>/`) and `lanes`. Each lane object holds, in order: `slug`, `root` (the SDD's path), `state`, `verified`, `total` (non-Retired units), `in_flight`, `waiting` (units neither Verified nor in flight) and `waits_on` (the `<slug> <unit>` of other lanes' units that hold the lane back, sorted; empty unless the lane is idle or blocked); the counts are `null` when they cannot be read. Each worker object holds, in order: `unit`, `claim_seq`, `holder`, `pid`, `retry`, `worktree`, `started_at`, `ended_at`, `outcome`, `stage`, `alive`, `log_bytes`, `log_quiet_s`, `files_changed`, `insertions`, `deletions`, `carried_over`, `stamped`, `detail`, `error`. Probe fields are `null` for a worker that is not running.

`gate.json` holds `step`, `since` (when the step began), `batch` (the units being integrated), `running` (`[{command, since}]`, oldest first), and `done` and `total` (Verification rows finished and to run; `null` in other steps). `step` is one of `lock`, `sync`, `merge`, `bootstrap`, `build`, `rows`, `lint`, `test`, `test-base` (measuring the Test command before the merge), `publish`, `head-check` (the termination check) and `idle`.

**Events.** `--follow --json` prints one event object per line, the same shape as the lines of `events.jsonl`. Its fields, in order: `at`, `event` (the kind), `unit` (or `null`), `claim_seq` (or `null`), `detail` (human-readable, possibly `""`), the kind's own fields in alphabetical order, and `replay` (`true` for a replayed line; only in `run status` output, never in `events.jsonl`). Use this table to read the kinds. An event a lane writes carries a `lane` field, its slug, among its kind's fields — every kind of the run but `run-started`, `gate`, `waiting`, `pass` and `run-ended`; the follower's `lane` events carry it too. Events written by the follower are seen only while one is watching.

| `event` | Written by | Kind fields | Example `detail` |
| --- | --- | --- | --- |
| `run-started` | run | `cap`, `harness`, `sdd` (the SDDs the run builds, comma-separated) | `specs/calc · claude · cap 3` |
| `dispatched` | run, follower | `holder`, `retry` | `→ w3 (retry)` |
| `adopted` | run | `attempt`, `holder` | `built attempt c3 → gate` |
| `integrating` | run, follower | none | `reported built; waiting for the gate` |
| `gate` | run, follower | `batch`, `step` | `rows · U-002, U-005` |
| `outcome` | run, follower | `duration_s`, `outcome` | ``rejected-gate — failing gate: `pytest -q` `` |
| `stale` | run | none | the cause |
| `back-in-pool` | run | none | `its reading set changed` |
| `recheck` | run | `units` | `re-check U-001, U-002` |
| `rechecked` | run | `recorded` | `re-checked U-001 at a1b2c3d` |
| `rotten` | run | `base`, `row` | `row 2 fails on mainline at a1b2c3d — finding the culprit` |
| `culprit` | run | `flaky` | the culprit search's finding, without the unit: `row 2 broke at a1b2c3d (core U-004 integrated) — culprit found` |
| `pass` | run (project runs) | `cap`, `idle`, `waiting`, `working` | `3 working · 1 idle · 0 waiting` |
| `recovered` | run | `holder` | `its holder … is gone — claim released` |
| `escalation` | run | `verdict` | the judge's reason |
| `waiting` | run | none | the `waiting: …` line |
| `run-ended` | run, follower | `state` | `complete: done-condition met` |
| `snapshot` | follower | `snapshot` (the one-shot object) | `""` |
| `stage` | follower | `from`, `to` | `reading → editing` |
| `lane` | follower | `from`, `lane`, `to` | `idle → build` |
| `work` | follower | `deletions`, `files`, `insertions` | `4 files +120 -3` |
| `quiet` | follower | `quiet_s` | `no output for 5m` |
| `active` | follower | none | `output again` |
| `run-died` | follower | `pid`, `survivors` (process-group ids still alive) | `the run is gone and recorded no end` |

With `--since-start`, the follower first prints the run's `events.jsonl`; a run with no event log replays nothing.

**Exit status.** Without `--follow`: 0 whenever a run is shown, whatever its state; 4 when no run is recorded (`error: no run recorded in this repository → start and watch one: vitrify brief run`) or `--job` names none; 2 for a usage error. With `--follow`, the follower exits with the followed run's status:

| The followed run | Exit |
| --- | --- |
| ended `complete` | 0 |
| ended `blocked` | 2 |
| ended `error` or `interrupted` | 1 |
| died | 1 |
| no run recorded | 4 |

Ctrl-C stops the follower (a shell shows 130); a reader that goes away (`| head`) stops it with exit 0.

### `vitrify ledger progress`

```text
vitrify ledger progress [--sdd <root> | --all]
```

Prints the one-line progress summary, computed from unit front matter: `Progress: <V>/<total> Verified · <n> <Status> · …`, with statuses in the order Verified, Built, Claimed, Ready, Stale, Draft, Retired, zero counts omitted, and Retired excluded from the total. The full ledger is written by [`ledger regen`](#vitrify-ledger-regen). `--all` prints one line per SDD of the repository (`alpha: Progress: 2/2 Verified`).

### `vitrify questions`

```text
vitrify questions [--sdd <root> | --all] [--open] [--json]
```

Lists every `Q-###` of the Decision Log — `decisions.md`, or the `## Decisions & Open Questions` section of a single file — with its status and the units it blocks, then the counts:

```text
Q-001  open      blocks U-003, U-004  TDD §3.2 is silent on a leading `+`; conflicts with C-002. Blocked: U-003.
Q-002  resolved  blocks —             settled by TDD 1.1.
1 open, 1 resolved
```

A question is **open unless its entry marks it resolved**: the word `resolved` in the parenthesis after its id (`- **Q-002** (2026-10-19, resolved 2026-10-24) — …`). An entry with no parenthesis, or one that says nothing of its status, is open — so an entry a run landed is never read as settled. The units a question blocks are those whose entry carries its `OPEN(Q-###)` marker and, while it is open, the unit a run reported it on.

`--open` lists only the open questions; the count line still gives both. `--json` emits `{open, resolved, questions: [{sdd, q, status, blocks, noted, text}]}`. Exit 0 whatever is open: the check that fails is lint L8 — a marker in a Ready unit, or a marker whose question has no entry. An answer lands as a change to the spec, through `vitrify brief change`.

## Agent Commands

The protocol verbs, in the order a unit meets them. A worker's prompt comes from `brief build-unit`; the worker claims the unit (unless the orchestrator already did), loads its reading set with `readset`, builds, and stamps the unit Built with `trace record`. The integrator then stamps it Verified with `trace record` and records the transitions with `journal append` and `ledger regen`. Under `vitrify run`, the orchestrator itself claims, renews leases, generates worker prompts and integrates. You run these by hand mainly to ship an authored baseline ([Ship an Authored Baseline](../user-guides/author-a-spec.md#ship-an-authored-baseline)); they are not run by hand while `vitrify run` builds the SDD. A change entry's re-stamps and marks are [`vitrify change apply`](#vitrify-change-apply)'s.

### `vitrify brief`

```text
vitrify brief
vitrify brief <task> [--sdd <root>]
vitrify brief build-unit --unit U --holder H [--sdd <root>]
```

Prints vitrify's procedures — one task each, in this binary's version — from `~/.vitrify/briefs/<task>.md` (an override on disk wins). With no task it prints the procedure index and exits 0:

```text
vitrify 0.8.0 procedures — print one with: vitrify brief <task>

  onboard     take a repository from nothing to ready-to-build, or name the gaps     session
  authoring   a PRD to an Approved TDD and an Active, lint-clean SDD                 session
  change      change TDDs and SDDs, one or many, and record what each change stales  session
  run         start, watch and read a `vitrify run` build                            session
  kickoff     drive the build loop from a session instead of `vitrify run`           session
  build-unit  build one unit (--unit --holder prints a worker's whole prompt)        workers
  integrate   take one built unit through the gate to Verified                       integrator
```

The last column is who follows the procedure: a coding-agent session working with you, a worker, or the integrator. Use this table to pick a task.

| Task | Use it to |
| --- | --- |
| `onboard` | onboard a repository: take it from nothing to ready-to-build, or name the gaps |
| `authoring` | turn a PRD into an Approved TDD and an Active, lint-clean SDD: routing, the interview, splitting and sizing, the test posture, gated by `lint` |
| `change` | change TDDs and SDDs, one or many — producers before consumers, pins synced between them, one entry per SDD, `vitrify change apply`, then `vitrify check` |
| `run` | start a `vitrify run` build, wait on it without spending turns, and read its result: each blocker and warning, and what to do about it |
| `kickoff` | drive the build loop from a session — the state machine `run` executes |
| `build-unit` | the implementer procedure for one unit |
| `integrate` | the serial integrator procedure: one Built unit to Verified, or rejected with evidence |

Every brief opens with its purpose, a `Use when:` line, a `Not for:` line and the vitrify version, and ends with what to report. Values are filled in at print time:

| Placeholder | Filled with |
| --- | --- |
| `{version}` | the installed vitrify's version |
| `{worktrees}` | the real worktree root of the repository the brief is printed in |
| `{status_check_seconds}` | `run.status_check_seconds` (default 270), in `vitrify brief run`: how often to check `vitrify run status` when the harness cannot notify — see [Configuration](configuration.md#why-270-seconds) |

A harness session finds the brief its task needs from `vitrify --help`; each user guide's Agent Prompts section gives the prompts, as in [Run a Build](../user-guides/run-a-build.md#agent-prompts).

With `--unit` and `--holder`, `build-unit` emits the complete worker prompt — exactly what `vitrify run` hands each worker:

1. the `vitrify-job` block, filled with the unit, holder, the SDD's root relative to the checkout (what every command in the prompt passes as `--sdd`), the configured `lease_minutes`, and branch `sdd/<sdd>/<unit>/c<seq>` (the SDD's slug; the current `claim_seq` when the unit is Claimed, else the seq the next claim will take);
2. the implementer procedure, with the unit's worktree path filled in and — when there is something to say — a "This session" note: what the orchestrator has already done for the worker, and what an earlier attempt at the unit left behind and why it did not land;
3. a "Resolved reading set" appendix inlining every reading (see [`readset`](#vitrify-readset)) — a pinned contract's own entry, then the body it stands for, read from the SDD that owns it.

`--unit` and `--holder` must be given together and only to `build-unit` (exit 2); an unknown unit exits 4.

### `vitrify claim`, `renew`, `release`

```text
vitrify claim   --unit U --holder ID [--lease-minutes N] [--sdd <root>] [--mainline BRANCH]
vitrify renew   --unit U --holder ID [--lease-minutes N] [--sdd <root>] [--mainline BRANCH]
vitrify release --unit U --holder ID [--sdd <root>] [--mainline BRANCH]
```

Each is one atomic mainline transaction (design: [Claim Transactions](../about/architecture.md#claim-transactions)):

1. **Resolve mainline:** `--mainline` > config `mainline` (`VITRIFY_MAINLINE`) > the primary checkout's current branch > `origin/HEAD` > `main` or `master`. Publishing uses `git push` when `origin` exists and `git fetch origin <mainline>` succeeds; otherwise it moves the local branch with a compare-and-swap `git update-ref`.
2. **Build at the head:** check out the mainline head into a throwaway `.vitrify/tx-<pid>-<n>` worktree and re-check preconditions there. The SDD must already be committed to mainline. `--sdd` is a path from where the verb runs — the primary checkout or any worktree of it — and reaches the same SDD from each; a path in no checkout of the repository is refused (exit 4), never edited where it lies.
3. **Commit:** edit the unit's front matter, append the journal event, regenerate the ledger, and commit as `sdd: <op> <unit> (c<seq>) by <holder>`.
4. **Publish:** never forced. A lost race resyncs and retries — 3 attempts in all.
5. **Resync:** if the primary checkout has mainline checked out, its index and files are fast-forwarded to the new commit (`git read-tree -m -u`); uncommitted changes ride along. If one of them, or an untracked file, is in the way, nothing is touched: vitrify warns, names it, prints the command that brings the checkout up to date, and notes in `.vitrify/primary-behind` which commit the files hold — the next transaction or run catches up from there once the way is clear. Until then the checkout is behind its branch and nothing should be committed from it. A checkout that cannot be moved never fails the transaction: the commit is published.

| Op | Precondition | Front-matter edits |
| --- | --- | --- |
| `claim` | the unit is claimable (see [`locate --units`](#vitrify-locate)) | `status: Claimed`, `claimed_by`, `claim_expires` = now + lease, `claim_seq` + 1 |
| `renew` | Claimed by this holder, lease still live | `claim_expires` = now + lease |
| `release` | Claimed by this holder | `status: Ready`, holder and expiry cleared, `claim_seq` kept |

`--lease-minutes` defaults to 60; it does not read `run.lease_minutes`, which only `vitrify run` and the generated worker prompt use. Success prints one JSON line: `{unit, op, holder, claim_seq, claim_expires, commit, mainline}`. Exit 4 when the unit is missing, not claimable, or not held by this holder (the reason is named); 5 when every publish attempt lost the race; 6 when renewing an expired lease — claim again instead.

### `vitrify readset`

```text
vitrify readset --unit U [--sdd <root>] [--json] [--budget [BYTES]]
```

Prints the unit's ordered minimal reading set — everything a worker reads to build it, nothing else:

1. `## Ground Rules` of the core file;
2. `## Technology Decisions` of the core file;
3. the unit's own entry (bundle: its unit file; single: the `### U-###` region);
4. each consumed contract in `consumes` order (bundle: the first sorted `contracts/C-###-*.md`, else `contracts/C-###.md`; single: the `### C-###` region) — and, directly after a contract that [pins](../reference/formats.md#81-pins) another SDD's: that contract's body, as `<its file>  ::  Surface onward`.

Text output is one path per line, with `  ::  <region>` appended for a region of a shared file; `--json` adds the SDD root, shape and `consumes` list. A missing required section, unknown unit or missing contract exits 4 with one `error:` line each — as does a pin that does not resolve or whose hash is not its authority's: its body is handed out against a matching hash or not at all.

`--budget` totals the set's bytes and exits 2 when over the limit: `--budget BYTES` sets the limit, bare `--budget` uses `lint.readset_budget_bytes`. A pinned body counts exactly as a copy of it would. The unit's own entry counts as authored: the front-matter fields vitrify writes — `status`, `claimed_by`, `claim_expires`, `claim_seq`, `built`, `verified`, `trace` — are left out, exactly as the trace hash leaves them out, so the total is the same before a unit is claimed and after it is Verified.

Workers run `readset --unit U --json` to load exactly their inputs, and `brief build-unit` inlines the same set into the worker prompt. Authors use `--budget` to size one unit while writing the SDD; `lint` L5 checks every unit at once.

### `vitrify trace`

```text
vitrify trace record --unit U --stamp built|verified [--commit SHA] [--sdd <root>]
vitrify trace verify --unit U [--sdd <root>] [--json]
```

A trace is how vitrify detects that a unit's inputs changed. Each key holds the first 12 hex characters of a SHA-256 over one input: `unit` (the unit's entry, volatile fields masked), each consumed contract `C-###`, and `global` (Ground Rules + Technology Decisions). The exact recipe is in [Trace Hash Recipe](../reference/formats.md#4-trace-hash-recipe).

**`record`** computes the map, writes it into the unit's `trace:` front matter, and stamps the unit; `--stamp` is required:

- `built` sets `status: Built` and `built: <date> @ SDD <version>, commit <sha>`;
- `verified` sets `status: Verified` and `verified:` the same way, and clears `claimed_by` and `claim_expires`.

`--commit` sets the stamp's SHA (default: short `HEAD`). It prints one JSON object, `{unit, recorded, stamp, file}`, and exits 4 if the unit or a consumed contract is missing. It appends no journal event and does not touch the ledger: pair it with [`journal append`](#vitrify-journal-append) and [`ledger regen`](#vitrify-ledger-regen), then commit. The editorial re-stamp — re-recording the hashes of a unit a PATCH entry moved, keeping its status and stamps — is [`vitrify change apply`](#vitrify-change-apply)'s.

**`verify`** recomputes the map and compares it key by key. It prints `U-###: trace matches` (exit 0) or `U-###: STALE — mismatched keys: …` (exit 3); `--json` lists each mismatch with its recorded and current values. Exit 4 when the unit has no recorded trace. `audit` check 5 runs the same comparison for every Built and Verified unit.

### `vitrify journal append`

```text
vitrify journal append --event KIND [--unit U] [--actor A] [--claim-seq N] [--entry CH-###]
                       [--stale U,U,…] [--cause TEXT] [--findings N] [--commit SHA] [--sdd <root>]
```

Appends one event to the journal — `journal.ndjson` in a bundle, the fenced block under `## Journal` in a single file, created on first use. The event gets `seq` = last + 1, `at` = now (UTC), `sdd` = the SDD's version, plus the fields given. `KIND` is one of `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, `audit`; every kind except `change` and `audit` requires `--unit` (exit 2). `--stale` takes comma-separated unit IDs, for `change` events. Prints the appended line and edits the working tree only. Existing lines are never rewritten; [`journal replay`](#vitrify-journal-replay) checks the result. Format: [Data Model and Formats Specification](../reference/formats.md#6-journal).

### `vitrify ledger regen`

```text
vitrify ledger regen [--sdd <root> | --all] [--check]
```

Rewrites the derived ledger from unit front matter — it is never hand-edited. A bundle's ledger is `ledger.md`; a single file's is its `## State Ledger` section. It holds a header (`# Ledger — <sdd> @ SDD <version> (derived <date>)`), the progress line (see [`ledger progress`](#vitrify-ledger-progress)), and a table of Unit, Status, Group, Built @ and Verified @ (the SDD version in each stamp), and Note (`lease to <expiry>` for Claimed, `awaiting gate` for Built). Prints the progress line, and the written path on stderr; edits the working tree only.

`--check` writes nothing: it derives the ledger afresh and compares it with the one on disk, the header's derived date apart. It prints `ledger current (<path>)` and exits 0, or names the first line that differs — as derived, and as found — and exits 3; a ledger that is not there is a mismatch too. It catches a version bump or a hand edit the ledger did not follow. `--all` does either for every SDD of the repository.

## Related Topics

- [Configuration](configuration.md) — the settings and environment variables these commands read.
- [Monitor a Run](../user-guides/monitor-a-run.md) — reading the output of `run` and the read verbs together.
- [Data Model and Formats Specification](../reference/formats.md) — the formats these commands read and write.
- [Troubleshooting Guide](../resources/troubleshooting.md) — what to do when a command fails.
