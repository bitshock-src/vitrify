# Vitrify

> Hand a whole spec to your AI coding agents, let them build it in parallel, unattended, for hours, and keep revising it as you learn. Deterministic orchestration decides what runs, mechanical gates decide what lands, and each change to the spec rebuilds exactly what it reaches.

- **Smart, deterministic orchestration.** Your coding agents build the spec in parallel, for hours, while code — not a model — tracks dependencies, contracts, and build order, and resolves conflicts before they land.
- **Specs that keep changing.** Revise the TDD and SDD whenever you need to; vitrify rebuilds exactly what each change reaches, even mid-build.
- **Safe to walk away from.** Build state lives in git, so the next run picks up where the last one stopped, and every run ends in one clear report: `Complete.` or `Blocked.`

## Synopsis

```sh
cargo install vitrify-cli --version 0.6.0 --locked   # Rust 1.89 or later; installs `vitrify`
vitrify init                                         # once per machine: settings and harness probe
vitrify run --dry-run                                # preflight and the plan; publishes nothing
vitrify run                                          # builds every Active SDD: Complete. or Blocked.
```

A build needs a spec; [Quick Start](#quick-start) gives the prompts that have your coding agent write one with you.

vitrify runs on macOS and Linux (Windows via WSL 2) and needs only git ≥ 2.30 at runtime, plus a signed-in harness CLI for `vitrify run`. `vitrify init` is the only command that creates `~/.vitrify/`; after installing or upgrading vitrify or a harness CLI, run `vitrify probe`. Workers are headless sessions with nobody to answer a permission prompt: if your harness settings do not already let a session edit files and run commands, grant that to the worker roles ([Grant Worker Permissions](docs/cli/harnesses.md#grant-worker-permissions)). [Installation](docs/getting-started/installation.md) covers building from source, upgrades, and removal.

## What vitrify Gives You

You and your coding agent write a decision-complete spec; `vitrify run` builds it. Each unit goes to its own headless session of Claude Code, Codex, or OpenCode, the sessions run in parallel, and the build carries on without you for as long as it takes. The result is AI-built code you can check, change, resume, and trust by construction rather than by review alone.

#### Deterministic Orchestration

Rust code, not a model, runs the loop: it reads the whole spec — every dependency, contract, and owned file — and makes each scheduling, integration, and recovery decision the same way every time. A model is invoked at exactly three points — to build a unit, to judge one that keeps failing, and to help you author — and never drives the loop.

- **One unit graph, across every SDD.** Units depend on units of their own SDD, or — through a pinned contract listed in `depends_on` — on the units of another SDD that produce it; a contract a unit only reads orders nothing. A unit goes to a worker the moment its own dependencies are built and a slot is free, so nothing waits for a wave or for a whole SDD. A dependency cycle is refused before anything runs, each one printed as the path that closes it.
- **Critical path first.** A free worker slot goes to the ready unit, in any SDD, with the longest chain of unfinished work behind it. Each SDD is a lane with its own integration checkout, so SDDs gate side by side, while one worker pool (`--cap`, 4 by default) and one cap on gate commands (`run.gate_jobs`, one per core) serve every lane. `vitrify run --dry-run` prints the plan and its critical path; `--sdd <root>` builds one SDD, and `--upstream` adds what it depends on.
- **Contracts, not each other's code.** Every worker gets exactly its unit's reading set — its own text, the contracts it consumes, and the rules every unit shares — in a git worktree of its own. Units own disjoint files, so parallel workers never edit the same one, and they code against the same frozen interfaces.
- **Re-checks, not rebuilds.** A code change re-checks, with no model call, only the Verified units that depend on it, in its SDD or another, and a unit rebuilt into the same bytes re-checks nothing. Every check that passes is recorded and trusted until its inputs change, so a run with nothing to do runs nothing.
- **Failures stay local.** A failed attempt keeps its files and is retried with what failed; after two failures a stronger model judges it, and a unit that still cannot land is set aside. A check that starts failing after an upstream change marks its unit rotten and names the commit that broke it, or the check as flaky. Only the units that depend on a failure wait.

[Runs](docs/about/concepts/runs.md) covers each stage of a run, and [Design Rationale](docs/about/design-rationale.md#design-positions) the evidence: deterministic orchestration matches model-driven orchestration on accuracy while improving worst-case robustness and run-to-run consistency.

#### Living TDD and SDD

The spec is never frozen. Revise the design and change the spec before the first build, between builds, or while one is going, and vitrify works out mechanically what each change reaches.

- **Revisions you approve.** The TDD changes by revisions, and every Active SDD pins the current one; a new revision stops the SDDs that pin it until a change entry re-pins them.
- **Edits in place.** Units, contracts, and rules are rewritten where they stand; a change entry (`CH-###`) records what moved and its version bump, and git keeps the old text.
- **Exactly what it reaches.** Every built unit records a hash of everything it read, so `vitrify change apply` marks Stale exactly the units an edit reaches, each with its cause, and a wording fix rebuilds nothing. Commit a change mid-build and the run that is going picks it up.
- **Questions, not guesses.** A worker that finds the spec silent stops at an open question (`Q-###`); your answer lands as a change, and the unit rejoins the build.
- **Several SDDs in lockstep.** A contract other SDDs use is pinned by hash, so its change reaches each of them through their own change entries, and `vitrify check --all` says the repository is consistent.

[Change a Spec](docs/user-guides/change-a-spec.md) walks through a change.

#### Mechanical Gates

Work lands only if its claim is current — a lapsed claim's work never lands, even green — it changed only the files it owns, it was built from the current spec text, and on the merged tree every check the change reaches still passes and the §5 Test command does not go from passing to failing. Every gate starts from a pristine checkout.

#### The Repository Is the Record

Claims, stamps, journal events, and the ledger are commits on your mainline, so build state lives in git, not in a process, and `git log` is the build's timeline. Ctrl-C a run or kill it, then run `vitrify run` again: it releases the claims the last run left, keeps everything that landed, sends work that was built but never landed straight to the gate, and restarts each unfinished unit from its last attempt's files. Every run ends with one report: `Complete.`, with its evidence quoted, or `Blocked.`, with each blocker named.

#### Decision-Complete Specs

A schema, templates, and twelve lint checks turn a design into units that each fit one agent session and leave the implementer no choices — gaps become recorded questions, not guesses.

#### Any Language, Any Test Posture

vitrify never reads source code: it judges units by the exit codes of the commands the SDD names and by which file paths changed. Tests may exist before the build, be written by the units, or not exist at all — chosen per unit as its [test posture](docs/about/concepts/test-posture.md) — and every posture runs through the same gates.

#### Your Harness, Your Login

Workers are headless sessions of Claude Code, Codex, or OpenCode on the account you already have. vitrify stores no credentials and opens no network connections of its own.

## Quick Start

Open a session of Claude Code, Codex, or OpenCode at the root of your repository and send your task; you answer the agent's questions and approve the design.

| To | Send |
| --- | --- |
| start a project | `Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>` |
| add a feature | `Use vitrify cli to add this feature: <its PRD, or its path>` |
| change a spec | `Use vitrify cli to change the spec: <what changes, and why>` |
| build it | `Use vitrify cli to build the spec.` |

Or start the build from your own terminal so it outlives it, and watch it from a second one:

```sh
nohup vitrify run > run.out 2> run.err &
vitrify run status --follow
```

The [Quick Start](docs/getting-started/quick-start.md) walks through each step and how to verify the result. To watch a build before writing a spec, [build the shipped example](docs/getting-started/quick-start.md#build-the-shipped-example), a small calculator with its spec already written.

## For Coding Agents

vitrify is built to be operated by a coding agent working for you. Given a request beginning "Use vitrify cli to", the agent starts at `vitrify --help`, which routes each task — authoring a spec, changing specs, building, watching a run — to its procedure, printed by `vitrify brief <task>`; every command's own `--help` carries what it reads, writes and prints, its exit status, and examples. Findings and refusals name the command to run next. vitrify installs no agent files: everything the agent needs comes from the installed binary, in its version.

## How It Works

A spec moves from design to verified code through five steps, and the fifth repeats for as long as the project lives:

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
4. **Watch** — follow the run with `vitrify run status --follow` ([Monitor a Run](docs/user-guides/monitor-a-run.md)).
5. **Change** — revise the TDD and update the SDD as often as the work needs; `vitrify change apply` stales exactly what each change reaches, and the next run — or the one already going — rebuilds it ([Change a Spec](docs/user-guides/change-a-spec.md)).

> [!NOTE]
> vitrify does not replace your design judgment, your harness and its account, your build tools and tests, or your repository hosting and review. The [vitrify Primer](docs/about/primer.md) explains where each boundary lies.

## Documentation

Use this table to find where to start. The [documentation index](docs/README.md) lists every page.

| Goal | Start with |
| --- | --- |
| Get from install to a verified build | [Quick Start](docs/getting-started/quick-start.md) |
| Install, upgrade, or remove vitrify | [Installation](docs/getting-started/installation.md) |
| Understand the model in five minutes | [vitrify Primer](docs/about/primer.md) |
| Turn a PRD into a TDD and SDD | [Author a Spec](docs/user-guides/author-a-spec.md) |
| Decide how units are tested | [Test Posture](docs/about/concepts/test-posture.md) |
| Build, resume, and unblock | [Run a Build](docs/user-guides/run-a-build.md) |
| Watch a build and read its output | [Monitor a Run](docs/user-guides/monitor-a-run.md) |
| Revise the TDD or update the SDD | [Change a Spec](docs/user-guides/change-a-spec.md) |
| Build a repository of several SDDs | [Work With Several SDDs](docs/user-guides/multiple-sdds.md) |
| Set up Claude Code, Codex, or OpenCode | [Harness CLIs](docs/cli/harnesses.md) |
| Change settings | [Configuration](docs/cli/configuration.md) |
| Look up a command, output, or exit code | [Command Reference](docs/cli/command-reference.md) |
| Fix a problem | [Troubleshooting Guide](docs/resources/troubleshooting.md) |
| Look up a term | [Glossary](docs/resources/glossary.md) |
| See the components and where state lives | [Architecture](docs/about/architecture.md) |
| Understand why it is built this way | [Design Rationale](docs/about/design-rationale.md) |
| Read the file formats | [Data Model and Formats Specification](docs/reference/formats.md) |
| Change vitrify itself | [Develop vitrify](docs/contribute/development.md) |

## License

vitrify is licensed under the MIT license; the full text is in [`LICENSE`](LICENSE).
