# Run a Build

> Build an Active SDD, or every Active SDD of a repository, to Complete with `vitrify run` or an interactive kickoff, resolve a Blocked result, and stop and resume safely.

Use this guide when an SDD is Active, lint-clean, and committed, and you want it built. In a repository with several SDDs, a plain `vitrify run` builds every Active SDD as one [project run](#build-several-sdds). `vitrify run` is the deterministic orchestrator: it runs unattended until the SDD is **Complete** or **Blocked**, and resumes from repository state whenever you run it again. This guide is the procedure; [Runs](../about/concepts/runs.md) explains how a run behaves at each stage. A coding agent operates a run with the procedure `vitrify brief run` prints: preflight, who starts the run, how to wait, and how to read the result.

## Agent Prompts

Your coding agent can operate a build for you. In a session of Claude Code, Codex, or OpenCode at the repository root, send the prompt for your task, naming the SDD if the repository has several. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| build the spec, or resume a build | `Use vitrify cli to build the spec.` |
| find out why a build stopped | `Use vitrify cli to tell me why the build stopped.` |
| drive the build from the session itself ([step 3](#3-or-build-interactively)) | `Use vitrify cli to build the spec from this session, without vitrify run.` |

For a build, the agent finds the run procedure in `vitrify --help` — `vitrify brief run` — and follows it: it runs the dry run and fixes what is mechanical, starts the run with its harness's background execution or asks you to start a long one in your own terminal, waits without spending turns, and reports `Complete.` with its evidence, or `Blocked.` with what each blocker needs and from whom. For a build that stopped, it reads the report `vitrify run status` keeps and the open questions, and names what each blocker needs. It never writes a unit's code or claims a unit itself; an answer or a fix you give it goes through [Change a Spec](change-a-spec.md#agent-prompts).

## Before You Start

Check each item; `vitrify run` refuses to start if any of the first four fails. In a project run each item holds for every Active SDD.

- The SDD is `status: Active` and committed at the mainline head, with no uncommitted changes under its root.
- `vitrify lint --built` and `vitrify audit` exit 0.
- The SDD's §5 Bootstrap, Build, and Lint commands exit 0 from the repository root.
- `vitrify init`, or a later `vitrify probe`, has recorded at least one working harness in `~/.vitrify/config.toml`.
- Your harness lets a headless session edit files and run commands in the worktree directory — see [Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions).
- Build and test byproducts are in `.gitignore`: the handoff commits every change git does not ignore.

## 1. Check the Plan With a Dry Run

Run the whole preflight without starting anything:

```sh
vitrify run --dry-run
```

A dry run prints the job header — the done-condition, the plan, the worker cap, the parameters in effect — and names any claim it would recover from a run that died, lapsed leases, and a primary checkout that is behind; then it exits 0. In a repository with several SDDs it prints the plan of the whole project instead, as [Plan a Project Run](#plan-a-project-run) shows. It publishes nothing and moves no file, and is safe beside a run that is going. Its last line is the command that starts the run it checked, such as `Start: vitrify run --sdd specs/calc`. If preflight fails, it prints each failure as `preflight failed: <cause> → <fix>` — the fix is the command or action that clears it — and exits 2. [Runs](../about/concepts/runs.md#preflight) lists every check and its fix.

## 2. Start the Build

Start the run and keep both of its output streams:

```sh
vitrify run 2>&1 | tee run.log
```

stdout is the narration — the header, one line per event, and the final report — and stderr carries each worker's report, each gate failure, and any notes and warnings. To leave a run going after you close the terminal:

```sh
nohup vitrify run > run.out 2> run.err &
```

The header's last line, `Watch:`, names the command that watches the run and, once it ends, prints its report: `vitrify run status --job <job>`.

`vitrify run` preflights (Active SDD committed at the mainline head, lint, audit, the SDD's §5 bootstrap, build and lint commands, ≥ 1 probed harness), prints the job header with the done-condition, then keeps every worker slot busy: derive state fresh → claim-then-build a worker for each claimable unit, whatever its group → take each worker's one-line report as it arrives → integrate what is built as one batch, fencing check then write-set check first → re-derive and refill the freed slots.

Use these options when the defaults do not fit:

| Option | Use it to |
| --- | --- |
| `--sdd <root>` | build one SDD of a repository that holds several, as a single lane |
| `--upstream` | with `--sdd`, build the SDDs it depends on too, as one project run |
| `--recheck` | run every Verified unit's rows and every §5 Test command again, whatever the records say; nothing is rebuilt |
| `--cap N` | change how many workers run at once, across every SDD (default 4; a single-file SDD is always 1) |
| `--gate-jobs N` | change how many Verification rows the gate runs at once (default one per CPU core; `1` runs them one at a time) |
| `--wait` | keep the run alive when only blocked units remain, until the spec changes |
| `--mainline BRANCH` | override which branch is mainline |

### Start a Run for an Agent

A build can take hours, and a command started from an agent session ends with the session. When an agent operates the build, it follows `vitrify brief run`: a long build is best started by the human in their own terminal, with the `nohup` line above, and watched from the session; when the agent starts it, it uses the harness's background execution, which notifies on completion — never a foreground call, which outlasts the harness's limit on a command. vitrify stops cleanly on a signal, and the next run resumes, so nothing is lost either way. [Agent Prompts](#agent-prompts) gives the prompt that hands a build to an agent.

## 3. Or Build Interactively

Instead of `vitrify run`, you can drive the same state machine from a harness session. Start the session with the prompt:

- **Claude Code** — `claude "Use vitrify cli to build the spec from this session, without vitrify run."`
- **Codex** — `codex "Use vitrify cli to build the spec from this session, without vitrify run."`
- **OpenCode** — `opencode run "Use vitrify cli to build the spec from this session, without vitrify run."`

The agent finds the kickoff procedure, which `vitrify brief kickoff` prints, and follows it. The session plans and dispatches; it never writes code, claims a unit itself, or edits spec bodies. Prefer `vitrify run` for unattended builds, since it executes the same procedure deterministically.

## 4. Follow Progress

Watch the build from a second terminal:

```sh
vitrify run status --follow
vitrify ledger progress
vitrify questions --open
tail -f .vitrify/logs/<job>/worker-<job>-w1.out
```

`<job>` is the run's ID, from the first line of its output and the header's `Watch:` line. `run status --follow` shows what each unit is doing and what the gate is running, then one line per change until the run ends. `ledger progress` prints one line, such as `Progress: 1/5 Verified · 4 Ready`; `questions --open` lists what a Blocked run is waiting on, and for which units. [Monitor a Run](monitor-a-run.md) lists every way to watch a run.

## 5. Read the Result

The run terminates in exactly one of two states, with the evidence quoted. Use this table to read it.

| Report | Means | Exit |
| --- | --- | --- |
| `Complete.` | every non-Retired unit Verified; audit clean; replay clean; §5 Test passing | 0 |
| `Blocked.` | every remaining unit's blocker named | 2 |

Other endings have their own exit. A preflight that refuses, or a judge's `abort` verdict, exits 2. An error exits 1. A project run that meets an SDD at schema 2.0 whose units consume pins refuses with exit 4. The same table, with what a signal does, is in [Monitor a Run](monitor-a-run.md#tell-whether-a-run-is-alive).

The report is kept in `.vitrify/logs/<job>/report.txt`, and `vitrify run status` prints it under `Report:` once the run has ended — so the result can be read even when nobody kept the run's output. An error ends the run with exit 1, with the message as the last thing on stderr; a signal ends it after it has cleaned up (see [Stop and Resume](#stop-and-resume)). [Monitor a Run](monitor-a-run.md#read-what-the-run-prints) shows both reports in full.

## Resolve a Blocked Build

`Blocked.` gives every unfinished unit one line, with the most specific cause known. Above them come whatever is wrong with mainline itself — audit findings, a §5 command or the Test command failing at the head, a journal that no longer replays — so fix those first. Use this table to act on each unit line.

| Blocker line | Do this |
| --- | --- |
| `open Q-###: …` | Answer the question in the spec under a change entry, mark its Decision Log entry `resolved`, and commit to mainline. `vitrify questions --open` lists the units each question holds up. |
| `its reading set does not resolve: …` | Restore the missing section or contract; for a pin whose authority moved, run `vitrify pin sync`, write the change entry, and run `vitrify change apply`. |
| `set aside after N attempts that did not land; last outcome …` | Read why the attempts failed — see [Find Why an Attempt Did Not Land](monitor-a-run.md#find-why-an-attempt-did-not-land) — then fix the spec or the environment. |
| ``failing gate `<command>`: <its last lines>`` | Run the command yourself from the repository root, and fix the spec, the command, or the environment. |
| `N attempt(s); last outcome <outcome> — <why>` | Read the outcome in [Monitor a Run](monitor-a-run.md#read-the-run-record) and act on the reason. |
| `dependency chain blocked: U-001 (Ready) ← …` | Resolve the unit at the root of the chain. |
| `not attempted this run (status …)` | Check the unit's status and dependencies with `vitrify locate --units`. |

Then run `vitrify run` again. Resume is re-derivation from repository state: a new run continues from what the files say.

## Stop and Resume

Ctrl-C (or `kill`) stops a run cleanly: it stops its workers, releases their claims, in every SDD of a project run, and closes `.vitrify/run.json`. A run that was killed outright cannot do that, so the next one does it for it — claims held by a run that is gone are released before anything is dispatched. Either way no work is lost: an attempt that did not land is kept on its branch, and the unit's next worker starts from its files and is told why it failed.

To resume — after a crash, an interrupt, or a Blocked exit once the named blockers are fixed — run `vitrify run` again. A second Ctrl-C ends a run at once, and the next run recovers what is left; if a worker outlived its run, the next run prints the `kill` command that stops it.

## Fix the Spec While a Run Is Going

A spec fix can land while `vitrify run` is running — that is how a `blocked` unit gets unblocked without a restart. Commit the change to mainline; the run notices: units whose recorded trace no longer matches are marked Stale and rebuilt, a worker still building from the old text is stopped and its unit started again, and a unit parked on an open `Q-###` returns to the pool as soon as its reading set changes. `vitrify run --wait` keeps the run alive when only blocked units remain, so the fix can arrive at any time.

Commit from a checkout that is on mainline and up to date — never from a primary checkout the run has reported as behind. [Change a Spec](change-a-spec.md) covers writing the change entry.

## Build Several SDDs

In a repository with several SDDs, a plain `vitrify run` is a **project run**: every Active SDD is a lane, and one scheduler builds them all as one deterministic, concurrent, incremental build. A unit whose `depends_on` names a pin waits for the units that produce it, in any SDD; every other ready unit builds at once, critical path first, over one worker pool and one gate pool. A change that lands in one SDD makes the dependent units of another re-checked, without a model call. A repository must be prepared for this; see [Work With Several SDDs](multiple-sdds.md#prepare-the-repository).

Everything above applies to a project run, with these differences.

- **`--sdd <root>` builds one SDD** as a single lane, with the one-SDD header and report. A unit that waits on an SDD outside the run ends it `Blocked.`, naming that SDD. `--sdd <root> --upstream` builds the SDD and the SDDs it depends on.
- **Preflight has a project part,** before anything runs: schema 2.1 wherever units consume pins, one acyclic unit graph, unique slugs. A failure refuses the whole run, with exit 2, or exit 4 for a 2.0 SDD that consumes pins. A lane's own checks run before its first unit; a lane that fails them is blocked, and only what depends on its units waits.
- **A repository with one SDD** is the same run with one lane, and prints the one-SDD header.

### Plan a Project Run

`vitrify run --dry-run` prints the plan of the whole project, lane by lane, and its critical path:

```text
Project build job — 2 SDDs: 1 to build · 1 idle  [dry-run]
Plan: regenerate 6 units · adopt 0 built attempts · re-check 0 units
Critical path: core U-001 → api U-001 → api U-002 → api U-003 (4 units)
Capacity: workers 4 · gate jobs 8 · lanes unlimited
Lanes:
  api   idle   0/3 Verified · regenerate 3 · adopt 0 · re-check 0  (specs/api)
  core  build  0/3 Verified · regenerate 3 · adopt 0 · re-check 0  (specs/core)
Regenerate (6):
  api U-001
  api U-002
  api U-003
  core U-001
  core U-002
  core U-003
Adopt (0):
Re-check (0):
dry-run: preflight green; stopping before dispatch
Start: vitrify run
```

The header line counts the lanes by state. `Plan:` counts the units that will be regenerated by workers, the built attempts that will be adopted — claimed again and sent to the gate with no worker — and the units that will be re-checked. A unit is re-checked rather than rebuilt when a path it is checked against changed; the dry run counts those that are already due, and a change that lands during the run adds more. `Critical path:` is the longest chain of units that wait on each other: no worker count finishes the build sooner than that chain. `Capacity:` shows the worker cap, the gate pool, and the lane cap. The same repository and configuration print the same plan. The last line is the command that starts the run, with the flags you gave.

### Read a Project Run

The run prints the header, then one line per event, each starting with its lane in brackets, then one report line per SDD:

```text
Project build job — 2 SDDs: 1 to build · 1 idle  [run-20261122T043831Z-p72112]
Plan: regenerate 6 units · adopt 0 built attempts · re-check 0 units
Critical path: core U-001 → api U-001 → api U-002 → api U-003 (4 units)
Capacity: workers 4 · gate jobs 8 · lanes unlimited
Watch: vitrify run status --job run-20261122T043831Z-p72112 --follow
[core] dispatch U-001 (c1) → w1
[core] integrated U-001 (c1)
[api] dispatch U-001 (c1) → w2
[core] dispatch U-002 (c1) → w3
[api] integrated U-001 (c1)
…
[api] integrated U-003 (c1)

Complete.
  api      Complete     3/3 Verified · built 3 · §5 Test passing
  core     Complete     3/3 Verified · built 3 · §5 Test passing
```

`api` U-001 is dispatched as soon as `core` U-001 has landed, because it depends on a pin that unit produces. [Monitor a Run](monitor-a-run.md#read-a-project-runs-output) lists every line, including the re-checks, adopted attempts, and rotten units.

The report has one line per SDD, sorted by slug: `Complete` with what the lane did (`built 3`, `re-checked 2 (core changed), no model calls`, `§5 Test passing`), `Up to date` with `nothing changed since <commit>`, or `Blocked` with its cause, each further blocker indented below. The run is `Blocked.` when any SDD is. Resolve each blocker as in [Resolve a Blocked Build](#resolve-a-blocked-build); a unit that waits on another SDD's unit names it, as `waits on core U-002 (Ready)`, and a rotten unit names the commit that broke it ([Read a Rotten Unit](monitor-a-run.md#read-a-rotten-unit)).

A run with nothing to do ends at once, `Complete.`, with every SDD `Up to date`. `vitrify run --recheck` runs every Verified unit's rows and every SDD's §5 Test command again, rebuilds nothing, and is the way to distrust the recorded checks.

### Interrupt and Resume

One Ctrl-C stops a project run as it stops any run: it stops every worker, releases every claim in every lane, and ends by the signal. Run `vitrify run` again to resume. Units that landed stay landed, and an attempt that was built but did not land is adopted — sent to the gate with no worker — if the text it was built from has not moved (`[api] U-002 adopted (built attempt c1) → gate`).

## Common Issues

- **Every attempt is released, or workers make no progress.** A headless session cannot answer permission prompts. See [Workers Make No Progress](../resources/troubleshooting.md#workers-make-no-progress-or-every-attempt-is-released).
- **`preflight failed: …`** — each line names its fix after the arrow; see [`vitrify run` Exits 2 With `preflight failed`](../resources/troubleshooting.md#vitrify-run-exits-2-with-preflight-failed).
- **`vitrify run` exits 4 with an SDD named.** A project run refuses an SDD at schema 2.0 whose units consume pins; migrate it, or build it alone with `--sdd`. See [A Project Run Refuses an SDD at Schema 2.0](../resources/troubleshooting.md#a-project-run-refuses-an-sdd-at-schema-20).
- **The run warns that the primary checkout is behind.** Clear what is in the way and it catches up at the next commit; see [The Primary Checkout Is Behind](../resources/troubleshooting.md#the-primary-checkout-is-behind).

## Next Steps

- [Monitor a Run](monitor-a-run.md) — read the header, events, run record, worker logs, and history.
- [Change a Spec](change-a-spec.md) — change an Active SDD and rebuild what the change reaches.
- [Work With Several SDDs](multiple-sdds.md) — prepare a repository of several SDDs for a project run.
- [Runs](../about/concepts/runs.md) — what happens at each stage of a run.
