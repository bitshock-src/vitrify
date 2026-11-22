# Test Posture

> Choose, per unit, whether tests exist before the build, are written during it, or are not used — and learn the rules every posture shares.

Use this page when you decide how each unit of an SDD is verified. vitrify takes no position on tests. Tests may exist before `vitrify run` or not; an SDD may verify its units with existing tests, have units write tests as they build, or use no tests at all. A unit's **test posture** is that choice, made per unit and encoded in the spec: ownership and Verification tables are the whole mechanism, and there is no posture setting. For the authoring steps, see [Author a Spec](../../user-guides/author-a-spec.md#4-decide-the-test-posture); the normative rules are SDD schema §10, Test posture.

## Why Test Posture Exists

Projects arrive in different states: some have a suite already, some want acceptance tests locked before any code exists, some have nothing to test but a build. A single built-in posture would force every SDD into one of them. Instead, every posture runs through the same fixed gates, and a unit's work may change only the files it owns — tests included — plus its own status stamps. The §5 Test command never has to pass before a build starts; once it does pass, no merge may break it.

## Posture Types

Decide, per unit, whether tests exist before the build, whether its Verification runs them, and whether building the unit includes writing them. Use this table to see how each posture is expressed.

| Posture | Who owns the tests | The unit's Verification runs |
| --- | --- | --- |
| No tests | — | checks that are not test runs: build output, a smoke run, file or format checks |
| The unit writes its tests | the unit itself; its Implementation names each test and what it asserts | the tests it writes |
| A test unit writes them first | a test unit, consuming the contracts its tests exercise, that the implementation units `depends_on` | the test unit's tests |
| Written before the build, governed by the SDD | a unit already `Verified` when the build starts (the authored baseline), consuming the contracts its tests exercise | those tests |
| Written before the build, outside the SDD | nobody — they lie outside `scope_roots` | those tests |
| Existing tests the SDD does not use | whoever owns them — nobody, outside `scope_roots` | nothing — and the Test command leaves them out: then no gate runs them, and the build can break them unnoticed |

Postures mix across units: acceptance tests written before the build can sit alongside unit tests each unit writes.

## Choose a Posture

Use this table to match what you want to a posture.

| You want | Posture |
| --- | --- |
| tests that prove the generated code correct, written and locked before any implementation exists | Written before the build, governed by the SDD (the calc example) |
| each unit to deliver code and its own tests together | The unit writes its tests |
| tests written by one worker and implemented against by others | A test unit writes them first |
| to keep an existing suite as the judge without letting the build edit it | Written before the build, outside the SDD |
| a build with nothing to test but its output | No tests |

## Posture Rules

What vitrify does is the same in every posture. These rules follow from it.

- **Units are judged only by their gates.** Both gates run bootstrap, build, the unit's Verification table and lint; the integration gate adds the write-set check, the input check (the unit was built from the spec text mainline holds now), the table of every Verified unit a change has reached, the §5 Test command as a regression check, and audit checks 1–3 (orphans, ghosts, contract references). See [Gates](gates.md).
- **The whole suite must not go backwards.** Verification rows pick tests one by one; they cannot see a suite that no longer collects, or another unit's test that broke. So after every merge the gate runs the §5 Test command: if it passed on mainline before the merge and fails after, the work is rejected. While it already fails — no tests yet, tests awaiting their code — it judges nothing; and the build is Complete only when it passes. Tests whose subject is not built yet must therefore skip there (as calc's do), or the SDD says `Test: none`.
- **Only a file's owner may change it — tests included.** Before merging, the write-set check rejects any change to a path the unit does not own — another unit's tests, `unmanaged` paths and the SDD's own files included — and any edit to its own spec text beyond the `status`, `built` and `trace` fields that stamping writes. A unit cannot rewrite the tests that judge it unless it owns them, nor its own Verification table at all. The handoff commits every change git does not ignore, so keep build and test byproducts in `.gitignore`.
- **Tests reach other units' code through contracts.** When a test owned by one unit imports a module another unit owns, the test's unit must consume a contract naming that module — which also stales the tests when the contract changes. vitrify never reads code, so nothing in vitrify checks this; pin a boundary checker for your language and run it as the Lint command or a Verification row (SDD schema §5).
- **Ready and Stale units get rebuilt.** Any Ready or Stale unit can be claimed, and its implementer may rewrite whatever the unit owns. Tests that only you may write belong to a unit you ship `Verified` — and re-stamp yourself after a change entry stales it — or outside `scope_roots`.
- **Verified checks keep running.** A Verified unit's Verification runs again whenever a change reaches it — in a re-check at preflight, in a later integration gate, or in a re-check batch after an upstream merge — and in audit check 4 (each distinct command once), so it must pass in every later state — for a baseline, starting before the code its tests judge exists. Verify a test unit or a baseline whose tests judge other units' code with checks that hold both before and after that code exists (the tests load, they lint clean), never with "the tests fail".
- **Tests-first is an instruction, not a gate.** A unit's Implementation may say: run the Verification command, watch it fail, implement. Workers follow it; no gate observes the failing run.

## Make Absence Fail

A Verification command that runs tests should fail when the code it judges is absent: a test that skips when its code is missing lets a unit pass by not existing. At the same time, the §5 Test command must stay green in every state the build passes through, so tests of unbuilt code must skip there. calc resolves the two with one switch:

- By default an unbuilt module's acceptance tests report as *skipped*, so the §5 Test command runs green in the shipped state and in every state the build passes through.
- Every generated unit's Verification table runs with `SDD_REQUIRE_IMPL=1`, where an absent module is a **failure** — a unit cannot pass its gate by not existing. A present-but-broken module fails in both modes.

## Authored Baseline

To lock tests written at authoring time, ship their unit already Verified: the **authored baseline**. Stamp it with vitrify's own state verbs — never by hand — so that its trace, journal, and ledger agree: `vitrify trace record --unit U-001 --stamp built` then `--stamp verified`, the matching `vitrify journal append --event KIND` events, and `vitrify ledger regen`. The exact sequence is in [Author a Spec](../../user-guides/author-a-spec.md#ship-an-authored-baseline).

Because the baseline's unit consumes the contracts its tests exercise, editing a frozen contract stales the baseline along with every consumer. A Stale unit is claimable, so to keep the suite author-written, update and re-stamp the baseline yourself before the next build; otherwise `vitrify run` dispatches a worker to rebuild it.

## Limitations

These are the things vitrify deliberately does not do about tests.

- There is no declared posture field. The posture is already stated by ownership and Verification tables — the things the gates enforce — and a separate field could only drift from them.
- No gate observes tests-first. Proving red-before-green would need a per-row marker, which is a format change; the post-merge green the gate already requires is the evidence that counts.
- The §5 Test command is not a preflight gate. Its result before the build depends entirely on the posture — no tests yet, tests awaiting their code, a runner that fails on an empty suite.
- Existing tests that the Test command leaves out are run by no gate, and the build can break them unnoticed.
- vitrify does not check that tests import other units' code only through contracts; that needs a language-aware boundary checker that you pin.

## Practical Guidance

- State the posture in Ground Rules, so every worker reads it.
- Make absence fail in every Verification row that runs tests.
- Keep the §5 Test command green in every intermediate state, or write `Test: none`.
- Ship author-written tests as a Verified baseline, and re-stamp it yourself after a change entry stales it.
- Verify a test unit or baseline with checks that hold before and after the code exists — the tests load and lint clean — never with "the tests fail".
- Keep build and test byproducts in `.gitignore`; the handoff commits everything git does not ignore.
