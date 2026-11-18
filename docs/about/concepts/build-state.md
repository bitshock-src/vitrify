# Build State

> Understand where vitrify keeps the state of a build — unit front matter, claims and leases, input traces, the journal, and the ledger — and why mainline is the only authority.

Use this page to learn what vitrify records, where, and who writes it. The **state of the build** is the units' front matter as committed on mainline. Claims, stamps, and journal events land only through mainline commits, so a build can be stopped and resumed from the repository alone: resume is re-derivation from repository state, never recall. For how to read this state while a run is going, see [Monitor a Run](../../user-guides/monitor-a-run.md).

## Why State Lives in the Repository

A build can run for hours across many workers, and processes die. If the state lived in a process or a database, a crash would need a recovery engine. vitrify keeps it in git instead: the history is the repository itself — journal, ledger, and front matter — so recovery is re-derivation rather than replay. Every change of state is a commit on mainline, which makes `git log` the timeline of the build.

## What Build State Is

The state has one authoritative record and several derived or supporting ones. Use this table to see each record, where it lives, and whether vitrify decides anything from it.

| Record | Where | Authoritative? |
| --- | --- | --- |
| Unit front matter | each unit's entry: status, claim fields, stamps, trace | Yes, on mainline |
| Journal | `<sdd-root>/journal.ndjson`, or `## Journal` in a single file | The history; its replay must agree with front matter |
| Ledger | `<sdd-root>/ledger.md`, or `## State Ledger` in a single file | No — derived from front matter |
| Run record | `.vitrify/run.json` | No — a snapshot for people and scripts |
| Worker logs, event log, gate progress and attempt branches | `.vitrify/logs/<job>/` (with `events.jsonl` and `gate.json`), `sdd/<sdd>/<unit>/c<seq>` | No — evidence |

## Claims and Leases

A **claim** is how one holder gets a unit to itself. Each claim, renewal, and release is one atomic mainline commit — the front-matter edit, the journal event, and the ledger regeneration together — published without force.

- **Claim** sets `status: Claimed`, `claimed_by` (the holder), and `claim_expires` (now plus the lease, 60 minutes by default), and increments `claim_seq`.
- **Renew** extends `claim_expires` while the work continues. Under `vitrify run`, a monitor renews each worker's lease when less than a quarter of it remains, and work waiting for the gate keeps its lease alive.
- **Release** returns the unit to Ready, clears the holder and expiry, and keeps `claim_seq`.

Lease expiry is advisory: clocks drift, and a lapsed claim simply makes the unit claimable again. The correctness mechanism is the **fencing token**, `claim_seq`, which every claim increments and nothing resets. It is carried in the work branch name, `sdd/<sdd>/<unit>/c<seq>`, and the integration gate rejects work whose token is not the unit's current one — so a delayed worker whose claim lapsed and was re-claimed can never land its work, even if it is green. `vitrify audit --fix-leases` returns lapsed claims to Ready explicitly.

## Input Traces and Staleness

A **trace** is how vitrify detects that a unit's inputs changed. When a unit is stamped Built, and again when it is stamped Verified, its `trace:` front matter records one hash per input — the first 12 hex characters of a SHA-256:

```yaml
trace:
  unit: 4f2a91c03d7e     # the unit's entry, volatile fields masked
  C-001: 9b11d2e07c44    # each consumed contract
  C-002: 77aa01f3b9c2
  global: 5e30cc18aa09   # Ground Rules + Technology Decisions
```

The keys are exactly the unit's reading set, so "what a unit is built from" and "what can stale it" are the same list. A unit is **Stale** when any recorded hash differs from the current content's hash, and the mismatched key names the cause, as in `stale because C-002 changed`. vitrify compares traces in four places:

- `vitrify trace verify --unit U-###` — one unit, on demand; exit 3 names the mismatched keys.
- Audit check 5 — every Built and Verified unit.
- The integration gate's input check — a unit's work merges only if its trace matches the head.
- A running `vitrify run` — after every mainline change, units whose traces no longer match are marked Stale and rebuilt.

An editorial change re-records traces without a rebuild: write a PATCH change entry and run `vitrify change apply`, which re-records the traces it moved and keeps each unit's status and stamps. The exact recipe is in the [Data Model and Formats Specification](../../reference/formats.md#4-trace-hash-recipe).

## The Journal

The **journal** is the append-only history of every state transition, one JSON event per line. Its vocabulary is closed — `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, and `audit` — and each event carries its `seq`, time, unit, actor, claim number, and commit. Events are appended only inside commits that land on mainline and are never rewritten. Replaying the journal from empty must reproduce every unit's status and `claim_seq`; `vitrify journal replay` checks that it does, and compares the projection with the front matter.

## The Ledger

The **ledger** is the current table — how much of the spec is built and what is stale — in one read. It is derived from unit front matter and never edited by hand: `vitrify ledger regen` rewrites it, and `vitrify ledger regen --check` reports a ledger that a version bump or a hand edit left behind. Its first line after the header is the progress line, which `vitrify ledger progress` also prints:

```text
Progress: 9/14 Verified · 1 Built · 1 Claimed · 2 Ready · 1 Stale
```

## Who Writes State

Some verbs publish their own mainline commits; others edit only the working tree and leave the commit to you. Check this table before running a state verb by hand.

| Verb | Writes | Commits? |
| --- | --- | --- |
| `vitrify claim --unit U-### --holder ID`, and likewise `renew` and `release` | unit front matter, a journal event, the ledger | Yes — one atomic mainline commit each |
| `vitrify run` | claims, renewals, releases, merges, and integration commits | Yes |
| `vitrify trace record --unit U-### --stamp built` (or `--stamp verified`) | the unit's trace, status, and stamp | No — working tree only |
| `vitrify journal append --event KIND` | one journal event | No — working tree only |
| `vitrify ledger regen` | the ledger | No — working tree only |
| `vitrify audit --fix-leases`, `--apply-stale`, `--journal` | front matter, journal events, the ledger | No — working tree only |
| `vitrify change apply` | re-recorded traces or Stale marks, the entry's journal events, the ledger | No — working tree only |
| `vitrify pin sync` | pin hashes | No — working tree only |

## Limitations

These are the boundaries of what the recorded state tells you.

- `.vitrify/run.json` describes the latest run only and is never authoritative; earlier runs keep their copies under `.vitrify/logs/<job>/`.
- A checkout on another branch, or behind mainline, shows old state. Read mainline itself with `git show main:<sdd-root>/ledger.md`.
- Clocks are advisory. A lease that expires mid-gate does not by itself reject work; the fencing token decides.
- The journal vocabulary is closed. Escalation verdicts live in the run record and logs; only their effects append journal events.

## Practical Guidance

- Never hand-edit a status, stamp, trace, journal line, or ledger; use the state verbs and commit their output.
- Read state from the primary checkout when it follows mainline, or from mainline itself with `git show`.
- Record a change entry's bookkeeping with `vitrify change apply`, not by hand.
- Run `vitrify check` after any state change made outside `vitrify run`; it includes `vitrify journal replay` and `vitrify ledger regen --check`.
- Pair every `trace record` of an authored baseline with the matching `journal append` and a `ledger regen`, then commit.
- Treat a lapsed lease as harmless: the next claim supersedes it, and the fencing token keeps the old holder's work out.
