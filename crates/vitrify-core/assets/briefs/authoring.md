# Authoring — from a PRD to a buildable spec

Take a Product Requirements Document (PRD) to an Approved TDD and an Active,
lint-clean SDD. The PRD is the human's: a file, or a few lines in the
prompt, however short. Authoring is the expensive step *by design*: spend
the deep reasoning here so building is cheap and mechanical. Splitting and
sizing happen here, at plan time, fixed in a reviewed artifact and
validated by `vitrify lint` — never at dispatch time.

Use when: a new scope needs its TDD and SDD — a new project, or a feature
outside every existing SDD's `scope_roots`.
Not for: a change inside an existing SDD's scope (`vitrify brief change`),
a whole repository to ready-to-build (`vitrify brief onboard`), or building
(`vitrify brief run`).
Version: vitrify {version} — every command below is this binary's.

## 1. Start from the PRD

**Route.** Run `vitrify locate` (`vitrify locate --json` adds each SDD's
`scope_roots`). A PRD for a feature inside an existing SDD's scope becomes a
TDD revision plus a change entry that adds units: interview the human for
the revision as below, then follow `vitrify brief change`. A small fix
inside an SDD is a change entry, not a new TDD. A PRD for a new scope
becomes a new TDD and SDD, with its own `sdd:` slug and disjoint
`scope_roots`: go on.

**Interview.** Read the PRD against the TDD schema
(`vitrify schema show --tdd`), then ask the human in rounds of **at most five
targeted questions**, each aimed at a TDD section (§1–§6) that the PRD
leaves open. Write each answer into the TDD, in the section it decides. Record
what is still unknown as an open question in the TDD — never a guess.
Repeat until §1–§6 can be written. The human approves the TDD, or its
revision; you do not.

## 2. Write the TDD — the design authority

Materialize the template (`vitrify new tdd`, lands `docs/tdd/<scope>.md` or
any predictable path) and fill §1–§6 from the interview: problem/goals/non-goals,
a component diagram with **every cross-component edge named** (each edge
becomes exactly one SDD contract), exact schemas and algorithms,
cross-cutting constraints with numeric thresholds, rollout ordering, and
named rejected alternatives. The human drives it to **Approved** with a
pinnable revision identifier.

**Gate**: the SDD-readiness checklist at the end of the TDD schema
(`vitrify schema show --tdd`), line by line, pass/fail. A TDD that fails it
will stall SDD authoring with open questions — stop and sharpen the failing
lines rather than carrying them forward.

## 3. Author the SDD — the buildable spec

Pick the shape: a single file `specs/<scope>.sdd.md` (`vitrify new sdd
--single`) when ≤ ~8 units and one implementer at a time is expected;
otherwise a bundle `specs/<scope>/` (`vitrify new sdd --bundle`) — which is
what lets parallel workers claim and stamp units without touching the same
file. The normative rules live in the SDD schema (`vitrify schema show`);
the SDD's front matter declares `schema_version: 2.0`.

* Pin Technology Decisions exactly — language, runtime, every dependency
  version — and write the four **Commands** (bootstrap/build/test/lint).
  Bootstrap, Build and Lint must run clean from the repo root before anything
  is built; kickoff refuses to start otherwise, and the integration gate
  keeps all three green. Test is the whole suite. It need not pass before
  the build — but once it passes on mainline no integration may turn it
  red, and the build is Complete only when it passes. So make it a command a
  correct build keeps green: a test whose subject is not built yet must
  skip (or not be collected) there, however strict the Verification rows
  that demand it are. Write `none` when no one command behaves that way, or
  there is no test command.
* Make ownership **exclusive and exhaustive** over `scope_roots` — over what
  git would commit; ignored files need no owner. Freeze every cross-unit
  surface in exactly one contract *before* its consumers go Ready.
* vitrify never reads source code, so it cannot see code that crosses a
  unit boundary without a contract. If the language has a boundary checker,
  pin it in Technology Decisions and run it as the Lint command or a
  Verification row.
* Give every unit EARS requirements, a decision-complete Implementation
  block (exact paths, signatures, algorithm steps, prohibitions), and a
  Verification table mapping every requirement to an exact command. A
  sentence that leaves the implementer a choice is a defect — decide it or
  open a `Q-###`; **stop at `Q-###` rather than guess**.
* Every unit must be buildable from its reading set alone (Ground Rules,
  Technology Decisions, its own entry, its consumed contracts — nothing
  else); check sizes with `vitrify readset --unit U-### --budget`. The
  budget counts what you wrote: the fields vitrify fills in later (claim,
  stamps, trace) are left out, so building a unit never pushes it over.
* Several SDDs may share a repository: give each its own `sdd:` slug (it
  names their work branches and worktrees) and disjoint `scope_roots`, and
  pass `--sdd <root>` to every verb (`check`, `lint`, `audit`, `ledger`,
  `pin`, `questions` and `journal replay` also take `--all`).
* A unit consumes only contracts of its own SDD. A surface another SDD of
  the repository owns is consumed through a **pin**: a contract of *this*
  SDD whose header is your own, `- Producer: external — <slug>, C-###` and
  `- Pin: <slug> C-### @ <hash12>`, and no body (SDD schema §7, External
  contracts). Write the header, then run `vitrify pin sync --sdd <root>`,
  which records the hash. `vitrify pin list` shows each pin and whether it
  still matches its authority.
  A surface no SDD owns can only be **transcribed**: say so on the Producer
  line, name no contract id, and write the body yourself — it is listed as
  unchecked, because it is.
* Keep each contract's `Consumers:` line true to the units' `consumes`
  lists, and `version` / `updated` true to the last change entry.
* Order the Execution Order foundation-first, with `[P]` groups of mutually
  independent units.
* Every consequential decision cites `TDD §n` or records a `D-###`.

**Gate**: before setting `status: Active`, `vitrify lint` passes — the
mechanical subset of the authoring checklist (L1–L11): acyclic dependencies,
exclusive/exhaustive ownership, independent parallel groups, contract
reference integrity (Consumers lines included), reading-set budgets,
requirement coverage, pre-build state sanity, no `OPEN(Q-###)` in Ready
regions and none without its Decision Log entry, external contracts in
step with their authorities, `version` and `updated` matching the last
change entry, and the TDD pin (the path resolves; Active needs the
current revision of an Approved TDD).

## 4. Decide the test posture

Decide, per unit, how it is verified: whether tests exist before the build,
whether its Verification runs them, and whether building the unit includes
writing tests. There is no default, and postures mix across units. Encode
the decision in the spec itself — ownership and Verification are what the
gates enforce (SDD schema §10, Test posture):

| Posture | Encode it as |
| --- | --- |
| No tests | Verification rows that are not test runs: build output, a smoke run, file or format checks |
| The unit writes its tests | the unit owns its test files; its Implementation names each test and what it asserts; its Verification runs them |
| A test unit writes them first | a test unit owns and prescribes them, consumes the contracts they exercise, and is verified by checks that hold before and after the code exists (the tests load; lint is clean); implementation units `depends_on` it and run its tests |
| Tests written now, governed by the SDD | a baseline unit owns them, consumes the contracts they exercise, and ships `Verified` (below); implementation units run them |
| Existing tests outside the SDD | they stay outside `scope_roots`; Verification runs them; no unit can change them |
| Existing tests the SDD does not use | nothing — keep them out of the Test command too, and no gate runs them, so the build can break them unnoticed; reference them in Verification, or let Test run them, to make them count |

Then:

1. State the posture in Ground Rules — it is in every unit's reading set.
2. Make absence fail: a test that skips when the code it judges is missing
   lets a unit pass by not existing, so Verification runs tests in a mode
   where a missing subject is a failure.
3. Give each tests-first unit the instruction in its Implementation: run the
   Verification command, observe it fail, implement, observe it pass.
4. Route tests through contracts: a unit whose tests import another unit's
   module consumes a contract naming that module, which also stales the
   tests when it changes. Only a boundary checker the SDD runs can verify
   this; vitrify does not read code.
5. Keep Verified checks passing: a Verified unit's Verification commands
   re-run at every preflight and every later integration, so they must hold
   in every later state — a baseline's from before the code its tests judge
   exists. The Test command is held to the same standard from the moment it
   first passes: Verification rows pick tests one by one and cannot see a
   suite that no longer collects, or another unit's test that broke.
6. Make every check stand alone: the gate runs Verification rows side by
   side — one per CPU core unless told otherwise — so a row relies on
   nothing another row leaves behind and changes nothing another reads. If
   the checks cannot be made independent, say so in Ground Rules: the build
   is then run with `--gate-jobs 1`.
7. Gitignore build and test byproducts: the handoff commits every change git
   does not ignore, and the write-set check rejects any file the unit does
   not own — `unmanaged` paths included.
8. Tests written now and governed by the SDD: ship their unit `Verified` as
   the authored baseline — `vitrify trace record --unit U-001 --stamp built`,
   then `--stamp verified`, the matching `vitrify journal append` events,
   `vitrify ledger regen`, commit. A Ready or Stale unit is claimable, so a
   test unit left unstamped — or staled by a later change entry — is rebuilt
   by an implementer unless you re-stamp it first.

**Gate**: `vitrify lint` passes (L6: every requirement has a check); every
test written now fails without the code it judges, run the way the
implementation units' Verification runs it, while its baseline unit's own
checks pass; after stamping a baseline, `vitrify journal replay` exits 0.

## 5. Preflight

Commit the spec (and any authored baseline) at the mainline head, then run
the same preflight a build enforces: `vitrify check` exits 0; the
Bootstrap, Build and Lint commands exit 0; `vitrify locate --units` shows
exactly the Phase-1 units claimable; `vitrify run --dry-run` exits 0. A
`preflight failed:` line or an error whose fix names `vitrify init` or
`vitrify probe` is machine setup, done by the human: name it as a gap rather
than run it.

## 6. Report

End with exactly one of: **Ready to build** (quote the evidence: lint pass,
`vitrify check` clean, the claimable Phase-1 units, the dry run's `Start:`
line; then hand off to `vitrify brief run`) or **Gaps named** (per gap: the
step, the failing check or command with its output, and whether it needs
the human — design authority goes through the TDD, machine setup is
`vitrify init` and `vitrify probe`).
