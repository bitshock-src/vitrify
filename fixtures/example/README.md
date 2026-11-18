# example/ — a complete, verified SDD project

This directory is a full worked instance of the SDD lifecycle on a deliberately small scope:
**`calc`**, a zero-dependency command-line evaluator for integer arithmetic expressions that
returns exact rational results (`1/3+1/6` → `1/2`, never `0.49999…`).

It contains everything that exists **before** a code-generation pass starts — and nothing that the
pass is supposed to produce:

| Path | What it is |
| --- | --- |
| `tdd/calc.md` | the Approved TDD (rev 1.0) — the *why*; written to [`tdd-schema.md`](../../crates/vitrify-core/assets/schemas/tdd-schema.md) |
| `specs/calc/` | the Active SDD bundle @ 1.0.0 — the *what and exactly how*; written to [`sdd-schema.md`](../../crates/vitrify-core/assets/schemas/sdd-schema.md): `sdd.md` (core regions), `contracts/C-001…C-004`, `units/U-001…U-005`, `decisions.md`, `changelog.md`, plus the live state artifacts `ledger.md` (derived) and `journal.ndjson` (append-only) |
| `calc/__init__.py` | the only shipped production file — the package root (owned by U-001) |
| `tests/` | **the acceptance suite, written before the build** (owned by U-001; see below) |
| `reference/` | a test-side convenience, not a project file: the known-correct implementation the tier-2 battery's stub worker stands in with (see [Verification status](#verification-status)) |

This tree doubles as vitrify's primary fixture: it is the instance the golden corpus
(`../golden/calc/`) is generated from, and the trace hashes are computed over the bytes under
`specs/calc/` — a change to either moves both, in one reviewed change. Treat the whole directory
as read-only; to play with it, copy it out (below).

Five work units. U-001 (scaffold + suite) ships `Verified` — it is the authored baseline. The
generation pass builds the other four: `U-002` errors → `U-003` tokenizer ∥ `U-004` evaluator
(parallel group A) → `U-005` CLI.

```
$ vitrify locate --units
U-001    Verified  group=-   seq=1
U-002    Ready     group=-   seq=0 claimable
U-003    Ready     group=A   seq=0
U-004    Ready     group=A   seq=0
U-005    Ready     group=-   seq=0
```

## Test posture: acceptance tests written before the build

vitrify supports any test posture — no tests, units that write their own, a test unit built first,
existing tests outside the SDD ([Test Posture](../../docs/about/concepts/test-posture.md);
schema §10, Test posture). calc uses one of them: **the tests that prove the generated code
correct are written at authoring time, checked in, and locked before any implementation
exists.** The suite is the specification's own evidence, not the implementer's homework.

How the lock works:

1. **Ownership.** Every file under `tests/` is owned by U-001 (`sdd.md` §6), which ships
   `Verified`. No implementation unit owns anything under `tests/`, and the write-set law
   (schema §13.3) forbids a unit from writing outside its owned paths.
2. **The write-set check enforces it** (schema §10). Before merging a unit's work, the
   integration gate rejects every changed path the unit does not own — and any edit to its own
   spec text beyond the `status`, `built` and `trace` fields stamping writes. A change set that
   "fixes" a test, or rewrites its unit's Verification table, to make an implementation pass
   cannot land. (calc's `sdd.md` §10 states this check more loosely, allowing any edit to the
   unit's own file; vitrify applies the schema's stricter rule.)
3. **Strict mode closes the absence loophole.** By default an unbuilt module's acceptance tests
   report as *skipped*, so the §5 `Test` command runs green in the shipped state and in every
   state the build passes through. That is what lets it gate: vitrify's preflight does not run
   it, but the integration gate runs the whole suite after every merge and rejects work that
   turns it red, and the build is Complete only when it passes. Every generated unit's
   Verification table instead runs with `SDD_REQUIRE_IMPL=1`, where an absent module is a
   **failure** — a unit cannot pass its gate by not existing. A present-but-broken module fails
   in both modes.
4. **Tests precede implementation** (`sdd.md` §10): each unit's Implementation block tells the
   worker to run its Verification command, observe it fail, implement, and observe it pass.
   Nothing records or checks the failing run.
5. **Staleness binds the suite to the contracts.** U-001 `consumes` C-001…C-004, so its recorded
   input-trace hashes cover every contract: editing a frozen contract mechanically marks the
   suite's unit (and every consumer unit) `Stale` at the next audit (schema §11). A Stale unit is
   claimable, so to keep the suite author-written, update and re-stamp U-001 yourself before the
   next build; otherwise `vitrify run` dispatches a worker to rebuild it.

The suite also demonstrates contract-first decoupling: `tests/test_evaluator.py` builds tokens *by
shape* (C-002's structural-typing rule, TDD §3.1) and never imports the tokenizer — which is what
lets U-003 and U-004 build and gate **in parallel**.

In the shipped state the suite behaves like this:

```
$ python3 -m unittest discover -s tests -t .          # §5 Test — lenient
Ran 10 tests … OK (skipped=4)                         # 4 acceptance modules await their units

$ SDD_REQUIRE_IMPL=1 python3 -m unittest discover -s tests -t .
Ran 10 tests … FAILED (errors=4)                      # strict: absence is failure
```

After a correct generation pass, the same strict command reports `Ran 42 tests … OK` — zero
skips, zero failures. That transition *is* the proof.

## Run it yourself

Prerequisites: `git`, the `vitrify` binary, and — for the calc project itself — `python3` ≥ 3.10
and a POSIX shell (the project's §5 Commands run its own zero-dependency test suite). To build
it, the machine must be set up with `vitrify init`, and a harness CLI recorded by its probe — a
one-time setup of the machine, not of this repository
([Set Up Your Machine](../../docs/getting-started/quick-start.md#set-up-your-machine)).

The example must run from its **own repository** (every verb resolves paths against the enclosing
git repo), so copy it out first:

```bash
cp -R fixtures/example ~/calc-demo && cd ~/calc-demo
rm -rf reference                           # test-side convenience, not a project file
git init -b main && git add -A
git commit -m "calc: authored baseline (SDD 1.0.0)"
```

### Build it

Unattended, with the deterministic orchestrator (uses whichever harness CLI the probe
recorded in `~/.vitrify/config.toml`):

```bash
vitrify run
```

Or have your coding agent operate the run: from a session of your harness in `~/calc-demo`, send:

```text
Use vitrify cli to build the spec.
```

Or interactively from whichever harness you use — the agent follows the kickoff procedure,
which drives the build loop from the session instead of `vitrify run`:

- **Claude Code** — `claude "Use vitrify cli to build the spec from this session, without vitrify run."`
- **Codex** — `codex "Use vitrify cli to build the spec from this session, without vitrify run."`
- **OpenCode** — `opencode run "Use vitrify cli to build the spec from this session, without vitrify run."`

Whichever way it starts, the job prints its header first — done-condition, plan (`Phase 1: U-002 ·
Phase 2: group A — [P] U-003, U-004 · Phase 3: U-005`), worker cap, lease — then claims one
worker per claimable unit and integrates serially. It terminates **Complete** when the evidence
holds (every unit Verified, audit clean, replay consistent, the §5 Test command passing), or
**Blocked** with every blocker named.

## Drift detection, demonstrated

Touch a frozen contract and every stamped consumer goes stale, with the cause named:

```
$ echo "<!-- drift -->" >> specs/calc/contracts/C-002-token-stream.md
$ vitrify audit ; echo exit=$?
[check 5] U-001: recorded trace differs from current content: C-002 → stale because C-002 changed: write the change entry (§12), then vitrify change apply --sdd specs/calc
exit=2
$ git checkout -- specs/calc/contracts/C-002-token-stream.md    # restore → audit clean again
```

In the shipped state only U-001 is stamped, so one finding; after a full build the same edit
flags U-001, U-003, U-004, and U-005 — every consumer of C-002. (U-002 stays green: it does not
consume C-002. The reading set and the staleness set are the same list by construction —
schema §11.)

## How the shipped baseline was produced

U-001's `Verified` state was generated with vitrify's own state verbs — never by hand
(schema §11; the stamps record the placeholder commit `0000000` per `decisions.md` D-002):

```bash
vitrify journal append --event change  --entry CH-001 --actor author --commit 0000000
vitrify journal append --event claim   --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify trace record --unit U-001 --stamp built    --commit 0000000
vitrify journal append --event built   --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify trace record --unit U-001 --stamp verified --commit 0000000
vitrify journal append --event verified --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify ledger regen
```

If you edit any spec text (Ground Rules, Technology Decisions, a contract, or U-001's own entry),
U-001's trace will mismatch and the audit will flag it. Write the change entry (schema §12) and
run `vitrify change apply`: under a PATCH entry it re-records U-001's trace; under a MINOR or
MAJOR entry it marks U-001 Stale, for the next build to rebuild. `vitrify check` then says the
repository is consistent.

## Verification status

vitrify's tier-2 end-to-end battery builds this example through `vitrify run` on every CI run,
with no harness and no tokens: the deterministic stub worker stands in for the implementer, with
`reference/` as its canned solution. Run it from a checkout of this repository:

```bash
cargo test -p vitrify-cli --test e2e -- --ignored
```

It resets the protocol fields on a scratch copy, drives `vitrify run` to **Complete** with the
evidence triple, asserts zero spec-body edits in either direction, and rehearses the
kill-a-worker fencing case (a lapsed claim's green branch must be rejected at the gate).
[Develop vitrify](../../docs/contribute/development.md#build-the-calc-example-without-a-model)
covers it with the other test tiers.

- **Shipped state** — held by the golden corpus (`../golden/calc/`) and the tier-1
  suites on every CI run: locate/readset JSON byte equality, journal replay, ledger bytes
  (date-normalized), audit clean (all seven checks), and every recorded trace verifying clean
  under the hashing recipe.
- **Generation pass** — driven end to end by the tier-2 battery on every CI run (see above):
  claims with leases and fencing tokens as atomic mainline commits, isolated worktrees on
  `sdd/calc/<unit>/c<seq>`, Built stamps with traces, fenced serial integration with write-set
  checks, `--no-ff` merges and the full integration gate, Verified stamping, journal events,
  ledger regeneration, the drift audit, and compare-and-swap publication — terminating
  `Progress: 5/5 Verified`, audit clean, replay consistent. The stub worker copies the reference
  solution and stamps it Built, so the worker-side steps — observing tests fail first, running
  the unit gate — are not exercised.
- **Kill-a-worker rehearsal** — same battery: a built branch whose claim lapsed and was
  re-claimed is rejected at the integration gate even though it is green; the re-claimed build
  lands with the superseding fencing token.
