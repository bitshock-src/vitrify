# Vitrify

> Hand a whole spec to your AI coding agents, let them build it in parallel, unattended, for hours, and keep revising it as you learn. Deterministic orchestration decides what runs, mechanical gates decide what lands, and each change to the spec rebuilds exactly what it reaches.

- **Smart, deterministic orchestration.** Your coding agents build the spec in parallel, for hours, while code — not a model — tracks dependencies, contracts, and build order, and resolves conflicts before they land.
- **Specs that keep changing.** Revise the TDD and SDD whenever you need to; vitrify rebuilds exactly what each change reaches, even mid-build.
- **Safe to walk away from.** Build state lives in git, so the next run picks up where the last one stopped, and every run ends in one clear report: `Complete.` or `Blocked.`

You and your coding agent write a decision-complete spec; `vitrify run` builds it. Each unit of the spec goes to its own headless session of Claude Code, Codex, or OpenCode; the sessions run in parallel, and the build carries on without you — for hours, if that is what it takes. The loop around them is **Deterministic Orchestration**: Rust code, not a model, tracks the dependencies and contracts between units, works out what can build next, keeps each worker to its own files, gates every merge on the combined tree, and retries or escalates what fails. A procedure a model follows varies from run to run; this one does not.

The spec is never frozen. vitrify is designed for regular TDD revisions and SDD updates, for the life of the project: revise the design, change the spec, and vitrify marks Stale exactly the units each change reaches, with the cause named, so the next run rebuilds those and nothing else. Commit the change mid-build and the run that is going picks it up.

Build state lives in git, not in a process, so a long build is safe to walk away from: stop the run or kill it, and the next `vitrify run` resumes where it left off. When nothing more can be built, it reports one of two outcomes: `Complete.`, with its evidence quoted, or `Blocked.`, with each blocker named.

## What vitrify Gives You

vitrify makes AI-built code something you can check, change, resume, and trust by construction rather than by review alone.

#### Deterministic Orchestration

Deterministic means smart. `vitrify run` reads the whole spec — every dependency, contract, and owned file — and makes each scheduling, integration, and recovery decision in Rust, the same way every time:

- **It tracks dependencies.** Each unit goes to a worker the moment the units it depends on are built and a slot is free, so independent units build side by side and nothing waits for a wave.
- **It works out the build order.** The order comes from the dependency graph, re-derived after every event — not from a script or a model's plan. A unit that lands frees the next ones at once, and a blocked unit holds up only the chain behind it.
- **It knows the contracts.** Every worker gets exactly its unit's reading set — its own text, the contracts it consumes, and the rules every unit shares — so units built in parallel code against the same frozen interfaces. When a contract changes, the run rebuilds exactly the units that consume it, stopping any worker still building from the old text.
- **It resolves conflicts before they land.** Units own disjoint files, so parallel workers never edit the same one. Every merge is gated on the combined tree; a red batch is gated again unit by unit, so the sound work lands and the unit at fault goes back with the reason.
- **It recovers on its own.** A failed attempt keeps its files and is retried with what failed; after two failures a stronger model judges it, and a unit that still cannot land is set aside rather than retried forever.
- **It resumes where it stopped.** Ctrl-C it, kill it, or lose power mid-build, then run `vitrify run` again — no resume flag, no recovery step. It releases any claims the last run left behind, keeps everything that landed, and restarts each unfinished unit from its last attempt's files.

A model is invoked at exactly three points — to build a unit, to judge one that keeps failing, and to help you author — and never drives the loop. [Inside `vitrify run`](#inside-vitrify-run) has the details.

#### Living TDD and SDD

Specs are meant to change, and vitrify is designed for regular TDD revisions and SDD updates — before the first build, between builds, and while one is going:

- **Revise the design as you learn.** The TDD changes by revisions you approve, and every Active SDD pins the current one. A new revision stops the SDDs that pin it from building until a change entry re-pins them, so the design and the spec never drift apart.
- **Edit the spec in place.** The SDD is current truth: units, contracts, and rules are rewritten where they stand, and a change entry (`CH-###`) records what moved and its version bump. git keeps the old text.
- **Each change rebuilds exactly what it reaches.** Every built unit records a hash of everything it read, so `vitrify change apply` marks Stale exactly the units an edit reaches, each with its cause. A wording fix rebuilds nothing.
- **Change it mid-build.** Commit while `vitrify run` is going: the run rebuilds what the change reaches and stops any worker still building from the old text.
- **Gaps come back as questions.** A worker that finds the spec silent stops at an open question (`Q-###`) instead of guessing; your answer lands as a change, and the unit rejoins the build.
- **Several SDDs stay in lockstep.** A contract that other SDDs use is pinned by hash, so its change reaches each of them through their own change entries, and `vitrify check --all` says the whole repository is consistent.

[Change a Spec](docs/user-guides/change-a-spec.md) walks through a change.

#### Mechanical Gates

Work lands only if its claim is current, it changed only the files it owns, it was built from the current spec text, and every Verified unit's checks still pass on the merged tree — without turning the suite from green to red.

#### The Repository Is the Record

Claims, stamps, journal events, and the ledger are commits on your mainline. Stopping, crashing, and resuming lose nothing: a new run re-derives state from the files, and every failed attempt is kept on its branch.

#### Decision-Complete Specs

A schema, templates, and eleven lint checks turn a design into units that each fit one agent session and leave the implementer no choices — gaps become recorded questions, not guesses.

#### Your Harness, Your Login

Workers are headless sessions of Claude Code, Codex, or OpenCode on the account you already have. vitrify stores no credentials, opens no network connections of its own, and has no runtime dependency but git.

## Install

vitrify is published on crates.io as [`vitrify-cli`](https://crates.io/crates/vitrify-cli). Install it with cargo, from Rust 1.89 or later:

```sh
cargo install vitrify-cli --version 0.5.0 --locked    # installs the `vitrify` binary
```

Runtime dependency set: exactly `{git}` (≥ 2.30). Harness headless CLIs are optional and only needed by `vitrify run`. Supported platforms: macOS and Linux (Windows via WSL 2). To build from source, prepare a harness, or upgrade, see [Installation](docs/getting-started/installation.md).

Then set up the machine — once, not per repository:

```sh
vitrify init                # ~/.vitrify/config.toml with every default, then probes git + harness CLIs
```

`vitrify init` is the only command that creates `~/.vitrify/`; every other command refuses until it exists. `vitrify run` uses only the harness commands the probe recorded, so after installing or upgrading vitrify or a harness CLI, run `vitrify probe`: it shows what changed and asks before it updates `config.toml`. Workers are headless sessions, with nobody to answer a permission prompt: if your harness settings do not already let a session edit files and run commands, grant that to the worker roles in `config.toml` ([Grant Worker Permissions](docs/cli/harnesses.md#grant-worker-permissions)).

## Quick Start

Write the spec by hand or with the coding agent you already use, then build it with `vitrify run`. With your agent, open a session of Claude Code, Codex, or OpenCode at the root of your repository and say what you want, beginning "Use vitrify cli to": the agent learns the rest from `vitrify --help`, which routes it to the procedure for the task, so it needs no other setup. You answer its questions and approve the design; it stops at an open question rather than guess. The [Quick Start](docs/getting-started/quick-start.md) walks through each step and how to verify the result.

### Start a New Project

Write the TDD and SDD by hand, or let your coding agent write them with you. By hand, start from the templates `vitrify new` generates: [Write the TDD](docs/user-guides/author-a-spec.md#2-write-the-tdd) and [Create the SDD](docs/user-guides/author-a-spec.md#create-the-sdd) show how. With your agent, write a Product Requirements Document (PRD) — what to build, for whom, and how to tell it works; a few lines will do — and, in a new repository (`git init -b main`), send:

```text
Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>
```

The agent interviews you about what the PRD leaves open, at most five questions at a time, writes the design — the TDD — for you to approve, derives the buildable spec — the SDD, split into units that each own their files — and holds it to vitrify's checks. It ends **Ready to build**, quoting the evidence, or with the gaps that remain and who must close them.

### Enhance an Existing Project

Write the feature's TDD and SDD by hand, or let your coding agent write them with you. By hand, a feature with a scope of its own starts from the templates `vitrify new` generates ([Write the TDD](docs/user-guides/author-a-spec.md#2-write-the-tdd), [Create the SDD](docs/user-guides/author-a-spec.md#create-the-sdd)), and a feature inside an SDD you already have is a TDD revision and a change entry ([Change a Spec](docs/user-guides/change-a-spec.md)). With your agent, send the feature's PRD from a repository that already has code, specs, or both:

```text
Use vitrify cli to add this feature: <its PRD, or its path>
```

The agent routes the feature first: one inside an SDD you already have becomes a revision of its TDD and a change to that SDD, and any other gets a TDD and an SDD of its own beside your code. Either way it interviews you, and you approve the design. A build changes only the files the spec's units own. To change a spec you already have — a fix, or the answer to a question a build is waiting on — send `Use vitrify cli to change the spec: <what changes, and why>`.

### Build It With `vitrify run`

Once the spec is ready, start the build from your own terminal:

```sh
vitrify run --dry-run                      # preflight and the plan; publishes nothing
nohup vitrify run > run.out 2> run.err &   # outlives the terminal; ends Complete or Blocked
```

The run works unattended, for hours if the build needs them, and ends `Complete.`, with its evidence quoted, or `Blocked.`, with each blocker named. Run it again to resume from wherever it stopped.

#### Watch the Build

A running build can be read at any moment from a second terminal, without disturbing it:

```sh
vitrify run status --follow     # each unit's stage, time and changes, then one line per change
```

Each worker's session streams to its own log under `.vitrify/logs/<job>/`, and every claim, merge, and stamp is a commit, so `git log` is the timeline of the build. Once the run has ended, `vitrify run status` prints its final report. [Monitor a Run](docs/user-guides/monitor-a-run.md) covers everything a run lets you see.

#### Run It From Your Agent

To hand the build to your coding agent instead, send:

```text
Use vitrify cli to build the spec.
```

The agent checks the build with `vitrify run --dry-run`, starts it with its harness's background execution — or asks you to start a long build in your own terminal — waits without spending turns, and reports how it ended: `Complete.` with its evidence, or `Blocked.` with what each blocker needs and from whom. For a build you started yourself, send `Use vitrify cli to watch the build and tell me how it ends.`

### Or Build the Shipped Example

To watch a build before you write a spec, build the one this repository ships: [`fixtures/example/`](fixtures/example/README.md) — **calc**, a small command-line calculator with an Approved TDD, an Active SDD bundle (five units, four contracts), and an acceptance suite written before the build, already checked in and `Verified`. It must run from its own repository, so copy it out of a checkout of this one:

```sh
cp -R fixtures/example ~/calc-demo && cd ~/calc-demo
rm -rf reference            # test-side convenience, not a project file
git init -b main && git add -A
git commit -m "calc: authored baseline (SDD 1.0.0)"
vitrify run                 # builds U-002 → U-003 ∥ U-004 → U-005, to Complete
```

Or send the build prompt above from a session in `~/calc-demo`. calc's §5 Commands run its own zero-dependency test suite, so it needs `python3` ≥ 3.10. A successful build ends with its evidence quoted:

```text
Complete.
  Progress: 5/5 Verified
  audit: exit 0 (clean)
  replay: exit 0 (consistent)
  §5 Test: exit 0 (python3 -m unittest discover -s tests -t .)
```

The example's [README](fixtures/example/README.md) is its guided tour: how its up-front acceptance suite locks the spec, and drift detection demonstrated.

## For Coding Agents

vitrify is built to be operated by a coding agent working for you. Say what you want, beginning "Use vitrify cli to", as in `Use vitrify cli to build the spec.`, and the agent starts at `vitrify --help`: the help routes each task — authoring a spec, changing specs, building, watching a run — to its procedure, which `vitrify brief <task>` prints, and every command's own `--help` carries what it reads, writes and prints, its exit status and examples. Findings and refusals name the command to run next. vitrify installs no agent files: everything the agent needs comes from the installed binary, in its version.

## How It Works

vitrify is one Rust binary for the whole spec-driven development (SDD) lifecycle. You write a design (a TDD) and a decision-complete, buildable spec (an SDD) that splits the work into units with exclusive ownership; `vitrify` owns every mechanical verb and every procedure of the protocol around them — leases with fencing tokens, input-trace hashes, a derived ledger, an append-only journal, and mechanical gates. A spec moves from design to verified code through five steps, and the fifth repeats for as long as the project lives:

```text
TDD (why) ──► SDD (what, exactly how) ──► vitrify lint ──► status: Active
                                                               │
          ┌────────────────────────────────────────────────────┘
          ▼
     vitrify run ──► claim unit ──► worker builds it in its own worktree ──► report
       ▲  ▲                                                                   │
       │  └──── re-derive, refill ◄── stamp Verified, publish ◄── gate ◄──────┘
       │
       └──── Stale units ◄── vitrify change apply ◄── TDD revision · SDD change entry
```

1. **Author** — turn your PRD into the TDD, derive the SDD, and decide each unit's test posture ([Author a Spec](docs/user-guides/author-a-spec.md)).
2. **Lint** — hold the plan to the authoring checks, then set it Active ([Gates](docs/about/concepts/gates.md)).
3. **Build** — `vitrify run` builds every unit, in parallel where the plan allows, until it is Complete or Blocked ([Run a Build](docs/user-guides/run-a-build.md)).
4. **Watch** — follow the run unit by unit with `vitrify run status --follow`, and read what the run prints, its record, each worker's log, and the history in git ([Monitor a Run](docs/user-guides/monitor-a-run.md)).
5. **Change** — revise the TDD and update the SDD as often as the work needs, each change recorded as a change entry; `vitrify change apply` stales exactly what it reaches, `vitrify check` says the repository is consistent, and the next run — or the one already going — rebuilds what was staled ([Change a Spec](docs/user-guides/change-a-spec.md)).

> [!NOTE]
> vitrify does not replace your design judgment, your harness and its account, your build tools and tests, or your repository hosting and review. The [vitrify Primer](docs/about/primer.md) explains where each boundary lies.

## Inside `vitrify run`

`vitrify run` is a state machine, written in Rust, over the build state committed on your mainline. It refuses to start until the spec, the repository, and a harness pass preflight; from then on, the control flow is all code:

- **Dispatch.** Each unit is claimed and handed to a worker as soon as its dependencies are built and a slot is free — up to `--cap` workers at once, 4 by default; nothing waits for a wave to finish. Every worker runs in a git worktree of its own, with exactly its unit's reading set — its text and the contracts it consumes — on a lease the run renews and a time budget it enforces.
- **Integration.** Finished work goes through the gate on the merged tree, one gate at a time, and what passes is published to your mainline. A red batch is gated again unit by unit, so the sound work still lands and the failing unit is named.
- **Retries.** An attempt that does not land is kept on its branch; the next attempt starts from its files and is told what failed. After two failures, a stronger model returns a JSON verdict — `retry`, `release`, `block`, or `abort` — that the run enforces, and a unit that still cannot land is set aside rather than retried forever.
- **Recovery.** State is re-derived from mainline after every event, so a spec change committed mid-run marks Stale exactly the units it reaches, and a worker still building from the old text is stopped and its unit started again. On Ctrl-C the run stops its workers, keeps their work on branches, and releases its claims; a run killed outright is cleaned up by the next one.
- **Termination.** The run ends with exactly one report: `Complete.` — every unit Verified, the audit clean, the journal consistent, and the SDD's Test command passing, all quoted — or `Blocked.`, with each unfinished unit's blocker named.

A model is invoked at exactly three points, each with a fixed input and an output the code acts on: a **worker** builds one unit from a prompt holding exactly that unit's reading set, **escalation** judges a unit that keeps failing, and an **authoring** session helps you write the spec, outside the run. Adding a fourth takes a design revision, not code alone. The one controlled study on this axis found that deterministic orchestration achieves accuracy comparable to model-driven orchestration while improving worst-case robustness and reducing variability across runs. [Runs](docs/about/concepts/runs.md) covers each stage in full, and [Design Rationale](docs/about/design-rationale.md#design-positions) the evidence.

## Tests and Languages

vitrify assumes nothing about tests. Tests may exist before `vitrify run` or not; an SDD may verify its units with existing tests, have units write tests as they build, or use no tests at all — chosen per unit, as its [test posture](docs/about/concepts/test-posture.md). Every posture runs through the same fixed gates, and a unit's work may change only the files it owns — tests included — plus its own status stamps. The SDD's §5 Test command never has to pass before a build starts; once it does pass, no merge may break it.

Nor does vitrify care which language a project is written in: it never reads source code. It judges units by the exit codes of the commands the SDD names and by which file paths changed, and its file scans follow git's own ignore rules.

## Key Features

- **Exclusive ownership** so parallel workers never write the same file — and a unit can never rewrite the tests that judge it.
- **Input-trace hashes** so any edit to the spec — a unit's text, a contract, the Ground Rules — marks exactly the units that read it Stale, with the cause named.
- **Change entries** so each TDD revision or SDD update is one recorded step — `vitrify change apply` writes its marks and `vitrify check` gates it — and a run already going picks it up.
- **Leases with fencing tokens** so a worker whose claim lapsed can never land its work, even if it is green.
- **A serial integration gate** so a semantic conflict between parallel units is caught before the conflicting work lands, and the whole suite never goes from green to red.
- **A per-unit test posture** so tests can come first, come with the code, already exist, or not exist at all — through the same gates.
- **A derived ledger and an append-only journal** so progress is one read and history replays.
- **Continuous dispatch** so a unit starts the moment its dependencies are built and a worker slot is free.
- **Several SDDs per repository** with checked pins for the contracts that cross them.
- **Content served by the binary** so briefs, schemas, and templates are never copied into harness folders, and an agent learns vitrify from `vitrify --help` and `vitrify brief`.

The [concept pages](docs/README.md#about-vitrify) explain each feature in depth.

## Choose Your First Path

Use this table to pick where to start.

| Goal | Start with | Why |
| --- | --- | --- |
| Get from install to a verified build | [Quick Start](docs/getting-started/quick-start.md) | the prompts that have your coding agent spec a new project or a feature, and build it |
| See a build work before writing a spec | [Build the Shipped Example](docs/getting-started/quick-start.md#build-the-shipped-example) | builds the calc example to Complete in one sitting |
| Understand the model first | [vitrify Primer](docs/about/primer.md) | five minutes, one build seen from both sides |
| Spec your own project | [Author a Spec](docs/user-guides/author-a-spec.md) | every authoring phase, from a PRD to an Active, lint-clean SDD |
| Build an Active SDD | [Run a Build](docs/user-guides/run-a-build.md) | unattended or interactive, with Blocked and resume handled |
| Keep changing a spec you have built | [Change a Spec](docs/user-guides/change-a-spec.md) | TDD revisions and SDD updates, each rebuilding exactly what it reaches |
| Build from your AI coding assistant | [Run a Build](docs/user-guides/run-a-build.md#agent-prompts) | the prompts that have Claude Code, Codex, or OpenCode operate a run, or drive the build from its own session |
| Look up a command or flag | [Command Reference](docs/cli/command-reference.md) | every verb, output, and exit code |
| Understand why it is built this way | [Design Rationale](docs/about/design-rationale.md) | positions, thresholds, and rejected alternatives, with sources |

## Documentation

Use this table to find the page for a task. The [documentation index](docs/README.md) lists every page.

| Task | Start with |
| --- | --- |
| Install, upgrade, or remove vitrify | [Installation](docs/getting-started/installation.md) |
| Learn the model | [vitrify Primer](docs/about/primer.md) |
| Turn a PRD into a TDD and SDD | [Author a Spec](docs/user-guides/author-a-spec.md) |
| Decide how units are tested | [Test Posture](docs/about/concepts/test-posture.md) |
| Build, resume, and unblock | [Run a Build](docs/user-guides/run-a-build.md) |
| Watch a build and read its output | [Monitor a Run](docs/user-guides/monitor-a-run.md) |
| Revise the TDD or update the SDD | [Change a Spec](docs/user-guides/change-a-spec.md) |
| Use several SDDs in one repository | [Work With Several SDDs](docs/user-guides/multiple-sdds.md) |
| Set up Claude Code, Codex, or OpenCode | [Harness CLIs](docs/cli/harnesses.md) |
| Change settings | [Configuration](docs/cli/configuration.md) |
| Look up a command | [Command Reference](docs/cli/command-reference.md) |
| Fix a problem | [Troubleshooting Guide](docs/resources/troubleshooting.md) |
| Look up a term | [Glossary](docs/resources/glossary.md) |

## Learn More

- **Architecture** — [Architecture](docs/about/architecture.md) — the components, where state lives, and the managed execution pipeline.
- **Design rationale** — [Design Rationale](docs/about/design-rationale.md) — the design positions, goals, schema grounding, thresholds, and rejected alternatives.
- **Formats** — [Data Model and Formats Specification](docs/reference/formats.md) — the trace hash recipe, journal, ledger, and versioning.
- **Sources** — [Sources](docs/reference/sources.md) — the papers, documentation, and web pages behind the design's citations.
- **Contributing** — [Develop vitrify](docs/contribute/development.md) — build conventions, the golden corpus, and the two test tiers.

## License

vitrify is licensed under the MIT license; the full text is in [`LICENSE`](LICENSE).
