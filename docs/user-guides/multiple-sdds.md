# Work With Several SDDs

> Keep several SDDs in one repository, declare what each unit needs from another SDD, bring consumers back in step when a producer changes, and build them all with one `vitrify run`.

Use this guide when one repository holds more than one SDD — typically one per subsystem — and some of them code against each other's surfaces. vitrify scopes each SDD's text to itself, so a unit that uses another SDD's surface consumes an **external contract** of its own SDD that stands for it: a *pin* on the hash of a contract another SDD owns, or a *transcription* of a source no SDD owns. [Contracts](../about/concepts/contracts.md#external-contracts) explains why; the exact formats are in the [Data Model and Formats Specification](../reference/formats.md#8-external-contracts). A pin that a unit also lists in `depends_on` tells a build to wait for the producer's code. Then a plain `vitrify run` builds every Active SDD as one **project run**: one lane per SDD, one scheduler, one worker pool, and a re-check of what a change in another SDD reached. A coding agent changing several SDDs at once follows the procedure `vitrify brief change` prints.

## Agent Prompts

Your coding agent can keep several SDDs in step for you. In a session of Claude Code, Codex, or OpenCode at the repository root, send the prompt for your task. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| change a contract other SDDs use, and bring them in step | `Use vitrify cli to change <producer>'s <contract>: <what changes>, and bring every SDD that uses it in step.` |
| have one SDD use another's contract | `Use vitrify cli to make <consumer> use <producer>'s <contract>.` |
| build every SDD | `Use vitrify cli to build every SDD.` |
| check the whole repository | `Use vitrify cli to check the whole repository is consistent.` |

For a change, the agent follows `vitrify brief change`: it checks the SDDs and their pins first, then works producers before consumers — one change entry per SDD, `vitrify pin sync` between them — gates the repository with `vitrify check --all`, and commits once. It reports each SDD's entry and `Staleness applied` line. For a build, it follows `vitrify brief run`, which starts one project run, as in [Build Every SDD](#7-build-every-sdd).

## What You Build

When you finish, the repository has these properties:

- Several SDDs, each with its own `sdd:` slug, disjoint `scope_roots` and `schema_version: 2.1`.
- For every surface one SDD uses from another, a pin in the consuming SDD; for every surface no SDD owns, a transcription.
- In each consuming unit, every pin whose code it needs in `depends_on` as well as `consumes`, and every pin it only reads as an interface or as data in `consumes` alone.
- A routine that keeps consumers in lockstep with producers: one entry per SDD, producers first, pins synced between them, one gate, one commit.
- One build, `vitrify run`, that builds every Active SDD in dependency order and re-checks what a change reached.

## Before You Start

Settle these points first; vitrify enforces the first four and leaves the last to you.

- Every SDD needs a unique `sdd:` slug. A run refuses to start on a slug two SDDs share.
- Owned paths of coexisting SDDs must not overlap.
- The units' dependencies, across SDDs, must not form a cycle. A run refuses to start on one and prints it.
- Each SDD whose units consume pins needs `schema_version: 2.1` ([step 6](#6-migrate-an-sdd-to-schema-21)); a project run refuses an SDD at 2.0 that does.
- The repository must meet the conditions of [Prepare the Repository](#prepare-the-repository), which no tool can check for you.

## 1. Lay Out the SDDs

Give each SDD its own root — `specs/<slug>/` for a bundle, `specs/<slug>.sdd.md` for a single file — its own slug, and disjoint `scope_roots`. Then name the SDD on every verb that acts on one with `--sdd <root>`, a bundle directory, its `sdd.md`, or a single file, written as a path from where the verb runs. The verbs that check — `lint`, `check`, `audit`, `ledger`, `pin`, `journal replay`, `change apply`, and `questions` — also take `--all`:

```sh
vitrify locate
vitrify lint --sdd specs/checkout
vitrify check --all
```

`vitrify locate` lists every SDD in discovery order. A verb that acts on one SDD exits 4 when several are found and `--sdd` is absent, and prints the candidates as `--sdd` values. With `--all`, each line of output begins with the SDD's slug (`calc: lint clean …`), one SDD's findings do not stop the rest, and the exit code is the worst of theirs. `vitrify run` is the exception: with several SDDs and no `--sdd`, it builds them all ([step 7](#7-build-every-sdd)).

## 2. Consume Another SDD's Contract

A unit that codes against a surface another SDD owns consumes a contract of its own SDD that stands for it: a pin. Write the pin's header yourself, and let `vitrify pin sync` write its hash: never compute it by hand.

### Write a Pin

Give the pin its own header, with a `Producer:` line that begins `external` and names the authority by the producing SDD's slug and then the contract's ID, a `Consumers:` line naming its own units, and a `Pin:` line. A pin has no body:

```markdown
# C-012 — Pool API: open, submit, admit (pinned)

- Producer: external — pool, C-010
- Consumers: U-005
- Stability: Frozen by its producer
- Pin: pool C-010
```

Other text, such as a path or an edge name, may surround the slug and ID on the `Producer:` line. Written without a hash, the `Pin:` line is a pin `vitrify pin sync` has yet to record; run `vitrify pin sync --sdd <root>`, and it becomes `Pin: pool C-010 @ <hash12>`, the hash of the authority's body from its `Surface` heading to its end. Workers then read the pin's entry followed by the authority's body, which vitrify hands out only while the hash matches.

### Write a Transcription

A surface no SDD owns — an external package, a vendor API, another team's service — can only be transcribed. Begin the `Producer:` line with `external`, name the source but no contract ID, and write the surface as the contract's body yourself. vitrify lists such a contract as `unchecked`, because it is: there is no authority to compare it with, so keeping it current is yours.

## 3. Declare What a Unit Needs

A pin in `consumes` puts the authority's body in the unit's reading set, so a change to the producer's contract reaches the unit as staleness. It orders nothing. To make a build wait, list the pin in the unit's `depends_on` as well (schema 2.1). Pick by what the unit needs:

| The unit needs | Write | Effect |
| --- | --- | --- |
| the **code** behind the surface: it imports or links it, installs its package, or runs its CLI in its checks | the pin in `consumes` and in `depends_on` | the unit waits until the units that produce the authority are built, and is re-checked when their code changes |
| only the **interface** or **data format**: it codes against the shape, or parses the data | the pin in `consumes` alone | nothing is ordered; the unit builds while the producer builds |

```markdown
depends_on: [C-001]
consumes: [C-001]
```

An SDD whose code others install or import publishes a contract for its package surface, produced by its foundation unit; a consumer's foundation unit lists that pin in `depends_on`. The package boundary is then a contract, and a change to the foundation's files — a lockfile, build configuration — re-checks every dependent unit, in every SDD. Pins running both ways between two SDDs are normal, since they order nothing; a cycle through `depends_on` is refused. A transcription and an internal contract never go in `depends_on`: name the unit instead. [Contracts](../about/concepts/contracts.md#pins-carry-order-too) gives the reasoning, and [Author a Spec](author-a-spec.md#classify-each-pin-a-unit-consumes) the rule.

Print the edges you declared:

```sh
vitrify locate --graph
```

```text
api → core  via C-001, C-002
  api U-001 → core U-001  via C-001
  api U-002 → core U-002  via C-002
core  (depends on no other SDD)
```

Each line under an SDD names a unit that waits for another SDD's unit, and the pin it waits through. A unit that uses another SDD's code but is missing from the graph is neither waited for nor re-checked when that code changes: compare the graph with what the code imports. `--json` prints the unit graph and its order.

## 4. Check External Contracts

List every external contract and its state:

```sh
vitrify pin list --all
```

```text
beta C-001   ok                 pin of alpha C-001  specs/beta/contracts/C-001-greeting.md
beta C-002   stale              pin of alpha C-002  specs/beta/contracts/C-002-farewell.md — its authority's body changed since the pin was written
beta C-003   missing-authority  pin of alpha C-009  specs/beta/contracts/C-003-ledger.md — its authority cannot be found: alpha has no contract C-009
beta C-004   unchecked          transcription       specs/beta/contracts/C-004-clock.md
gamma C-001  ok                 pin of alpha C-001  specs/gamma.sdd.md
5 external contracts: 2 ok · 1 stale · 1 missing-authority · 1 unchecked
```

Use this table to act on each state; the [Command Reference](../cli/command-reference.md#vitrify-pin) defines them exactly.

| State | Do this |
| --- | --- |
| `ok` | nothing |
| `stale` | run `vitrify pin sync` ([step 5](#5-bring-consumers-back-in-step)) |
| `missing-authority` | correct the slug or ID on the `Pin:` line, or restore the authority |
| `invalid` | fix the declaration: a pin has no body; a `Producer:` line that names another SDD's contract needs a `Pin:` line; the authority needs a `Surface` heading; slugs must be unique |
| `unchecked` | nothing vitrify can do; keep the transcription current yourself |

Lint reports `stale`, `missing-authority`, and `invalid` contracts as L9 findings, and `vitrify check` as `[pin]` findings, so a consumer with one of them does not start a run.

## 5. Bring Consumers Back in Step

When producers' contracts change, the change reaches their consumers through each consumer's own change entry, not by itself. Work producers before consumers, one change entry per SDD:

1. **Each producer.** Edit its text, write its change entry, and record it with `vitrify change apply --sdd <producer>`, as in [Change a Spec](change-a-spec.md#land-the-change).
2. **Sync the pins.** Once every producer's entry is written, sync every stale pin:

   ```sh
   vitrify pin sync --all
   ```

   `sync` rewrites each stale pin's hash in the working tree and prints, per consumer, the Built and Verified units whose recorded trace the rewrite moved, as the line that consumer's change entry needs:

   ```text
   synced beta C-002 ← alpha C-002  specs/beta/contracts/C-002-farewell.md
   beta: - Staleness applied: U-001 → Stale (consume C-002)
   1 synced · 1 not synced
   ```

   On stderr it names the next step for each consumer: `next: write beta's change entry around its line, then run vitrify change apply --sdd specs/beta`. Add `--dry-run` to see the same lines without writing anything.
3. **Each consumer it names.** Write that consumer's own change entry around its printed line, with its own version bump, then run `vitrify change apply --sdd <consumer>`. An entry never marks another SDD's units: each SDD's staleness is recorded by its own entry.
4. **Gate the repository:**

   ```sh
   vitrify check --all
   ```

   It exits 0 when every SDD's lint, audit, journal, ledger, and pins are clean. Then run the project's own checks, where the repository documents any — an index of SDD conventions, scripts beside the specs.
5. **Commit once,** at the mainline head. These verbs edit the working tree only, so the text, every entry, and every mark land together.

Each consumer's next `vitrify run` rebuilds what the producer's change reached, and re-checks the units that depend on the producer's code. Until its pins are synced, its `lint` — and so its run's preflight — refuses. `sync` writes pin hashes and nothing else — no status, no journal event, no version. What it cannot put right — a missing authority, an invalid declaration — is named on stderr as `not synced: …` and makes the exit code 2.

## 6. Migrate an SDD to Schema 2.1

An SDD whose units consume pins declares `schema_version: 2.1` once, so that a project run knows which pins carry code. Until it does, a plain `vitrify run` refuses with exit 4, naming the SDD, and `vitrify run --sdd <root>` builds it alone. For each SDD: classify each pin its units consume, add the code pins to `depends_on`, set `schema_version: 2.1`, write one PATCH change entry with `Staleness applied: —`, run `vitrify change apply --sdd <root>`, and commit. A PATCH entry keeps every unit's status and stamp: the edit records a need the built code already has, so nothing is rebuilt. [Migrate an SDD to Schema 2.1](change-a-spec.md#migrate-an-sdd-to-schema-21) has the steps. An SDD that `vitrify new sdd` writes starts at 2.1.

## 7. Build Every SDD

In a repository with several SDDs, a plain `vitrify run` is a project run: every Active SDD is a lane. Start with the dry run, which checks the whole repository and prints the plan:

```sh
vitrify run --dry-run
```

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
  …
dry-run: preflight green; stopping before dispatch
Start: vitrify run
```

Then start the build, as [Run a Build](run-a-build.md#2-start-the-build) describes:

```sh
vitrify run
```

```text
[core] dispatch U-001 (c1) → w1
[core] integrated U-001 (c1)
[api] dispatch U-001 (c1) → w2
[core] dispatch U-002 (c1) → w3
…

Complete.
  api      Complete     3/3 Verified · built 3 · §5 Test passing
  core     Complete     3/3 Verified · built 3 · §5 Test passing
```

`api` starts its first unit when `core`'s first unit has landed, because it depends on a pin that unit produces; every other unit that is ready builds beside it. A second run with nothing changed prints `Up to date   nothing changed since <commit>` for each SDD and builds nothing.

To build one SDD alone, name it: `vitrify run --sdd specs/api` builds it as a single lane, and a unit that waits on an SDD outside the run ends the run `Blocked.`, naming it. To build an SDD with the SDDs it depends on, add `--upstream`:

```sh
vitrify run --sdd specs/api --upstream
```

Only SDDs with `status: Active` are lanes. Each SDD's work lives on its own branches, `sdd/<slug>/<unit>/c<seq>`, and in its own worktrees, so a `U-001` in one never meets the `U-001` of another.

### What a Lane Is

A **lane** is one SDD's part of the run: its own integration checkout and lock, its own integrator, and its own records, with the worker pool, the gate pool, and the scheduler shared. A lane is in one state at a time; `vitrify run status` shows it.

| State | Means |
| --- | --- |
| `idle` | units remain, but none can be claimed, is in flight, or is due a re-check; each waits on units elsewhere |
| `build` | some unit is claimable, adoptable, or in flight |
| `recheck` | Verified units that a change in another SDD reached are being checked again, with no workers |
| `up to date` | nothing to regenerate, re-check, or build |
| `complete` | its done-condition holds at the end of the run |
| `blocked` | unfinished, set-aside, or rotten units remain, or its own preflight failed |

### How Failures Stay Local

A unit that is blocked, set aside, or rotten holds up only the units that depend on it, in any SDD. Every other unit keeps building and landing, and a lane whose preflight failed blocks only what depends on its units. The run ends `Blocked.` with one line per SDD; fix what the line names, commit, and run again. A change in one SDD re-checks the dependent units of another without a model call: their rows run again and a pass is recorded. A row that then fails marks its unit **rotten**, and the run searches for the commit that broke it; see [Read a Rotten Unit](monitor-a-run.md#read-a-rotten-unit).

## 8. Check the Whole Repository

Run the whole repository's gate in one command:

```sh
vitrify check --all
```

For every SDD it runs lint (`--built` semantics once the SDD has build state), the audit, the journal replay where a journal exists, the ledger check, and the pin states, and lists open questions as notes. Each line begins with the SDD's slug and the check that found it, and names its fix; the last line is `check clean: 3 SDDs`, or the count of findings. It writes nothing.

## Prepare the Repository

A project run is only as sound as the repository it builds. vitrify enforces what it can see; the rest is the repository's to provide, because vitrify reads no source code. [Design Rationale](../about/design-rationale.md#what-a-repository-provides) gives the reasons.

- **Make commands and tests pass while upstreams are partly built.** Bootstrap, Build, and Lint run clean before anything is built, and tests skip until their subject exists. Guard on presence rather than refusing.
- **Keep a whole-suite §5 Test command.** A re-check runs only the rows of units a change reached; the Test command runs at every gate and catches coupling the declarations miss. An SDD without one re-runs every row at every gate.
- **Write Verification rows that exercise what a unit uses.** A change in a dependency that breaks the unit should fail one of its rows; a row that checks nothing the dependency touches makes the re-check vacuous.
- **Make every command safe beside other SDDs' commands.** Commands of different SDDs run at once, so none may rely on a fixed port, a shared directory, or a global lock.
- **Reproduce upstream outputs in every checkout.** Each gate starts from a pristine checkout of mainline, with untracked and ignored files removed. What a downstream SDD needs from an upstream is committed by the upstream's units or rebuilt by the downstream's Bootstrap; nothing may depend on a file another checkout left behind. Keep tool caches outside the checkout.
- **Give repository-wide inputs an owner, or list them.** A file outside every SDD's `scope_roots` is an input of every unit, so a change to it re-checks everything. Put shared build inputs under an SDD the others depend on, or list the paths an SDD's commands read in its `shared_inputs`.
- **Bound each suite's parallelism.** A §5 Test command or Verification row takes one gate token, so a runner that fans out across every core on its own needs its own cap.

## Size the Pools

Three settings bound a project run, each for the whole repository:

| Setting | Default | Bounds |
| --- | --- | --- |
| `run.worker_cap` (`--cap N`) | 4 | model sessions at once, across every lane; also the cost dial |
| `run.gate_jobs` (`--gate-jobs N`) | one per CPU core | commands at once, from every gate, re-check, and preflight in any lane |
| `run.lane_cap` | no limit | lanes active at once, and so how many lane checkouts exist on disk |

A project run does not raise `run.worker_cap` on its own. A higher cap is not guaranteed to be faster: no cap beats the critical path, the longest chain of units that wait on each other, which `vitrify run --dry-run` prints. Raise `--cap` when the total work divided by the cap, not the critical path, dominates. [Configuration](../cli/configuration.md#tune-throughput) lists the settings.

## Common Issues

- **A plain `vitrify run` refuses with exit 4.** An SDD at schema 2.0 has units that consume pins. Migrate it ([step 6](#6-migrate-an-sdd-to-schema-21)), or build it alone with `--sdd`; see [A Project Run Refuses an SDD at Schema 2.0](../resources/troubleshooting.md#a-project-run-refuses-an-sdd-at-schema-20).
- **A run refuses with a dependency cycle.** Remove one `depends_on` entry on the printed path, keeping the pin in `consumes`; see [A Project Run Refuses a Cycle](../resources/troubleshooting.md#a-project-run-refuses-a-cycle).
- **A unit is `rotten`.** A change upstream broke its rows; the run names the culprit commit. See [A Unit Is Rotten](../resources/troubleshooting.md#a-unit-is-rotten).
- **A consumer's run refuses with L9.** A pin no longer matches its authority: run `vitrify pin sync --all`, write the consumer's change entry around its line, run `vitrify change apply --sdd <consumer>`, and commit.
- **`change apply` refuses because a pin is stale.** Sync first: `vitrify pin sync --sdd <root>`, then apply.
- **A unit is `not started — …` during a run.** A pin's authority moved after preflight; the unit is set aside until you sync, write the entry, and commit.
- **`not synced: …`.** The authority is missing or the declaration is invalid; fix the contract's header by hand.
- **A verb exits 4 and lists several SDDs.** Pass one of the printed `--sdd` values, or `--all` where the verb takes it.

## Next Steps

- [Run a Build](run-a-build.md) — start, read, and resume a project run.
- [Monitor a Run](monitor-a-run.md) — read the lanes, the events, and a rotten unit.
- [Change a Spec](change-a-spec.md) — write the change entries that carry a producer's change.
- [Contracts](../about/concepts/contracts.md) — why vitrify checks external contracts in lockstep, and what a pin orders.
- [Command Reference](../cli/command-reference.md#vitrify-pin) — every `pin` option and output field.
