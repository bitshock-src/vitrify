# vitrify Primer

> Learn the vitrify model in five minutes: the spec you write, the units it splits into, and how a run turns them into verified code.

Use this page before the concept pages, or instead of them if you only need to run a build. It gives the minimum model needed to read the rest of the documentation, follows one build from both sides, and says what vitrify leaves to you. Each term links to the page that explains it in full, and the [Glossary](../resources/glossary.md) defines them all. A coding agent learns vitrify from the binary itself: `vitrify --help`, then `vitrify brief` for the procedure that fits its task.

## What vitrify Adds

vitrify does not write code or decide what to build. It manages the building of a spec you write, and judges the result mechanically. It adds a small set of primitives:

- **A decision-complete spec.** You write a [TDD](concepts/specs.md) — the design and its reasons — and derive an SDD from it that leaves the implementer no choices.
- **Work units with exclusive ownership.** The SDD splits the work into [units](concepts/units.md), each small enough for one agent session and each owning its own files.
- **Frozen contracts.** Every surface two units share is written down once, as a [contract](concepts/contracts.md), before either is built.
- **A test posture per unit.** Each unit is verified the way you choose — tests first, tests written as it builds, existing tests, or none — through the same gates ([Test Posture](concepts/test-posture.md)).
- **Mechanical gates.** A unit's work lands only if the commands the spec names pass on the merged tree and it changed only the files it owns ([Gates](concepts/gates.md)).
- **Build state in git.** Claims, stamps, and an append-only journal are commits on your mainline, so the [state of the build](concepts/build-state.md) is always in the repository.
- **A deterministic orchestrator.** [`vitrify run`](concepts/runs.md) schedules, claims, integrates, and retries in Rust, and invokes an AI coding agent only to build a unit or to judge one that keeps failing.

## One Build, Two Views

The calc example — a small calculator spec with five units — shows the same build from your side and from vitrify's. Its plan:

```text
U-001  scaffold and acceptance tests       (shipped Verified)
  └─► U-002  errors
        ├─► U-003  tokenizer    ┐ parallel group A
        └─► U-004  evaluator    ┘
              └─► U-005  command-line interface
```

**What you see.** You commit the spec, run `vitrify run`, and watch lines go by: units dispatched, units integrated. When it finishes it prints `Complete.` with `Progress: 5/5 Verified`, and the project's own test suite passes.

**What vitrify records and controls.** Underneath, each step is a mechanical check or a commit:

1. **Preflight.** It checks that the SDD is Active and committed, that lint and the audit are clean, and that the project's Bootstrap, Build, and Lint commands pass — then prints the plan.
2. **Claim.** U-002 is the only claimable unit: its dependency, U-001, is already Verified. vitrify commits a claim on mainline that gives U-002 a lease and a new fencing token, `c1`.
3. **Build.** It creates a worktree on branch `sdd/calc/U-002/c1`, outside your checkout, and starts a headless session of your harness CLI with a prompt holding exactly U-002's reading set. The agent writes the files U-002 owns, runs U-002's Verification commands, stamps the unit Built, and ends with a one-line report.
4. **Gate.** vitrify commits the worktree, checks that the token is still current and that only U-002's files changed, merges, confirms that U-002 was built from the current spec text, and runs the gate: build, every Verified unit's checks, lint, and the full test suite as a regression check.
5. **Publish.** One commit stamps U-002 Verified and appends its journal events. Your mainline moves, and your checkout follows.
6. **Parallel.** U-003 and U-004 are now both claimable. Two workers build them side by side, and because every gate re-runs the checks of every Verified unit, a semantic conflict between them is caught before the conflicting work lands.
7. **Finish.** After U-005 nothing is left to claim. vitrify checks the done-condition — every unit Verified, audit clean, journal consistent, test suite passing — and prints `Complete.`

When a unit's work fails, vitrify keeps it on its branch, tells the next attempt why, and after two failures asks a stronger model for a verdict. When the spec leaves a gap, the agent stops at a question rather than guess, and the run ends `Blocked.`, naming it.

## What vitrify Does Not Replace

These responsibilities stay with you and your existing tools.

- **Your design judgment.** The TDD's decisions, the SDD's split into units, and every answer to a question are yours. vitrify checks the plan's structure, not its wisdom.
- **Your harness and its account.** Workers are sessions of Claude Code, Codex, or OpenCode, on your existing login and permission settings. vitrify stores no credentials and calls no model API itself.
- **Your build tools and tests.** vitrify runs the commands your spec names and reads their exit codes. It never reads source code, so it works the same for any language.
- **Your repository hosting.** vitrify publishes to your mainline branch — through `origin` when one exists — and leaves hosting, review, and CI to you.

## Next Steps

- [Quick Start](../getting-started/quick-start.md) — spec and build your own project through your coding agent, or build the calc example yourself.
- [Author a Spec](../user-guides/author-a-spec.md) — write your own TDD and SDD.
- The concept pages, in order: [Specs: TDD and SDD](concepts/specs.md), [Work Units](concepts/units.md), [Contracts](concepts/contracts.md), [Test Posture](concepts/test-posture.md), [Gates](concepts/gates.md), [Build State](concepts/build-state.md), and [Runs](concepts/runs.md).
