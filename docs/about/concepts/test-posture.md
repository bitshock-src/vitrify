# Test Posture

> Choose, per test file, whether tests exist before the build or are written during it, list them, and learn the rules every posture shares.

Use this page when you decide how each unit of an SDD is verified and who writes its tests. vitrify takes no position on tests. Tests may exist before `vitrify run` or not; an SDD may verify its units with existing tests, have units write tests as they build, or use no tests at all. A test's **posture** is the author's choice, encoded in ownership: `tests:` in §6 System Layout & Ownership says which files are tests (schema 2.2), and ownership says who writes them. There is no posture setting. For the authoring steps, see [Author a Spec](../../user-guides/author-a-spec.md#4-decide-the-test-posture); the normative rules are SDD schema §10, Test posture.

## Why Test Posture Exists

Projects arrive in different states: some have a suite already, some want acceptance tests locked before any code exists, some have nothing to test but a build. A single built-in posture would force every SDD into one of them. Instead, every posture runs through the same fixed gates, and a unit's work may change only the files it owns — a test it owns included — plus its own status stamps. The §5 Test command never has to pass before a build starts; once it does pass, no merge may break it.

## Two Postures

Whether a unit is verified by tests, and who writes them, is the author's choice, encoded in ownership. This schema has no default and no posture field: `tests:` (§6) says which files are tests, and ownership says who writes them. A unit whose Verification runs no tests needs neither.

| Posture | Who owns the test files (§6) | Who writes them |
| --- | --- | --- |
| Generated | the unit that writes them; its Implementation names each test and what it asserts | that unit's implementer, and again whenever the unit is rebuilt |
| Pre-existing | no unit | the author, before the build and between builds; no unit's work may change them |

Postures mix freely — a pre-existing acceptance suite beside generated unit tests, for example. The rules below hold in both.

## List Your Tests

From schema 2.2, an SDD lists its test files under `tests:` in §6 System Layout & Ownership, beside `unmanaged:`. Entries are paths or globs inside `scope_roots`, matched as `owns` globs are:

```
| tests/checkout/cart/*         | U-004  | generated with U-004       |
| tests/checkout/acceptance/*   | —      | pre-existing               |

tests:
  - tests/checkout/
```

A test file a unit also owns is a **generated test**, and that unit is its owner. A test file no unit owns is a **pre-existing test**. A `tests:` entry must lie inside `scope_roots` and must not overlap `unmanaged:`; lint L2 reports an entry that does. The list also carries an obligation: a path that another SDD's commands read — a test helper another SDD's tests import — is not a test of this SDD and must not be listed, because a change to a listed path re-checks only its own SDD.

## Choose a Posture

Use this table to match what you want to a posture.

| You want | Posture |
| --- | --- |
| tests locked before any implementation exists | Pre-existing |
| each unit to deliver code and its own tests together | Generated |
| tests written by one worker and implemented against by others | Generated, by a test unit that the implementation units `depends_on` |
| a build with nothing to test but its output | Rows that run no tests: build output, a smoke run, file or format checks |

## Posture Rules

What vitrify does is the same in both postures. These rules follow from it.

- **Units are judged only by their gates.** Both gates run bootstrap, build, the unit's Verification table and lint; the integration gate adds the write-set check, the input check (the unit was built from the spec text mainline holds now), the table of every Verified unit a change has reached, the §5 Test command as a regression check, and audit checks 1–3 (orphans, ghosts, contract references). See [Gates](gates.md).
- **The whole suite must not go backwards.** Verification rows pick tests one by one; they cannot see a suite that no longer collects, or another unit's test that broke. So after every merge the gate runs the §5 Test command: if it passed on mainline before the merge and fails after, the work is rejected. While it already fails — no tests yet, tests awaiting their code — it judges nothing; and the build is Complete only when it passes. Tests whose subject is not built yet must therefore skip there (as calc's do), or the SDD says `Test: none`.
- **Only a test's owner may change it.** The write-set check rejects every other unit's change to a test, and every unit's change to a pre-existing test. Edits made outside the build are caught when they make a Verified unit's checks fail (§11, check 4). The handoff commits every change git does not ignore, so keep build and test byproducts in `.gitignore`.
- **Who runs a test depends on who writes it.** A unit whose Verification runs tests another unit owns should list that unit in `depends_on`: the tests then exist before the rows that run them, and a change to them re-checks the units that run them. Without the edge, such a change reaches those units only through the §5 Test command. Tests that many units run, whatever their dependencies, are better pre-existing: a change to a pre-existing test re-checks every unit of its SDD. A test unit that implementation units depend on is the generated posture.
- **Tests stay in their SDD.** A change to a test path re-checks units of its own SDD only.
- **Tests reach other units' code through contracts.** When a generated test imports a module another unit owns, the test's unit must consume a contract naming that module — which also stales the tests whenever the contract changes. A pre-existing test has no unit: a change entry that changes a contract updates the pre-existing tests that exercise it, in the same change. vitrify never reads code, so nothing in vitrify checks this; pin a boundary checker for your language and run it as the Lint command or a Verification row (SDD schema §5).
- **Ready and Stale units are rebuilt by implementers.** Any Ready or Stale unit can be claimed, and its implementer may rewrite every file the unit owns, generated tests included. Tests that only the author may write are pre-existing.
- **Verified checks keep passing.** A Verified unit's Verification commands run again whenever a change reaches it since its last recorded check — at a build's preflight, at a later integration, in a re-check — at every later integration of an SDD with no §5 Test command, and in the drift audit (audit check 4, each distinct command once), so they must pass in every later state, starting, for tests of code not yet built, before that code exists. A test unit whose tests judge other units' code is therefore verified with checks that hold both before and after that code exists (the tests load; they are lint-clean) — never with "the tests fail".
- **Tests-first is an instruction, not a gate.** A unit's Implementation may say: run the Verification command, watch it fail, implement. Workers follow it; no gate observes the failing run.

## Test Changes Stay in Their SDD

A change to a generated test reaches its owner and the units of the owner's SDD whose `depends_on` closure contains the owner. A change to a pre-existing test reaches every Verified unit of its SDD. Either one also reaches that SDD's §5 Test command. Neither reaches another SDD: nothing outside the SDD runs the test, which is why the list must not name a path that another SDD's commands read.

For example, an `orders` SDD lists `tests/orders/` under `tests:`. U-003 owns `tests/orders/cart/*`, and U-005 depends on U-003. A change to `tests/orders/cart/test_totals.py` re-checks U-003 and U-005 and the `orders` Test command, and no unit of a `billing` SDD, even one that depends on `orders` units. A change to a file under `tests/orders/acceptance/`, owned by no unit, re-checks every Verified unit of `orders` — and still none of `billing`. A change to `orders` code, by contrast, still re-checks its dependents in every SDD.

## Make Absence Fail

A Verification command that runs tests should fail when the code it judges is absent: a test that skips when its code is missing lets a unit pass by not existing. At the same time, the §5 Test command must stay green in every state the build passes through, so tests of unbuilt code must skip there. calc resolves the two with one switch:

- By default an unbuilt module's acceptance tests report as *skipped*, so the §5 Test command runs green in the shipped state and in every state the build passes through.
- Every generated unit's Verification table runs with `SDD_REQUIRE_IMPL=1`, where an absent module is a **failure** — a unit cannot pass its gate by not existing. A present-but-broken module fails in both modes.

## Before Schema 2.2

An SDD below 2.2 has no `tests:` list. It keeps tests from the build either with a unit that is `Verified` when the build starts and owns them — an **authored baseline**, which the author re-stamps after a change entry stales it — or by leaving them outside `scope_roots`. calc is a 2.0 SDD whose U-001 owns its acceptance suite. A `tests:` list in an SDD below 2.2 is a lint L2 finding. To move an SDD to 2.2, see [Migrate an SDD to Schema 2.2](../../user-guides/change-a-spec.md#migrate-an-sdd-to-schema-22).

## Limitations

These are the things vitrify deliberately does not do about tests.

- There is still no posture field. The posture is already stated by ownership and Verification tables — the things the gates enforce — and a separate field could only drift from them.
- vitrify cannot check that no other SDD reads a test path. Listing a path that another SDD's commands read keeps that SDD from being re-checked when the test changes.
- No gate observes tests-first. Proving red-before-green would need a per-row marker, which is a format change; the post-merge green the gate already requires is the evidence that counts.
- The §5 Test command is not a preflight gate. Its result before the build depends entirely on the posture — no tests yet, tests awaiting their code, a runner that fails on an empty suite.
- vitrify does not check that tests import other units' code only through contracts; that needs a language-aware boundary checker that you pin.

## Practical Guidance

- State the posture in Ground Rules, so every worker reads it.
- List every test file under `tests:`, and leave out a path that another SDD's commands read.
- Make absence fail in every Verification row that runs tests.
- Keep the §5 Test command green in every intermediate state, or write `Test: none`.
- Write tests that only you may change as pre-existing tests, and update them yourself in the change that needs it.
- Verify a test unit with checks that hold before and after the code exists — the tests load and lint clean — never with "the tests fail".
- Keep build and test byproducts in `.gitignore`; the handoff commits everything git does not ignore.
