# Build State

> Understand where vitrify keeps the state of a build — unit front matter, claims and leases, input traces, anchors, the journal, and the ledger — what a change redoes, what it keeps, and why mainline is the only authority.

Use this page to learn what vitrify records, where, and who writes it. The **state of the build** is the units' front matter as committed on mainline. Claims, stamps, and journal events land only through mainline commits, so a build can be stopped and resumed from the repository alone: resume is re-derivation from repository state, never recall. For how to read this state while a run is going, see [Monitor a Run](../../user-guides/monitor-a-run.md).

## Why State Lives in the Repository

A build can run for hours across many workers, and processes die. If the state lived in a process or a database, a crash would need a recovery engine. vitrify keeps it in git instead: the history is the repository itself — journal, ledger, and front matter — so recovery is re-derivation rather than replay. Every change of state is a commit on mainline, which makes `git log` the timeline of the build.

## What Build State Is

The state has one authoritative record and several derived or supporting ones. Use this table to see each record, where it lives, and whether vitrify decides anything from it.

| Record | Where | Authoritative? |
| --- | --- | --- |
| Unit front matter | each unit's entry: status, claim fields, stamps, trace | Yes, on mainline |
| Journal | `<sdd-root>/journal.ndjson`, or `## Journal` in a single file | The history; its replay must agree with front matter; its `verified` events fix each unit's anchor |
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

## The Two Cascades

A change can force two different kinds of redo, and vitrify asks both questions about every unit after every change. The first is the trace mechanism above; the second is the re-check. Use this table to see how they differ.

| | **Regenerate** (a model call) | **Re-check** (gates, no model call) |
| --- | --- | --- |
| Depends on | The unit's reading set: its own entry, the contracts it consumes (pins included), Ground Rules and Technology Decisions | The code it is checked against (below) |
| A change is seen when | the unit's recorded trace stops matching | a commit since the unit's **anchor** changed one of those paths |
| Crosses SDDs through | pin hashes: the authority changes, `pin sync` moves the pin, the consumer's change entry marks it Stale | `depends_on` through pins |

The pairing is the one build systems make. In a C build, editing a header recompiles every file that includes it, while changing a library's implementation only relinks and tests its dependents again [mokhov]. Here a model is the compiler: an edited contract regenerates its consumers, and changed upstream code re-checks its dependents.

### The Anchor

A unit's **anchor** is the commit of its latest `verified` journal event: its integration, or its latest re-check. It says where the unit's checks last passed, and every question about whether its code inputs changed is asked of the commits since it. A Verified unit with no anchor — a hand-edited status, or a `verified` event without a commit — is dirty, because nothing recorded vouches for it.

### What a Unit Is Checked Against

A Verified unit is **dirty** when a commit since its anchor changed a path in this list:

- the files it owns, and the files of every unit it depends on, transitively — locally, and across SDDs through pins;
- its SDD's `unmanaged:` files, and any path inside the SDD's scope roots that no unit owns;
- paths outside every SDD's scope — or only those its SDD lists in `shared_inputs`, when it lists any. A declared empty list excludes every outside path.

Spec files never dirty a re-check: a path under an SDD's own root is an input of the regeneration cascade only. A change therefore marks every transitive dependent of the units whose files it touched, and no other unit. The resolver computes each unit's closure once, and `git diff` runs once per distinct anchor, so units integrated in the same commit share one diff.

### Why Upstream Code Never Regenerates a Unit

A unit is generated against frozen contracts, never against its dependencies' implementations; its reading set is its own entry, the contracts it consumes, and the global regions. A change to upstream code leaves every dependent's generation inputs untouched. What it can break is whether the dependent's code still passes its checks against the new upstream code, and only a re-check answers that. If the contract changes, the regeneration cascade takes over.

### Early Cutoff and Fingerprints

A path counts as changed when `git diff <anchor>..<head>` lists it, beyond protocol state: claims, journals, ledgers, and the managed fields of unit entries change no path. Git names a file's content by its hash, so the comparison is of content, not of modification times — the verifying-trace discipline [mokhov], where make compares timestamps. Because the input is every file in the closure, cutoff happens at file identity: a unit regenerated into byte-identical files changes no path, and nothing downstream is re-checked [rustc-incr].

### Where Re-checks Run

Within an SDD, the dirty units' rows run in the integration gate, before the merge; see [Gates](gates.md#the-integration-gate). A run of one SDD also re-checks them at the start, after the lane's §5 Bootstrap, Build and Lint, and the head check that ends a run runs the rows of any unit a change has reached. Across SDDs, a re-check runs after the upstream merge, in the dependent's lane, once nothing the unit depends on has work pending, in one batch with no worker. `vitrify check` lists the dirty units with the paths that made them dirty. A project run's `vitrify run --dry-run` prints the same list under `Re-check`, with the owner of each path.

### Distrusting Everything

`vitrify run --recheck` distrusts every recorded check: every Verified unit's rows, and every SDD's §5 Test command, run again whatever their anchors. It regenerates nothing and still adopts Built attempts, because adoption decides only who wrote the code, never whether it is trusted. Use it after something changed that vitrify cannot see, such as a tool or system dependency outside every declared input.

## Kept Progress

vitrify records every success when it happens and trusts it until its inputs change. Failures are never recorded as results. Use this table to see what is kept, for how long, and what a later run does with it.

| Progress | Recorded as | Trusted until | On a later run |
| --- | --- | --- | --- |
| A unit integrated | its `verified` stamp and journal event, in the integration commit | its trace stops matching (regenerate), or a path it is checked against changes (re-check) | nothing runs |
| A unit re-checked | a `verified` event with actor `recheck`, and a fresh `verified:` stamp | as above | nothing runs |
| An SDD's suite | `suite: pass` on the `audit` event | a path any of its units is checked against changes | the Test command does not run again |
| An attempt Built but never landed | its branch: the Built stamp and the recorded trace | its trace stops matching the reading set | **adopted** |
| An attempt that failed its own gate | its branch | — | the next worker starts from its files |
| A failure | nothing | — | its rows run again; while they fail, its dependents wait |

A re-check that passes is published as one mainline commit, `sdd: re-check U-001, U-002 at <sha>`, which appends a `verified` event for each unit it covered, carrying the checked commit as the unit's new anchor, and `suite: pass` on the `audit` event when the Test command passed. The event type is `verified`: replay sets the status to Verified, which it already is. The `verified:` stamp is a volatile field, masked from traces, so restamping moves no trace. A run killed mid-batch loses only the batch in flight; everything published before it stands.

**Adoption.** An attempt is adopted when its worker is known to be gone — a run set the attempt aside, as it ended or when the next run recovered one that died — its branch carries a Built stamp, and its recorded trace matches the current reading set. An attempt whose lease merely lapsed may still have a worker writing, so it goes to a worker. The scheduler then claims the unit, which takes a new fencing token, re-labels the attempt's work as that claim's branch, and sends it straight to the gate with no worker session. Adoption skips the model, never the gate: the old token can never land, and the adopted work passes every check a fresh attempt would. An attempt the gate or its worker already judged — set aside as `rejected-gate`, `rejected-write-set`, `ownership-conflict`, `blocked`, or `released` — is not adopted, and nor is one that changed paths outside the unit's write-set; those go to a worker.

**Failures stay local.** A unit that is blocked, set aside, or rotten never counts as built, so the units that depend on it, in any SDD, are neither claimed nor re-checked while it fails. Everything else keeps building and landing. Rotten units and how they are traced to a culprit are in [Gates](gates.md#rotten-units-and-culprits).

## The Journal

The **journal** is the append-only history of every state transition, one JSON event per line. Its vocabulary is closed — `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, and `audit` — and each event carries its `seq`, time, unit, actor, claim number, and commit. A `verified` event whose actor is `recheck` records a re-check rather than an integration, and an `audit` event may carry `suite: pass`, recording that the SDD's §5 Test command passed at its commit. Events are appended only inside commits that land on mainline and are never rewritten. Replaying the journal from empty must reproduce every unit's status and `claim_seq`; `vitrify journal replay` checks that it does, and compares the projection with the front matter.

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
- Let a run re-check dirty units rather than re-running every row by hand; `vitrify run --recheck` is the deliberate way to distrust the record.
- Run `vitrify check` after any state change made outside `vitrify run`; it includes `vitrify journal replay` and `vitrify ledger regen --check`.
- Pair every `trace record` of an authored baseline with the matching `journal append` and a `ledger regen`, then commit.
- Treat a lapsed lease as harmless: the next claim supersedes it, and the fencing token keeps the old holder's work out.
