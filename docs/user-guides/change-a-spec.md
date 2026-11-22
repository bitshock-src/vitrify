# Change a Spec

> Change an Active SDD after it is built: edit the text, write the change entry, record it with `vitrify change apply`, gate with `vitrify check`, and let the next run rebuild only what the change reaches.

Use this guide when an Active SDD needs to change — a fix to a contract, a new feature, a correction to a unit's text, or the answer to a question. An Active SDD changes only through change entries: the spec text is always current truth, edited in place, and history exists to compute impact. Editing a frozen contract moves the trace of every stamped consumer, with the cause named — the reading set and the staleness set are the same list by construction. [Build State](../about/concepts/build-state.md#input-traces-and-staleness) explains the mechanism. A coding agent follows the same procedure from `vitrify brief change`.

## Agent Prompts

Your coding agent can land a change for you. In a session of Claude Code, Codex, or OpenCode at the repository root, send the prompt for your change. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| change the spec | `Use vitrify cli to change the spec: <what changes, and why>` |
| answer a question a build is waiting on | `Use vitrify cli to answer Q-###: <your answer>` |
| add a feature to an SDD you have | `Use vitrify cli to add this feature: <its PRD, or its path>` |

The agent finds the change procedure in `vitrify --help` — `vitrify brief change` — and follows it: it edits the text in place, writes the change entry, records it with `vitrify change apply`, gates it with `vitrify check`, and commits once. A change that revises the TDD, such as a feature, starts with the agent's questions, and you approve the revision before the SDD changes. It reports each entry with its version bump, its `Staleness applied` line, and what the next build rebuilds.

## What You Build

A landed change has these properties:

- One change entry, `CH-###`, recording the version bump and what the change adds, modifies, removes, and stales.
- Edited spec text, with the SDD's `version` and `updated` matching the entry (lint L10).
- Staleness marks on exactly the units whose traces the edit moved — or re-recorded traces, for an editorial change — and the entry's `change` event in the journal.
- One commit on mainline. The next `vitrify run` rebuilds what the change reached.

## Before You Start

Decide what kind of change it is; the bump decides what `vitrify change apply` does with the units the edit moved.

| Bump | When | What `vitrify change apply` does |
| --- | --- | --- |
| PATCH | editorial only: wording that changes no behavior | re-records their traces, keeping status and stamps, so nothing is rebuilt |
| MINOR | units added, modified, or retired | marks them Stale, so the next run rebuilds them |
| MAJOR | a contract or a technology decision changed | marks them Stale, so the next run rebuilds them |

If a run is building this SDD, you can still land the change — see [Change the Spec During a Run](#change-the-spec-during-a-run).

### What a Change Reaches

A change can force two different kinds of redo, and the next `vitrify run` does each only where the change reaches. Use this table to see which one your edit triggers.

| | Regenerate | Re-check |
| --- | --- | --- |
| What changed | spec text a unit is generated from: its own entry, a contract it consumes (a pin included), Ground Rules, Technology Decisions | code a unit is checked against: its own files, and those of every unit it depends on, in this SDD or through a pin |
| What the next run does | a model call: the unit is marked Stale and rebuilt | no model call: its Verification rows run again, and a pass is recorded |
| Where you see it | `vitrify change apply` names the Stale units | `vitrify check` lists the dirty units; `vitrify run --dry-run` plans them |

A change entry therefore regenerates what the spec edit moved and nothing else. Code that changes afterward, in this SDD or in another, re-checks its dependents and regenerates none of them, because a unit is generated against frozen contracts, never against its dependencies' code. A unit that is regenerated into byte-identical files changes no path, so nothing downstream of it is re-checked: that is early cutoff. [Build State](../about/concepts/build-state.md#the-two-cascades) explains both.

## Write the Change Entry

Every change entry lists the version bump, the TDD pin status, the units and contracts added, modified, and removed, the staleness marks applied, and the resulting codebase delta in one line. This is the SDD schema's example:

```markdown
## CH-006 — Cart line-item discounts (2026-10-23) — 1.3.0 → 1.4.0
- TDD pin: unchanged (2.1)
- Added:    U-013, U-014 (group C); contract C-007
- Modified: U-004 (R2 rewritten), C-002 (additive field `discountMinor`)
- Removed:  —
- Staleness applied: U-004 → Stale; U-009, U-012 → Stale (consume C-002)
- Codebase delta = build U-013, U-014; rebuild U-004, U-009, U-012
```

Modified unit text is edited in place; the previous text survives in version control. SDD schema §12 (`vitrify schema show`) has the full rules. You need not assemble the `Staleness applied` line yourself: `vitrify change apply` prints it.

## Land the Change

Follow these steps for every bump:

1. Edit the spec text in place. If the TDD gained a revision, re-pin `tdd_version`.
2. Write the change entry, and set the SDD's `version` and `updated` to match it.
3. Record the entry:

   ```sh
   vitrify change apply --sdd specs/calc
   ```

   It reads the last entry's id and version transition, takes the units whose recorded trace no longer matches, and does what the bump says. For PATCH it re-records those traces and appends `change --entry CH-###`. For MINOR or MAJOR it appends `change --entry CH-### --stale <units>`, then marks each unit Stale with a `stale` event carrying its cause. Either way it regenerates the ledger and prints what it did:

   ```text
   calc: CH-004 1.2.0 → 1.3.0 (minor): U-002, U-004 marked Stale
   calc: - Staleness applied: U-002, U-004 → Stale
   ```

   Add `--dry-run` to see the same lines without writing anything.
4. Copy the `- Staleness applied:` line it prints into the entry. A PATCH entry's line reads `- Staleness applied: —`.
5. Gate the result:

   ```sh
   vitrify check --sdd specs/calc
   ```

   It exits 0 when lint, the audit, the journal replay, the ledger check, and the pin states are all clean; otherwise each finding names its fix.
6. Commit once, at the mainline head. `change apply` edits the working tree only; the commit is yours. `--commit <sha>` sets the code commit the events record.
7. Run `vitrify run`. Stale units are claimable, and the run rebuilds exactly them.

`vitrify change apply` refuses (exit 2) when `version` or `updated` does not match the entry, or when one of the SDD's pins is stale — run `vitrify pin sync --sdd <root>` first, since the sync moves traces too. Running it again is safe: an entry already recorded is skipped (`calc: CH-005 already recorded — nothing to apply`). A Draft SDD, or one with no change entry, has nothing to apply. An Active SDD with no journal yet — its activation entry — gets its `change` event, which creates the journal.

To retire a unit, set its status to `Retired` and say in the change entry which artifacts are deleted; the deletion is executed at integration of that entry.

> [!IMPORTANT]
> A Stale unit is rebuilt by an implementer, who may rewrite everything it owns. If the change stales an authored baseline — tests only you should write — update and re-stamp that unit yourself before the next build, as in [Ship an Authored Baseline](author-a-spec.md#ship-an-authored-baseline).

## Change a Contract Other SDDs Use

When other SDDs pin the contract, the change reaches them through each consumer's own change entry, not by itself. Once the producer's entry is written and applied:

```sh
vitrify pin list --all
vitrify pin sync --all
```

The first lists every pin and which ones are now `stale`; the second rewrites their hashes and prints each consumer's `Staleness applied` line. For each consumer it names, write that consumer's change entry around its line, then run `vitrify change apply --sdd <consumer>`. Until then the consumer's `lint` — and so its run's preflight — refuses. [Work With Several SDDs](multiple-sdds.md#4-bring-consumers-back-in-step) has the full procedure.

## Migrate an SDD to Schema 2.1

An SDD whose units consume pins moves to schema 2.1 so that a project run knows which pins carry code. Until it does, `vitrify run` in a repository of several SDDs refuses with exit 4, naming the SDD, and `vitrify run --sdd <root>` builds it alone. Migrating records a need the built code already has, so it is one PATCH change entry:

1. For each unit, classify each pin it consumes. A pin whose built code the unit imports, links, installs, or runs in its tests is a code pin; a pin it only codes against as an interface or parses as data stays in `consumes` alone. [Author a Spec](author-a-spec.md#classify-each-pin-a-unit-consumes) has the rule.
2. Add each code pin to that unit's `depends_on`, beside its entry in `consumes`.
3. Set `schema_version: 2.1` in the core front matter.
4. Write one PATCH change entry that says so, with `Staleness applied: —`, and set `version` and `updated` to match it.
5. Run `vitrify change apply --sdd <root>`, then `vitrify check --sdd <root>`, and commit once.

Use PATCH and not MINOR. `depends_on` is part of each unit's own entry, so the edit moves each edited unit's trace; a PATCH entry makes `vitrify change apply` re-record those traces and keep every status and stamp, where a MINOR or MAJOR entry would mark the units Stale and have the next run rebuild code that is already correct. A pin in `depends_on` under `schema_version: 2.0` is a lint L1 finding, so set the version in the same commit. `vitrify locate --graph` then shows the edges you declared. If a pin you added closes a cycle, lint L12 and the run's preflight print it as the path that closes it; see [A Project Run Refuses a Cycle](../resources/troubleshooting.md#a-project-run-refuses-a-cycle).

## Change the Spec During a Run

You can land a change while `vitrify run` is building this SDD: commit it to mainline, and the run marks and rebuilds what it stales, stops workers building from the old text, and returns units parked on a question to the pool. [Fix the Spec While a Run Is Going](run-a-build.md#fix-the-spec-while-a-run-is-going) has the details.

At preflight, by contrast, an unmarked trace mismatch is a refusal, so that a spec edit nobody meant does not quietly become a rebuild: outside a run, record the entry with `vitrify change apply` before starting one.

## Validate the Change

The change has landed correctly when these checks pass. Use this table as the acceptance list.

| Check | Evidence |
| --- | --- |
| The entry is recorded | `vitrify change apply --sdd <root> --dry-run` prints `already recorded — nothing to apply` |
| The SDD is consistent | `vitrify check --sdd <root>` exits 0 — lint including L10, the audit with no unmarked trace mismatch, the journal replay, the ledger, and the pins |
| The entry carries its line | the entry's `- Staleness applied:` line is the one `change apply` printed |
| The right units will rebuild | `vitrify locate --units` marks the Stale units `claimable` once their dependencies are built |

## Common Issues

- **`change apply` refuses: `version` or `updated` does not match.** Set both to the last change entry, then run it again.
- **`change apply` refuses: a pin is stale.** Run `vitrify pin sync --sdd <root>`, then run it again.
- **`change apply` refuses: an entry is outside `## Changelog`.** In a single-file SDD, write each entry as `### CH-### — …` under `## Changelog`; a `## CH-###` heading ends that section, so the entry is not read.
- **`[audit 6]` after a change.** A Stale mark has neither a declaring event nor a trace mismatch — usually a status edited by hand — or the journal disagrees with front matter. Undo the hand edit, then run `vitrify check` again; never edit status, traces, the journal, or the ledger by hand.
- **A unit you edited for wording is rebuilt.** The entry was MINOR or MAJOR; an editorial change is PATCH, and `change apply` re-records its traces instead of marking them.
- **Preflight refuses with a trace mismatch.** The edit landed without its entry being applied; write the entry, run `vitrify change apply`, and commit.

## Next Steps

- [Run a Build](run-a-build.md) — rebuild what the change staled.
- [Work With Several SDDs](multiple-sdds.md) — carry a producer's change to its consumers.
- [Build State](../about/concepts/build-state.md) — how traces, marks, and the journal fit together.
