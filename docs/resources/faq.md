# Support and FAQs

> Find answers about what vitrify is, how it builds, how it treats tests, data, and harnesses, and where to get help.

Use this page to decide where to start, to check whether vitrify fits your project, and to find where to look when something fails. Each answer is short and links to the page that owns the details.

## Product Positioning

### What is vitrify responsible for?

vitrify owns the mechanics of spec-driven development: every mechanical verb and every procedure of the protocol — claims, traces, the journal, the ledger, lint, and the audit — plus a deterministic orchestrator, `vitrify run`, that drives AI coding agents through your existing harness CLIs. You own the spec and every decision in it. See the [vitrify Primer](../about/primer.md).

### Is vitrify an agent framework?

No. It is a deterministic orchestrator: control flow, scheduling, and state transitions are Rust, and a model is invoked at exactly three enumerated points — workers, escalation, and interactive authoring. It is not a general-purpose agent orchestrator either: splitting and sizing work is plan-time authoring, never decided at dispatch. See [Design Rationale](../about/design-rationale.md#design-positions).

### What does vitrify not do?

It does not write code itself, read your source code, host a service, run an MCP server, or call a model API directly, and it installs nothing into your harness's own folders — every procedure is served on demand by `vitrify brief`. See [Non-Goals](../about/design-rationale.md#non-goals).

### How does vitrify relate to other spec-driven development tools?

vitrify's schema draws on practices documented by other spec-driven development tools and methods — scoped specs, EARS requirements, contract-driven development, and change entries against a spec that is always current truth — and adds a concurrency protocol and mechanical gates around them. [Schema Grounding](../about/design-rationale.md#schema-grounding) records, region by region, which practices each rule draws on, including spec-kit, Kiro, and OpenSpec.

## Technology

### Why is vitrify one Rust binary?

Its runtime dependency set is exactly git: no interpreter runs anywhere at runtime, so nothing else enters a repository, a harness sandbox, or a CI machine. One multi-call binary also means one install, one version, and one `--help` index. See [Requirements and Goals](../about/design-rationale.md#requirements-and-goals).

### Why does vitrify run the git binary instead of using a git library?

The protocol's correctness primitive is the git binary's own compare-and-swap and worktree semantics, so vitrify shells out to it (RA-6). See [Rejected Alternatives](../about/design-rationale.md#rejected-alternatives).

### Why is there no MCP server?

Context cost: in most harnesses every connected MCP server injects its tool schemas into the model's context on every turn, while a CLI costs almost nothing until it is invoked (RA-5).

### Why not call the model API directly?

Workers ride the harness login you already have. A direct-API backend would force API-key billing on users who already pay for a harness (RA-10).

### Which platforms and harnesses are supported?

macOS and Linux, with Windows through WSL 2; Claude Code, Codex, and OpenCode. The [Support Matrix](../reference/support-matrix.md) is the canonical list.

## Getting Started

### What should I read first?

Use this table to pick a starting page.

| Goal | Start with |
| --- | --- |
| go from install to a verified build | [Quick Start](../getting-started/quick-start.md) |
| understand the model in five minutes | [vitrify Primer](../about/primer.md) |
| spec your own project | [Author a Spec](../user-guides/author-a-spec.md) |
| build an Active SDD | [Run a Build](../user-guides/run-a-build.md) |
| look up a command | [Command Reference](../cli/command-reference.md) |

### Do I need tests?

No. vitrify assumes nothing about tests: each unit is verified by whatever its Verification table names — tests written before the build, tests it writes itself, or checks that are not tests at all. See [Test Posture](../about/concepts/test-posture.md).

### Does vitrify care what language my project is in?

No. It never reads source code. It judges units by the exit codes of the commands the SDD names and by which file paths changed, and its file scans follow git's own ignore rules.

## Specs and Units

### How big should a unit be?

Small enough for one worker session: typically 2–8 requirements, with a reading set within 65,536 bytes (lint L5). Check a unit with `vitrify readset --unit U-### --budget`. See [Work Units](../about/concepts/units.md#reading-set).

### What happens when the spec leaves a choice open?

It is a defect. The author decides it, or records a `Q-###` question; a worker that meets a gap stops at a question rather than guess, the run lands it in the Decision Log, and the unit waits until the spec changes. See [Specs: TDD and SDD](../about/concepts/specs.md#decisions-and-questions).

### Can several SDDs share a repository?

Yes. Each needs its own slug and disjoint `scope_roots`; one SDD uses another's contract through a checked pin, and each SDD is built by its own run. See [Work With Several SDDs](../user-guides/multiple-sdds.md).

## Builds

### How many workers run at once?

Four by default (`run.worker_cap`, or `--cap`); a single-file SDD always runs one. Dispatch is continuous, so a unit starts as soon as it is claimable and a slot is free. See [Configuration](../cli/configuration.md).

### What happens when a worker's attempt fails?

Its work is kept on its own branch with the reason, and the unit's next worker starts from it under the `retry` profile. After two failed attempts at one unit, one escalation call returns a verdict the orchestrator enforces; after four, the unit is set aside for the run. See [Runs](../about/concepts/runs.md#retries-and-escalation).

### Can I stop a build and resume it later?

Yes. Ctrl-C stops a run cleanly, and running `vitrify run` again continues from what the repository says; even a run killed outright is recovered by the next one. See [Run a Build](../user-guides/run-a-build.md#stop-and-resume).

### Can I change the spec while a build runs?

Yes. Commit the change to mainline: the run marks stale units and rebuilds them, stops workers building from old text, and returns units parked on a question to the pool. See [Change a Spec](../user-guides/change-a-spec.md#change-the-spec-during-a-run).

### Does vitrify push to my remote?

When `origin` exists and fetching mainline from it succeeds, every claim, release, and integration commit is pushed, never forced; otherwise vitrify moves the local branch with a compare-and-swap. The choice is made once per run and printed in the header's `Remote:` field.

### Will a run overwrite my uncommitted work?

No. Your checkout is fast-forwarded as the run publishes, with local changes carried along, and it is never reset. If a local change is in the way, the checkout is left exactly as it is and the run says so. See [The Primary Checkout Is Behind](troubleshooting.md#the-primary-checkout-is-behind).

### How much does a build cost?

vitrify does not count tokens or cost. Each worker and each escalation is a session on your harness account; whatever the harness reports about a session is in the last events of that worker's log.

## Security and Data

### Does vitrify send my code anywhere?

vitrify itself stores no credentials and opens no network connections; its only network activity is git's own fetch and push, with your credentials. The harness CLI it starts sends each worker's prompt — the unit's reading set — to its model provider, as any session of that harness does.

### What can a worker do on my machine?

Whatever your harness settings allow a headless session to do; vitrify passes no permission flag of its own. Workers edit files in worktrees outside your repository, and the gate runs the SDD's own commands through the shell, by design. See [Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions).

### What do the logs contain?

Each worker's full prompt and session output, and each escalation's evidence and verdict, under `.vitrify/logs/<job>/`. vitrify never deletes them. Protect them as you would any agent transcript.

## Troubleshooting, Releases, and Contributing

### What should I check first when something fails?

Run `vitrify probe`, then `vitrify run --dry-run`, which performs the whole preflight without starting anything; on a machine that is not set up, run `vitrify init` first. The [Troubleshooting Guide](troubleshooting.md) is indexed by symptom.

### Where are the release notes?

In [Release Notes](../about/release-notes.md), which say briefly what changed in each release.

### How do I report a problem?

Report it in the project's repository, `https://github.com/bitshock-src/vitrify`, with the vitrify version (`vitrify --version`), the command, its exit code and output, and — sanitized — the part of `.vitrify/run.json` or the logs that shows the failure.

### How does a coding agent learn vitrify?

Name vitrify to the agent and say what you want, not which command to run — `Use vitrify cli to build the spec.` It runs `vitrify --help`, which routes by task, and then `vitrify brief <task>` for the procedure that fits — `vitrify brief` with no task lists them all. Each user guide opens with the prompts for its tasks. Everything it needs comes from the installed binary, in that binary's version: each command's help, the procedures, and the next step named by each finding and refusal. vitrify installs no agent files, writes no harness instruction file, and needs no line in a repository's `AGENTS.md`.

### Where do contributors start?

[Develop vitrify](../contribute/development.md) covers the toolchain, the conventions, and the two test tiers.

### Which license applies?

vitrify is licensed under the MIT license; the full text is in [`LICENSE`](../../LICENSE).
