# Quick Start

> Install vitrify, have your coding agent spec a new project or a feature with you, and let `vitrify run` build it — agent sessions working in parallel, unattended, for hours, under deterministic orchestration. Then keep revising the spec: each change rebuilds exactly what it reaches.

Use this page to go from installing vitrify to a verified build of your own project. You work through the coding agent you already use: you answer its questions and approve the design, it runs the commands, and vitrify judges the result. Once the spec is ready, you start `vitrify run` from your terminal and the build is hands-off: it builds the spec with sessions of your agent working in parallel, and keeps going without you for as long as the build takes. The spec does not freeze there: vitrify is designed for regular TDD revisions and SDD updates, and rebuilds exactly what each one reaches.

## Deterministic Orchestration

`vitrify run` is a deterministic orchestrator: the loop that drives your agent's sessions is Rust code, not a model, and it follows the same procedure on every run. Deterministic means smart: the run reads the whole spec — every dependency, contract, and owned file — and works out what to build, in what order, and how to land it. That is what makes a build safe to leave running for hours:

- **It tracks dependencies.** Each unit goes to its own headless session of Claude Code, Codex, or OpenCode, on your existing account, the moment the units it depends on are built and a worker slot is free — up to four at once by default, or as many as `--cap` sets. A single-file SDD builds one unit at a time.
- **It works out the build order.** The order comes from the spec's dependency graph, re-derived after every event, not from a script or a model's plan. Nothing waits for a wave to finish: a unit that lands frees the next ones at once, and a blocked unit holds up only the chain behind it.
- **It knows the contracts.** Every worker gets exactly its unit's reading set — its own text, the contracts it consumes, and the rules every unit shares — so units built in parallel code against the same frozen interfaces. When a contract changes, the run rebuilds exactly the units that consume it, stopping any worker still building from the old text.
- **It resolves conflicts before they land.** Units own disjoint files, and every worker builds in a git worktree of its own, outside your checkout, so parallel workers never edit the same file. Work merges only through the integration gate, on the combined tree: its claim still current, only its own files changed, built from the current spec text, and every Verified unit's checks still passing. A red batch is gated again unit by unit, so the sound work lands and the unit at fault goes back with the reason.
- **Unattended for hours.** The run renews each worker's lease while it works, stops any worker that outlasts its time budget, and refills every freed slot until nothing more can be built. Nobody needs to watch it.
- **Code drives the loop; models build.** A model is invoked at exactly three points — a worker builds one unit, an escalation call judges a unit that keeps failing, and an authoring session helps you write the spec — each with a fixed input and an output the code acts on. Scheduling, claims, integration, and retries are code.
- **Failures come back with their reasons.** A failed attempt is kept on its branch, and the next attempt starts from its files and is told what failed. After two failures, a stronger model returns a verdict — `retry`, `release`, `block`, or `abort` — that the run enforces, and a unit that still cannot land is set aside rather than retried forever.
- **Nothing is lost.** Every change of state is a commit on your mainline. A run that is stopped, killed, or loses power resumes from the repository the next time `vitrify run` starts.
- **It resumes where it stopped.** No resume flag and no recovery step: run `vitrify run` again and it releases any claims the last run left behind, keeps everything that landed, and restarts each unfinished unit from its last attempt's files. [Resume the Build](#resume-the-build) shows how.
- **One report at the end.** The run ends `Complete.` — every unit Verified, the audit clean, the journal consistent, and your test command passing, all quoted — or `Blocked.`, with each unfinished unit's blocker named.

The design follows the evidence: in the one controlled study on this axis, deterministic orchestration achieved accuracy comparable to model-driven orchestration while improving worst-case robustness and reducing variability across runs. [Runs](../about/concepts/runs.md) explains each stage of a run, and [Design Rationale](../about/design-rationale.md#design-positions) cites the sources.

## Living TDD and SDD

A spec is never finished, and vitrify does not expect it to be. It is designed for regular TDD revisions and SDD updates — before the first build, between builds, and while one is going — and works out mechanically what each change reaches:

- **The design moves by revisions you approve.** The TDD changes by revisions, and an Active SDD pins the current, Approved one. Revise the TDD and every SDD that pins it refuses to build until a change entry re-pins it, so the design and the buildable spec never drift apart. Your agent drafts the revision with you; approving it stays yours.
- **The spec is current truth.** Units, contracts, Ground Rules, and Technology Decisions are edited in place, and each change entry (`CH-###`) records what moved, its version bump, and what the next build builds or rebuilds. git keeps the old text; nobody replays history to know what to build.
- **Every change rebuilds exactly what it reaches.** Each built unit records a hash of everything it read: its own text, the contracts it consumes, the Ground Rules, and the Technology Decisions. An edit moves exactly the hashes of the units that read it, and `vitrify change apply` marks exactly those units Stale, each with its cause. A wording fix is a PATCH: the hashes are re-recorded and nothing is rebuilt.
- **Change it while it builds.** Commit a change while `vitrify run` is going: the run marks and rebuilds what the change reaches, stops any worker still building from the old text and starts its unit again, and returns a unit that was waiting on an answer to the pool as soon as the answer changes what it reads.
- **Gaps come back as questions.** A worker that finds the spec silent stops at an open question (`Q-###`) rather than guess, and the run records it in the Decision Log. Your answer lands as a change entry, and the unit rejoins the build.
- **Several SDDs stay in lockstep.** A contract another SDD codes against is pinned by hash. `vitrify pin sync` carries a producer's change to every consumer as the staleness its own change entry records, and `vitrify check --all` says the whole repository is consistent.
- **Nothing drifts quietly.** An edit that reaches a built unit without its change entry is a `vitrify check` finding and stops the next build at preflight, so an edit nobody meant never becomes a rebuild. The audit also flags a Verified unit whose checks no longer pass.

Your coding agent lands each change for you — [Change a Spec You Already Have](#change-a-spec-you-already-have) gives the prompts — and [Build State](../about/concepts/build-state.md#input-traces-and-staleness) explains how the hashes decide what goes Stale.

## Goal

Take a project from a Product Requirements Document (PRD) — for a new project, or a feature for one you have — to a build that `vitrify run` reports `Complete.`: every unit built by an AI coding agent, each merged only through the integration gate, with your project's own test command passing. Then change the spec as the project moves, and rebuild only what each change reaches.

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
cargo install vitrify-cli --version 0.5.0 --locked
vitrify --version
```

```text
vitrify 0.5.0
```

To build from source instead, see [Install From Source](installation.md#install-from-source).

## Set Up Your Machine

Set up each machine once. This step belongs to no repository: `vitrify init` creates `~/.vitrify/`, which serves every repository you build on this machine, and no other command creates it.

```sh
vitrify init
```

`init` writes `~/.vitrify/config.toml` with every setting at its default, then probes git and each harness CLI on your `PATH` and records each working headless command in the same file; `vitrify run` uses only what it records. The machine is ready when `init` exits 0 with at least one line of the form `ok: harness <name>: headless = …`; [Verify the Installation](installation.md#verify-the-installation) shows the full output. Each probe is a real one-shot model call on your harness account. Whenever you install or upgrade vitrify or a harness CLI, probe again with `vitrify probe`: it shows what changed and asks before it writes.

Machine setup is yours: the procedures your agent follows never run `vitrify init` or `vitrify probe`. Until `init` has run, every other vitrify command refuses — `error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init` — and on a machine with no harness recorded, a build refuses to start: `preflight failed: ~/.vitrify/config.toml records no working harness → vitrify probe`. The agent reports either as a gap for you.

Workers are headless sessions, so nobody is there to answer a permission prompt. If your harness settings do not already let a session edit files and run commands in `~/.vitrify/worktrees/`, grant that to the `worker` and `retry` roles in `~/.vitrify/config.toml`, as described in [Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions), then run `vitrify probe`. No other configuration is needed: `config.toml` holds every setting at its default, and [Configuration](../cli/configuration.md) explains each one.

## Spec the Work With Your Agent

A build needs an Active SDD — a decision-complete spec, split into units — and the TDD it is derived from. Your coding agent can write both with you, starting from your Product Requirements Document (PRD): what to build, for whom, and how to tell it works, in your words. A PRD can be a file or a few lines in the prompt, however rough; the agent interviews you for what it leaves open.

Open a session of your harness CLI — `claude`, `codex`, or `opencode` — at the root of your project's repository, and send the prompt for your task. Each prompt begins "Use vitrify cli to" and says what you want, not which command to run: the agent reads `vitrify --help`, which routes the task to its procedure, prints the procedure with `vitrify brief <task>`, and follows it. The agent needs no other setup — vitrify installs no agent files, and every procedure comes from the installed binary. Use this table to pick the prompt.

| To | Send | The agent follows |
| --- | --- | --- |
| start a project from a PRD | [the new-project prompt](#start-a-new-project) | `vitrify brief authoring` |
| add a feature to a project you have | [the feature prompt](#enhance-an-existing-project) | `vitrify brief authoring`, then `vitrify brief change` for a feature inside an SDD you have |
| change a spec you already have | [the change prompt](#change-a-spec-you-already-have) | `vitrify brief change` |

Whichever you send, design authority stays with you: the agent asks rather than decides, records what nobody has decided as an open question (`Q-###`) instead of guessing, and leaves approving the design to you. For any other task, `vitrify --help` routes the agent to its procedure.

### Start a New Project

Write the TDD and SDD by hand, or let your coding agent write them with you. By hand, start from the templates `vitrify new` generates: [Write the TDD](../user-guides/author-a-spec.md#2-write-the-tdd) and [Create the SDD](../user-guides/author-a-spec.md#create-the-sdd) show how, and [Author a Spec](../user-guides/author-a-spec.md) takes the spec the rest of the way to Active. Either way, the spec lives in a git repository; create one if you have none yet:

```sh
mkdir my-project && cd my-project
git init -b main
```

For agent assist, send this prompt, with your PRD — or its path — in place of the placeholder:

```text
Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>
```

The agent works through the procedure, stopping wherever it needs you:

1. It reads your PRD against the TDD schema and asks you questions, at most five at a time, until every section of the TDD — the design and its reasons — can be written.
2. It writes the TDD and asks you to approve it.
3. It derives the SDD: the decision-complete, buildable spec, split into units that each own their files. It decides how each unit is verified — its [test posture](../about/concepts/test-posture.md) — and holds the SDD to `vitrify lint` before setting it Active.
4. It commits the spec and runs the same preflight a build runs.

It ends with one of two reports: **Ready to build**, quoting the evidence — `vitrify check` clean, the units the build starts with, and the `Start:` line of `vitrify run --dry-run` — or **Gaps named**, each with the check that failed and whether it needs you or a mechanical fix. [Author a Spec](../user-guides/author-a-spec.md) walks through each phase.

### Enhance an Existing Project

Write the feature's TDD and SDD by hand, or let your coding agent write them with you. By hand, a feature with a scope of its own starts from the templates `vitrify new` generates ([Write the TDD](../user-guides/author-a-spec.md#2-write-the-tdd), [Create the SDD](../user-guides/author-a-spec.md#create-the-sdd)), and a feature inside an SDD you already have is a TDD revision and a change entry ([Change a Spec](../user-guides/change-a-spec.md)). With your agent, open a session at the root of the repository — one that already has code, specs, or both — and send the feature's PRD:

```text
Use vitrify cli to add this feature: <its PRD, or its path>
```

The agent routes the feature before writing anything. A feature inside an SDD you already have becomes a change to that SDD — a TDD revision and a change entry that adds units — and the agent follows the change procedure from there. Any other feature gets a TDD and an SDD of its own, governing only the paths under its `scope_roots`. Either way the agent interviews you about what the PRD leaves open and asks you to approve the TDD, or its revision, and a build changes only the files a unit owns: the rest of your code stays as it is, and tests you already have can verify the new units from outside the SDD, where no unit can change them.

A new SDD ends as a new project does, **Ready to build** or **Gaps named**. A change ends committed, with `vitrify check` clean, and its change entry names what the next build builds or rebuilds.

## Build With vitrify run

`vitrify run` builds an Active SDD unattended, in parallel wherever the plan allows, and ends **Complete** or **Blocked**; [Deterministic Orchestration](#deterministic-orchestration) explains what it does along the way. A build makes real model calls through your harness account and can take hours, so start it from your own terminal. First check that it can start:

```sh
vitrify run --dry-run
```

The dry run runs the same preflight as a build, prints the plan, publishes nothing, and ends with a `Start:` line: the command that starts the run it checked. A check that fails prints a `preflight failed:` line naming its fix. Once it passes, start the build so that it outlives your terminal:

```sh
nohup vitrify run > run.out 2> run.err &
```

To keep it in the foreground instead, run `vitrify run 2>&1 | tee run.log`. Either way, the run's output starts with a job header that ends with a `Watch:` line — the `vitrify run status` command that follows this job from a second terminal — then has one line per event, such as `dispatch U-002 (c1) → w1` when a worker starts and `integrated U-002 (c1)` when its work lands, and ends with the run's report. A run that stops for any reason resumes from where it stopped when you run it again. In a repository with several SDDs, name the SDD to build — `vitrify run --sdd specs/checkout`, for one — and build them one at a time, in dependency order.

### Watch the Build

Everything a build is doing can be read while it runs, from a second terminal, without disturbing it. Start with the run's own view:

```sh
vitrify run status --follow
```

It shows each unit in progress — its stage, its time against its budget, how recently its log grew, and what it has changed so far — and the gate's current step, then prints one line per change until the run ends, and exits with the run's status:

```text
14:25:40  U-003 c1  editing → stamped
14:25:52  U-003 c1  reported built → waiting for the gate
14:26:01  gate      rows · U-002, U-003
14:27:44  U-002 c1  verified  10m
…
14:54:30  run       ended: complete · Progress: 5/5 Verified · exit 0
```

Warnings are marked `!`: a worker that has gone quiet, or one near its budget. Each worker's session streams to its own log under `.vitrify/logs/<job>/`, `vitrify ledger progress` prints how far the build is, and every claim, merge, and stamp is a commit on mainline, so `git log` is the timeline of the build. Once the run has ended, `vitrify run status` prints its final report. [Monitor a Run](../user-guides/monitor-a-run.md) covers everything a run lets you see, while it works and after it ends.

> [!WARNING]
> The run writes each worker's full prompt — including the spec text it read — and everything its session printed to `.vitrify/logs/<job>/`. Protect that directory as you would any agent transcript.

### Resume the Build

Every claim, merge, and Verified stamp a run makes is a commit on **mainline**, the one branch it builds from and publishes to, so the build's state lives in git, not in the process. To resume — after a crash, a Ctrl-C, or a `Blocked.` whose blockers you have fixed — run `vitrify run` again with the same branch checked out: it reads the state back from mainline and carries on where the last run stopped.

Mainline is the branch your checkout is on, unless `--mainline` or the `mainline` setting names another; the `Mainline:` line of `vitrify run --dry-run` shows which branch a run will use. To build on a branch other than `main`, check it out before the first run and resume from it. When `origin` has that branch, each commit is pushed there, never forced; otherwise only your local branch moves. [Command Reference](../cli/command-reference.md#vitrify-claim-renew-release) explains mainline in detail: how it is resolved, how commits reach it, and how your checkout follows them.

### Run It From Your Agent

Your coding agent can operate the build instead, in the session that wrote the spec or a new one. Send this prompt, naming the SDD if the repository has several:

```text
Use vitrify cli to build the spec.
```

The agent finds the run procedure, `vitrify brief run`, and follows it; it never writes code or claims a unit itself. It checks the build with `vitrify run --dry-run`, fixes what is mechanical, and hands design questions back to you; starts the build with its harness's background execution, or asks you to start a long build with the `nohup` line above; waits without spending turns, telling you where the build stands whenever you ask; and reports how the run ended: `Complete.` with its evidence, or `Blocked.` with what each blocker needs and from whom. If you started the run yourself, have the agent watch it:

```text
Use vitrify cli to watch the build and tell me how it ends.
```

To have a session drive the build loop itself instead of `vitrify run`, see [Or Build Interactively](../user-guides/run-a-build.md#3-or-build-interactively).

## Build the Shipped Example

To watch a build before you write a spec of your own, build the one this repository ships: [`fixtures/example/`](../../fixtures/example/README.md) — **calc**, a small command-line calculator with an Approved TDD, an Active SDD bundle of five units and four contracts, and an acceptance suite written before the build, already checked in and `Verified` as its first unit. The build takes the other four units to Complete.

calc's §5 Commands run its own zero-dependency test suite, so it needs `python3` 3.10 or later. It must run from its own repository — every verb resolves paths against the enclosing git repository — so copy it out. Run these commands from the root of a checkout of this repository:

```sh
cp -R fixtures/example ~/calc-demo && cd ~/calc-demo
rm -rf reference
git init -b main && git add -A
git commit -m "calc: authored baseline (SDD 1.0.0)"
```

`reference/` is a test-side convenience — the known-correct implementation that vitrify's own tests use in place of a model — not a project file, so the copy drops it. Then build calc from `~/calc-demo` as above — with `vitrify run`, or by sending your agent the build prompt from a session there. It builds U-002 first, then U-003 and U-004 in parallel, then U-005: `U-002 → U-003 ∥ U-004 → U-005`. The example's [guided tour](../../fixtures/example/README.md) explains how its acceptance suite locks the spec, and demonstrates drift detection: what one edit to a contract marks Stale.

## Verify the Build

The run ends with exactly one report. A successful build prints `Complete.` with its evidence, then the last five journal events, and exits 0. This is calc's:

```text
Complete.
  Progress: 5/5 Verified
  audit: exit 0 (clean)
  replay: exit 0 (consistent)
  §5 Test: exit 0 (python3 -m unittest discover -s tests -t .)
  journal tail:
    …
```

The run keeps this report: `vitrify run status` prints it under `Report:` once the run has ended, and it is in `.vitrify/logs/<job>/report.txt`. If the run ends `Blocked.` instead, it names each unfinished unit's blocker; see [Resolve a Blocked Build](../user-guides/run-a-build.md#resolve-a-blocked-build). An answer to an open question, or a fix to the spec, goes through [the change procedure](#change-a-spec-you-already-have), and running the build again resumes it from where it stopped.

Confirm the result independently. First the ledger:

```sh
vitrify ledger progress
```

```text
Progress: 5/5 Verified
```

Then your project's own tests. For calc, use its strict mode, in which a module that does not exist is a failure rather than a skip:

```sh
SDD_REQUIRE_IMPL=1 python3 -m unittest discover -s tests -t .
```

```text
Ran 42 tests in <time>

OK
```

`<time>` is the elapsed time unittest reports. There are zero skips and zero failures. Before the build, the same command reports `FAILED (errors=4)`, because four acceptance modules await their units: that transition is the proof.

### Inspect the Recorded Activity

Every change of state is a commit on mainline. List the build's history:

```sh
git log --first-parent --format='%h %s' main
```

The subjects show each step: `sdd: claim U-002 (c1) by <holder>`, `sdd: integrate U-002 (c1)`, `sdd: verify U-002 (c1)`, and so on. The journal holds the same history as data, one event per line; a bundle SDD keeps it in `journal.ndjson`, beside its `sdd.md`. Print calc's last event:

```sh
tail -n 1 specs/calc/journal.ndjson
```

Use this table to read an event's fields:

| Field | Meaning |
| --- | --- |
| `seq` | the event's position in the journal; strictly increasing |
| `at` | when it was appended, in UTC |
| `event` | what happened: `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, or `audit` |
| `unit` | the unit it concerns |
| `actor` | the holder or tool that caused it |
| `claim_seq` | the claim — the fencing token — it belongs to |
| `sdd` | the SDD's version at the time |
| `commit` | the commit it refers to |

## Change a Spec You Already Have

To change an Active SDD — correct a unit's text, or change a contract — send your agent:

```text
Use vitrify cli to change the spec: <what changes, and why>
```

To answer a question a build is waiting on, send `Use vitrify cli to answer Q-###: <your answer>`; a new feature inside an SDD you have is a change too, sent as `Use vitrify cli to add this feature: <its PRD, or its path>`. For each, the agent follows the change procedure, `vitrify brief change`. A change that revises the design — a feature, a new decision — starts with its questions: the agent drafts the TDD revision with you, you approve it, and the SDD's change entry re-pins it. Then the agent edits the spec text in place, writes the change entry, records it with `vitrify change apply` — which marks Stale exactly the units whose reading set the edit changed, and none for an editorial fix — checks the repository with `vitrify check`, and commits. The next build rebuilds only the units the change marked Stale, and a build that is going picks the commit up by itself. [Change a Spec](../user-guides/change-a-spec.md) covers change entries and version bumps.

## For More Details

- [Installation](installation.md) — requirements, harness setup, upgrades, and removal.
- [Author a Spec](../user-guides/author-a-spec.md) — every phase the authoring procedure takes a spec through.
- [Run a Build](../user-guides/run-a-build.md) — options, Blocked builds, and stopping and resuming.
- [Runs](../about/concepts/runs.md) — what deterministic orchestration does at each stage of a build, from preflight to the final report.
- [Monitor a Run](../user-guides/monitor-a-run.md) — everything a run lets you see while it works.
- [The calc example's guided tour](../../fixtures/example/README.md) — how its up-front acceptance suite locks the spec, and how its baseline was produced.
