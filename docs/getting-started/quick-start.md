# Quick Start

> Install vitrify, have your coding agent spec a new project or a feature with you, and let `vitrify run` build it — agent sessions working in parallel, unattended, for hours, under deterministic orchestration. Then keep revising the spec: each change rebuilds exactly what it reaches.

Use this page to go from installing vitrify to a verified build of your own project: from a Product Requirements Document (PRD), for a new project or a feature of one you have, to a run that reports `Complete.` — every unit built by an AI coding agent and merged only through the integration gate, with your project's own test command passing. You work through the coding agent you already use: you answer its questions and approve the design, it runs the commands, and vitrify judges the result.

## What a Build Does

`vitrify run` is a deterministic orchestrator: the loop that drives your agent's sessions is Rust code, not a model, so a build is safe to leave running for hours.

- **Units build in parallel, critical path first.** Each unit goes to its own headless session of Claude Code, Codex, or OpenCode the moment the units it depends on are built and a worker slot is free — up to four at once by default, or as many as `--cap` sets. In a repository of several SDDs, the units of every SDD form one graph: a unit can wait on another SDD's units through a pinned contract it lists in `depends_on`, each SDD gates in its own lane, and a free slot goes to the ready unit with the longest chain of work behind it.
- **Work lands only through the gate.** Every worker builds in a git worktree of its own, on files its unit alone owns, from exactly its unit's reading set. Its work merges only if its claim is current, it changed only its own files, it was built from the current spec text, and every check the merge reaches still passes on the combined tree.
- **Each change rebuilds exactly what it reaches.** Every built unit records a hash of everything it read, so a spec change — committed even while a build is going — rebuilds only the units it reaches, and a code change re-checks, with no model call, only the units that depend on it.
- **Failures come back with their reasons, and stay local.** A failed attempt is kept on its branch and retried from its files; after two failures a stronger model judges it. A unit that cannot land holds up only the units that depend on it.
- **Nothing is lost.** Every change of state is a commit on your mainline, so a stopped or killed run resumes from the repository when `vitrify run` starts again. Every run ends with one report: `Complete.`, with its evidence quoted, or `Blocked.`, with each blocker named.

[Runs](../about/concepts/runs.md) explains each stage of a run, and [Build State](../about/concepts/build-state.md) how the hashes decide what a change reaches.

## Prerequisites

You need these tools:

- macOS or Linux (on Windows, inside WSL 2), with a POSIX shell.
- git 2.30 or later.
- Rust 1.89 or later, with cargo, to install vitrify.
- One harness CLI — Claude Code, Codex, or OpenCode — installed and signed in.
- Whatever tools your project's build and tests run.

## Install

Install the version these docs describe from crates.io, where vitrify is published as `vitrify-cli`, and check it:

```sh
cargo install vitrify-cli --version 0.8.0 --locked
vitrify --version
```

```text
vitrify 0.8.0
```

To build from source instead, see [Install From Source](installation.md#install-from-source).

## Set Up Your Machine

Set up each machine once, for every repository you build on it:

```sh
vitrify init
```

`init` writes `~/.vitrify/config.toml` with every setting at its default, then probes git and each harness CLI on your `PATH` — each probe is a real one-shot model call — and records each working headless command; `vitrify run` uses only what it records. The machine is ready when `init` exits 0 with at least one line of the form `ok: harness <name>: headless = …` ([Verify the Installation](installation.md#verify-the-installation) shows the full output). Whenever you install or upgrade vitrify or a harness CLI, run `vitrify probe`: it shows what changed and asks before it writes. Machine setup is yours: the procedures your agent follows never run `init` or `probe`, and report a machine that needs either as a gap for you.

Workers are headless sessions, so nobody is there to answer a permission prompt. If your harness settings do not already let a session edit files and run commands in `~/.vitrify/worktrees/`, grant that to the `worker` and `retry` roles in `~/.vitrify/config.toml` ([Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions)), then run `vitrify probe`. Nothing else needs configuring; [Configuration](../cli/configuration.md) explains each setting.

## Spec the Work With Your Agent

A build needs an Active SDD — a decision-complete spec, split into units — and the TDD it is derived from. Your coding agent writes both with you from your PRD: what to build, for whom, and how to tell it works, in your words; a file or a few lines in the prompt will do, and the agent interviews you for what it leaves open.

Open a session of `claude`, `codex`, or `opencode` at the root of your project's repository and send the prompt for your task. Each begins "Use vitrify cli to" and says what you want, not which command to run: the agent reads `vitrify --help`, which routes the task to its procedure, and follows it. vitrify installs no agent files; every procedure comes from the installed binary.

| To | Send | The agent follows |
| --- | --- | --- |
| start a project from a PRD | [the new-project prompt](#start-a-new-project) | `vitrify brief authoring` |
| add a feature to a project you have | [the feature prompt](#enhance-an-existing-project) | `vitrify brief authoring`, then `vitrify brief change` for a feature inside an SDD you have |
| change a spec you already have | [the change prompt](#change-a-spec-you-already-have) | `vitrify brief change` |

Design authority stays with you: the agent asks rather than decides, records what nobody has decided as an open question (`Q-###`) instead of guessing, and leaves approving the design to you.

### Start a New Project

The spec lives in a git repository; create one if you have none yet:

```sh
mkdir my-project && cd my-project
git init -b main
```

Then send this prompt, with your PRD — or its path — in place of the placeholder:

```text
Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>
```

The agent asks you questions, at most five at a time, until it can write the TDD — the design and its reasons — and asks you to approve it. It then derives the SDD, split into units that each own their files, decides how each unit is verified — its [test posture](../about/concepts/test-posture.md) — holds the SDD to `vitrify lint`, commits the spec, and runs the same preflight a build runs. It ends **Ready to build**, quoting the evidence, or **Gaps named**, each with the check that failed and who must close it.

To write the TDD and SDD by hand instead, start from the templates `vitrify new` generates: [Author a Spec](../user-guides/author-a-spec.md) walks through every phase.

### Enhance an Existing Project

From a repository that already has code, specs, or both, send the feature's PRD:

```text
Use vitrify cli to add this feature: <its PRD, or its path>
```

The agent routes the feature before writing anything. A feature inside an SDD you already have becomes a TDD revision and a change entry that adds units; any other feature gets a TDD and an SDD of its own, governing only the paths under its `scope_roots`. Either way you approve the design, and a build changes only the files the spec's units own: the rest of your code stays as it is, and tests you already have can verify the new units from outside the SDD. A new SDD ends **Ready to build** or **Gaps named**; a change ends committed, with `vitrify check` clean.

## Build With vitrify run

A build makes real model calls through your harness account and can take hours, so start it from your own terminal. First check that it can start:

```sh
vitrify run --dry-run
```

The dry run runs the same preflight as a build, prints the plan, publishes nothing, and ends with a `Start:` line — the command that starts the run it checked — or a `preflight failed:` line naming its fix. Then start the build so that it outlives your terminal:

```sh
nohup vitrify run > run.out 2> run.err &
```

To keep it in the foreground instead, run `vitrify run 2>&1 | tee run.log`. The output starts with a job header whose `Watch:` line is the command that follows this run from a second terminal, then has one line per event — `dispatch U-002 (c1) → w1` when a worker starts, `integrated U-002 (c1)` when its work lands — and ends with the report. In a repository with several SDDs, a plain `vitrify run` builds every Active SDD at once, and `vitrify run --sdd specs/checkout` builds one of them; see [Work With Several SDDs](../user-guides/multiple-sdds.md).

### Watch the Build

Everything a build is doing can be read while it runs, from a second terminal, without disturbing it:

```sh
vitrify run status --follow
```

It shows each unit in progress — its stage, its time against its budget, and what it has changed so far — and the gate's current step, then prints one line per change until the run ends:

```text
14:25:40  U-003 c1  editing → stamped
14:25:52  U-003 c1  reported built → waiting for the gate
14:26:01  gate      rows · U-002, U-003
14:27:44  U-002 c1  verified  10m
…
14:54:30  run       ended: complete · Progress: 5/5 Verified · exit 0
```

Warnings are marked `!`: a worker that has gone quiet, or one near its budget. Each worker's session streams to its own log under `.vitrify/logs/<job>/`, and once the run has ended, `vitrify run status` prints its final report. [Monitor a Run](../user-guides/monitor-a-run.md) covers everything a run lets you see.

> [!WARNING]
> The run writes each worker's full prompt — including the spec text it read — and everything its session printed to `.vitrify/logs/<job>/`. Protect that directory as you would any agent transcript.

### Resume the Build

The build's state lives on **mainline** — the branch your checkout is on, unless `--mainline` or the `mainline` setting names another — not in the process. To resume after a crash, a Ctrl-C, or a `Blocked.` whose blockers you have fixed, run `vitrify run` again with that branch checked out: it reads the state back and carries on where the last run stopped. The `Mainline:` line of `vitrify run --dry-run` shows the branch a run uses; when `origin` has it, each commit is pushed there, never forced. [Command Reference](../cli/command-reference.md#vitrify-claim-renew-release) explains how mainline is resolved and how commits reach it.

### Run It From Your Agent

Your coding agent can operate the build instead, in the session that wrote the spec or a new one:

```text
Use vitrify cli to build the spec.
```

The agent follows the run procedure, `vitrify brief run`: it checks the build with `vitrify run --dry-run`, fixes what is mechanical and hands design questions back to you, starts the build in the background — or asks you to start a long one with the `nohup` line above — waits without spending turns, and reports how it ended. If you started the run yourself, send `Use vitrify cli to watch the build and tell me how it ends.` To have a session drive the build loop itself instead of `vitrify run`, see [Or Build Interactively](../user-guides/run-a-build.md#3-or-build-interactively).

## Build the Shipped Example

To watch a build before you write a spec of your own, build the one this repository ships: [`fixtures/example/`](../../fixtures/example/README.md) — **calc**, a small command-line calculator with an Approved TDD, an Active SDD bundle of five units and four contracts, and an acceptance suite written before the build, already checked in and `Verified` as its first unit. Its §5 Commands run its own zero-dependency test suite, so it needs `python3` 3.10 or later.

It must run from its own repository, so copy it out, from the root of a checkout of this repository, dropping `reference/` — the known-correct implementation vitrify's own tests use in place of a model:

```sh
cp -R fixtures/example ~/calc-demo && cd ~/calc-demo
rm -rf reference
git init -b main && git add -A
git commit -m "calc: authored baseline (SDD 1.0.0)"
```

Then build it from `~/calc-demo` as above, with `vitrify run` or the build prompt. It builds U-002 first, then U-003 and U-004 in parallel, then U-005. The example's [guided tour](../../fixtures/example/README.md) shows how its acceptance suite locks the spec, and what one edit to a contract marks Stale.

## Verify the Build

The run ends with exactly one report. A successful build prints `Complete.` with its evidence and exits 0. This is calc's:

```text
Complete.
  Progress: 5/5 Verified
  audit: exit 0 (clean)
  replay: exit 0 (consistent)
  §5 Test: exit 0 (python3 -m unittest discover -s tests -t .)
  journal tail:
    …
```

`vitrify run status` prints the same report once the run has ended, and it is kept in `.vitrify/logs/<job>/report.txt`. A run that ends `Blocked.` names each unfinished unit's blocker; see [Resolve a Blocked Build](../user-guides/run-a-build.md#resolve-a-blocked-build). Answer a question or fix the spec through [the change prompt](#change-a-spec-you-already-have), then run the build again to resume it.

Confirm the result independently, first with the ledger, then with your project's own tests. For calc, use its strict mode, in which a module that does not exist is a failure rather than a skip:

```sh
vitrify ledger progress
SDD_REQUIRE_IMPL=1 python3 -m unittest discover -s tests -t .
```

```text
Progress: 5/5 Verified
Ran 42 tests in <time>

OK
```

There are zero skips and zero failures. Before the build, the same test command reports `FAILED (errors=4)`, because four acceptance modules await their units: that transition is the proof.

### Inspect the Recorded Activity

Every change of state is a commit on mainline, so the history is the build's timeline:

```sh
git log --first-parent --format='%h %s' main
tail -n 1 specs/calc/journal.ndjson
```

The subjects show each step — `sdd: claim U-002 (c1) by <holder>`, `sdd: integrate U-002 (c1)`, `sdd: verify U-002 (c1)` — and the journal holds the same history as data, one event per line; [Journal](../reference/formats.md#6-journal) describes its fields.

## Change a Spec You Already Have

To change an Active SDD — correct a unit's text, change a contract, or answer a question a build is waiting on — send your agent:

```text
Use vitrify cli to change the spec: <what changes, and why>
```

The agent follows the change procedure, `vitrify brief change`. A change that revises the design starts with its questions: the agent drafts the TDD revision with you and you approve it. The agent then edits the spec in place, writes the change entry, records it with `vitrify change apply` — which marks Stale exactly the units whose reading set the edit changed, and none for an editorial fix — checks the repository with `vitrify check`, and commits. The next build rebuilds only what the change marked Stale, and a build that is going picks the commit up by itself. [Change a Spec](../user-guides/change-a-spec.md) covers change entries and version bumps.

## For More Details

- [Installation](installation.md) — requirements, harness setup, upgrades, and removal.
- [Author a Spec](../user-guides/author-a-spec.md) — every phase the authoring procedure takes a spec through.
- [Run a Build](../user-guides/run-a-build.md) — options, Blocked builds, and stopping and resuming.
- [Work With Several SDDs](../user-guides/multiple-sdds.md) — building a repository of several SDDs in one run.
- [Monitor a Run](../user-guides/monitor-a-run.md) — everything a run lets you see while it works.
- [The calc example's guided tour](../../fixtures/example/README.md) — how its up-front acceptance suite locks the spec, and how its baseline was produced.
