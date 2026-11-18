# Work With Several SDDs

> Keep several SDDs in one repository, consume another SDD's contract through a checked pin, and bring consumers back in step when a producer changes.

Use this guide when one repository holds more than one SDD — typically one per subsystem — and some of them code against each other's surfaces. vitrify scopes everything to one SDD, so a unit that uses another SDD's surface consumes an **external contract** of its own SDD that stands for it: a *pin* on the hash of a contract another SDD owns, or a *transcription* of a source no SDD owns. [Contracts](../about/concepts/contracts.md#external-contracts) explains why; the exact formats are in the [Data Model and Formats Specification](../reference/formats.md#8-external-contracts). A coding agent changing several SDDs at once follows the procedure `vitrify brief change` prints.

## Agent Prompts

Your coding agent can keep several SDDs in step for you. In a session of Claude Code, Codex, or OpenCode at the repository root, send the prompt for your task. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| change a contract other SDDs use, and bring them in step | `Use vitrify cli to change <producer>'s <contract>: <what changes>, and bring every SDD that uses it in step.` |
| have one SDD use another's contract | `Use vitrify cli to make <consumer> use <producer>'s <contract>.` |
| build every SDD | `Use vitrify cli to build every SDD.` |
| check the whole repository | `Use vitrify cli to check the whole repository is consistent.` |

For a change, the agent follows `vitrify brief change`: it checks the SDDs and their pins first, then works producers before consumers — one change entry per SDD, `vitrify pin sync` between them — gates the repository with `vitrify check --all`, and commits once. It reports each SDD's entry and `Staleness applied` line. A build runs one SDD at a time, producers first, as in [Build Several SDDs](#5-build-several-sdds).

## What You Build

When you finish, the repository has these properties:

- Several SDDs, each with its own `sdd:` slug and disjoint `scope_roots`.
- For every surface one SDD uses from another, a pin in the consuming SDD; for every surface no SDD owns, a transcription.
- A routine that keeps consumers in lockstep with producers: one entry per SDD, producers first, pins synced between them, one gate, one commit.
- One build per SDD, run in dependency order.

## Before You Start

Settle these points first; vitrify enforces the first two and leaves the third to you.

- Every SDD needs a unique `sdd:` slug. A run refuses to start on a slug two SDDs share.
- Owned paths of coexisting SDDs must not overlap.
- Decide the build order between SDDs. Nothing in an SDD declares it, and one run builds one SDD.

## 1. Lay Out the SDDs

Give each SDD its own root — `specs/<slug>/` for a bundle, `specs/<slug>.sdd.md` for a single file — its own slug, and disjoint `scope_roots`. Then name the SDD on every verb with `--sdd <root>`, a bundle directory, its `sdd.md`, or a single file, written as a path from where the verb runs. The verbs that check — `lint`, `check`, `audit`, `ledger`, `pin`, `journal replay`, `change apply`, and `questions` — also take `--all`:

```sh
vitrify locate
vitrify lint --sdd specs/checkout
vitrify check --all
```

`vitrify locate` lists every SDD in discovery order. A verb that acts on one SDD exits 4 when several are found and `--sdd` is absent, and prints the candidates as `--sdd` values. With `--all`, each line of output begins with the SDD's slug (`calc: lint clean …`), one SDD's findings do not stop the rest, and the exit code is the worst of theirs.

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

## 3. Check External Contracts

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
| `stale` | run `vitrify pin sync` ([step 4](#4-bring-consumers-back-in-step)) |
| `missing-authority` | correct the slug or ID on the `Pin:` line, or restore the authority |
| `invalid` | fix the declaration: a pin has no body; a `Producer:` line that names another SDD's contract needs a `Pin:` line; the authority needs a `Surface` heading; slugs must be unique |
| `unchecked` | nothing vitrify can do; keep the transcription current yourself |

Lint reports `stale`, `missing-authority`, and `invalid` contracts as L9 findings, and `vitrify check` as `[pin]` findings, so a consumer with one of them does not start a run.

## 4. Bring Consumers Back in Step

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

Each consumer's next `vitrify run` rebuilds what the producer's change reached. Until its pins are synced, its `lint` — and so its run's preflight — refuses. `sync` writes pin hashes and nothing else — no status, no journal event, no version. What it cannot put right — a missing authority, an invalid declaration — is named on stderr as `not synced: …` and makes the exit code 2.

## 5. Build Several SDDs

Run one build per SDD, in dependency order — producers before the SDDs that consume them:

```sh
vitrify run --sdd specs/pool
vitrify run --sdd specs/checkout
```

Each SDD's work lives on its own branches, `sdd/<slug>/<unit>/c<seq>`, and in its own worktrees, so a `U-001` in one never meets the `U-001` of another. vitrify guarantees that such runs cannot damage each other; the order between them is yours.

## 6. Check the Whole Repository

Run the whole repository's gate in one command:

```sh
vitrify check --all
```

For every SDD it runs lint (`--built` semantics once the SDD has build state), the audit, the journal replay where a journal exists, the ledger check, and the pin states, and lists open questions as notes. Each line begins with the SDD's slug and the check that found it, and names its fix; the last line is `check clean: 3 SDDs`, or the count of findings. It writes nothing.

## Common Issues

- **A consumer's run refuses with L9.** A pin no longer matches its authority: run `vitrify pin sync --all`, write the consumer's change entry around its line, run `vitrify change apply --sdd <consumer>`, and commit.
- **`change apply` refuses because a pin is stale.** Sync first: `vitrify pin sync --sdd <root>`, then apply.
- **A unit is `not started — …` during a run.** A pin's authority moved after preflight; the unit is set aside until you sync, write the entry, and commit.
- **`not synced: …`.** The authority is missing or the declaration is invalid; fix the contract's header by hand.
- **A verb exits 4 and lists several SDDs.** Pass one of the printed `--sdd` values, or `--all` where the verb takes it.

## Next Steps

- [Change a Spec](change-a-spec.md) — write the change entries that carry a producer's change.
- [Contracts](../about/concepts/contracts.md) — why vitrify checks external contracts in lockstep.
- [Command Reference](../cli/command-reference.md#vitrify-pin) — every `pin` option and output field.
