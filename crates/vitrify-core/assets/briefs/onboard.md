# Onboard — project onboarding

Take a repository from nothing to **ready to build**: an Approved TDD, an
Active decision-complete SDD with its test posture encoded (tests written
now, written during the build, or none), committed at the mainline head,
and a clean preflight — or a report naming exactly which gaps remain.

Use when: a repository has no spec yet, or a spec that has never been built,
and the goal is a build that can start.
Not for: adding a feature to an SDD that exists (`vitrify brief change`),
a new scope beside SDDs that exist (`vitrify brief authoring`), or starting
the build (`vitrify brief run`).
Version: vitrify {version} — every command below is this binary's.

Prerequisites: `git` on PATH, the `vitrify` binary, and a machine the
human has set up with `vitrify init` — machine setup, done once per
machine, not a step of this procedure or of any repository.

Each step ends with a pass/fail condition. Do not advance past a failing
condition — fix it or record it as a named gap for the final report. Never
improvise around a gap that needs design authority: that is TDD work; **stop
at `Q-###` rather than guess**.

## 1. Author the spec

Follow the authoring procedure — `vitrify brief authoring` — end to end:
the PRD routed and the human interviewed, TDD to Approved, SDD to Active
(gated by `vitrify lint`), and the test posture decided and encoded — with
any tests written now checked in and their unit stamped as the authored
baseline.

## 2. Commit

Claims are mainline transactions: commit the SDD, and any authored
baseline, at the mainline head before any build work starts.

**Gate**: no uncommitted changes under the SDD's root.

## 3. Preflight — the same gates a build enforces

* `vitrify check` exits 0 (`--sdd <root>` or `--all` with several SDDs):
  lint, the audit, the journal replay, the ledger and the pins in one call.
  Each finding names its fix.
* The SDD §5 Bootstrap, Build and Lint commands exit 0 from the repo root.
  Test does not gate the start: whether tests exist or pass before the build
  is the SDD's test posture. Once the build is under way it must not go from
  passing to failing, and it must pass at the end.
* `vitrify locate --units` shows exactly the Phase-1 units `claimable`.
* `vitrify run --dry-run` exits 0 and ends with its `Start:` line. A
  `preflight failed:` line or an error whose fix names `vitrify init` or
  `vitrify probe` is machine setup: name it as a gap for the human rather
  than run it.

## 4. Report

End with exactly one of:

* **Ready to build** — quote the evidence: `vitrify check` clean, the
  claimable Phase-1 units, the dry run's `Start:` line.
  Then hand off: `vitrify brief run` (operate `vitrify run`) or
  `vitrify brief kickoff` (drive the loop from a session).
* **Gaps named** — per gap: the step, the failing check or command with its
  output, and whether it needs the human (design authority → TDD; machine
  setup → `vitrify init` or `vitrify probe`) or is mechanical (fix and re-run
  the gate).
