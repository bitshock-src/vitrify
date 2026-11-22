# Change — change TDDs and SDDs, one or many

Change Active SDDs (and the TDD revisions behind them) so that each change
lands as spec text, its change entry and the staleness it causes, in one
commit, with `vitrify check` clean. One SDD or many: the steps are the same,
producers first.

Use when: an Active SDD must change — a contract, a unit's text, a new
feature inside its scope, the answer to a `Q-###` — or a TDD revision must
reach the SDDs that pin it.
Not for: a new scope (`vitrify brief authoring`), building what a change
stales (`vitrify brief run`), or a Draft SDD, which is edited freely.
Version: vitrify {version} — every command below is this binary's.

The spec text is current truth, edited in place; the change entry records
what moved. A change needs design authority: what the human has not decided
is a `Q-###`, never a guess. In a repository with several SDDs every command
takes `--sdd <root>` or `--all`.

## 1. Scope

List what the change touches: the SDDs, their roots and `scope_roots`
(`vitrify locate --json`); the contracts that cross SDDs, and their state
(`vitrify pin list --all`); and the TDD revisions involved. For **more than one SDD**, write the
list of changes first: which SDD gets which entry and bump, producers before consumers.
Check it against those two listings — every SDD named exists, every pin it
expects is listed — before editing anything. Show it to the human when the
change is theirs to approve.

A change that revises the TDD — a PRD for a feature inside an SDD's scope,
a new design decision — starts with the interview of
`vitrify brief authoring` §1: rounds of at most five targeted questions,
until the revision can be written. The human approves the revision before
any SDD text changes; you do not.

## 2. Each SDD, producers first

For each SDD, producers first:

1. Edit the text in place — units, contracts, Ground Rules, Technology
   Decisions. Never leave the old text beside the new.
2. If the TDD gained a revision, re-pin `tdd_version` in the front matter.
3. Write the change entry in the form of SDD schema §12
   (`vitrify schema show`): `## CH-### — <title> (<date>) — <from> → <to>`
   in a bundle's `changelog.md`, or `### CH-### — …` under a single file's
   `## Changelog`, then the TDD pin, Added, Modified, Removed,
   `Staleness applied` and `Codebase delta` lines. Leave
   `Staleness applied` to step 3.
4. Set the front matter's `version` and `updated` to the entry. The bump
   decides the bookkeeping:

   | Bump | When |
   | --- | --- |
   | PATCH | editorial wording that changes no behavior — traces are re-recorded, nothing rebuilds |
   | MINOR | units added, modified or retired — the units whose traces moved go Stale |
   | MAJOR | a contract or a technology decision changed — the units whose traces moved go Stale |

## 3. Record

```
vitrify change apply --sdd <root>
```

It reads the last entry, takes the units whose recorded trace no longer
matches, re-records them (PATCH) or marks them Stale (MINOR, MAJOR), appends
the journal events and regenerates the ledger — in the working tree. Copy
the `- Staleness applied:` line it prints into the entry. `--dry-run` shows
the same without writing. Exit 2 is a refusal that names its fix (`version`
or `updated` not matching the entry; a stale pin → `vitrify pin sync`
first). Running it again is safe: a recorded entry is skipped.

## 4. Across SDDs

Once every producer's entry is written and applied:

```
vitrify pin sync --all
```

It rewrites each stale pin's hash and prints, per consumer SDD, the
`Staleness applied` line that consumer's entry needs; `--dry-run` shows the
same without writing. For each consumer it names: write that consumer's own
change entry around the printed line (its own bump, `version` and
`updated`), then run `vitrify change apply --sdd <consumer>`. An entry never
marks another SDD's units.

## 5. Gate

```
vitrify check --all
```

It must exit 0. Each finding names its fix; fix and re-run. Then run the
project's own checks where the repository documents any (an index of SDD
conventions, scripts beside the specs).

## 6. Migrate an SDD to schema 2.1

Do this once for each SDD whose units consume pins and which declares
`schema_version: 2.0`. A project run (`vitrify brief run`) refuses such an
SDD (exit 4) until it is migrated. Start from a consistent repository:
everything committed, pins synced (`vitrify pin sync --all`), and
`vitrify check --all` clean.

1. Classify each pin each unit consumes (`vitrify pin list --sdd <root>`):
   **code** (the unit imports or links it, installs its package, or runs
   its CLI) or **interface/data** (it only codes against it or parses it).
2. Add each code pin to that unit's `depends_on`; it stays in `consumes`.
   Interface and data pins stay in `consumes` alone.
3. Set `schema_version: 2.1`.
4. Record it all as one **PATCH** change entry, and `vitrify change apply`
   it. `depends_on` is part of each unit's trace, so a MINOR or MAJOR entry
   would mark the units Stale and rebuild them; PATCH re-records the traces
   and rebuilds nothing, since the entry only records a need the built
   code already has.

## 7. What a change regenerates and what it re-checks

* **Regenerated (a model call):** a unit whose spec inputs moved — its
  entry, a contract it consumes (a synced pin included), Ground Rules or
  Technology Decisions. A change entry marks it Stale.
* **Re-checked (no model call):** a Verified unit whose code inputs moved
  since its last check: its own files, the files of the units it depends on
  (through pins too), its SDD's unmanaged files, and paths outside every
  scope (or only its SDD's `shared_inputs`). Another SDD's code change
  therefore re-checks its dependents and regenerates none of them.
* A unit regenerated into identical files changes no path, so nothing
  downstream of it is re-checked.
* `vitrify run --dry-run` prints both, with the changed paths behind each
  re-check.

## 8. Commit and rebuild

Commit everything once, at the mainline head: these verbs edit the working
tree only, and text, entries and marks land together. Then rebuild with
`vitrify brief run`: one plain `vitrify run` builds every SDD, in dependency
order. A build that is going picks the commit up by itself.

## Never

* Hand-edit `status`, `built`/`verified` stamps, `trace`, the journal, the
  ledger or a pin's hash — the verbs above write them.
* Land spec text without its change entry, or an entry without its
  `change apply`.
* Mark one SDD's units from another SDD's entry.
* Decide a design question yourself: record a `Q-###` and hand it back.
* Mark a TDD revision Approved: the human approves it.

## Report

Per SDD: the entry (`CH-###`), its version transition and bump, the
`Staleness applied` line, and the codebase delta. Then the result of
`vitrify check --all` (its last line and exit status), the project's own
checks, the commit, and any `Q-###` handed back.
