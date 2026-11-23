# Gates

> Understand the mechanical checks vitrify applies before, during, and after a build: lint, preflight, the unit gate, the integration gate, and the audit.

Use this page to learn what is checked, when, and what a failure means. A **gate** is a fixed sequence of commands and checks that work must pass to advance; no gate involves judgment. vitrify judges units by the exit codes of the commands the SDD names and by which file paths changed — it never reads source code. For the exact check lists, see [`vitrify lint`](../../cli/command-reference.md#vitrify-lint) and [`vitrify audit`](../../cli/command-reference.md#vitrify-audit); for what a run prints when a gate fails, see [Monitor a Run](../../user-guides/monitor-a-run.md).

## Why Gates Exist

Parallel workers produce work that nobody has read. Gates replace trust with mechanism: a unit lands only if its claim is still current, it changed only files it owns, it was built from the spec text mainline holds now, and the commands the SDD names exit 0 on the merged tree. Because the checks are exact commands and path comparisons, the same work always gets the same verdict.

## Gate Types

vitrify applies five kinds of check at different points. Use this table to see when each runs and what it guards.

| Check | When | Run by | Guards |
| --- | --- | --- | --- |
| Lint (L1–L12) | before an SDD goes Active; at every preflight as `lint --built` | you, `vitrify run`, kickoff | the spec's structure: dependencies, ownership, references, budgets, coverage, versions, the TDD pin, external contracts |
| Preflight | before a run starts | `vitrify run`, kickoff | that the build can start: an Active, committed SDD; lint; audit; the §5 Bootstrap, Build, and Lint commands in a fresh checkout; a working harness; and, in a project run, one acyclic unit graph |
| Unit gate | before a worker stamps its unit Built | the worker, in its worktree | the unit's own work: bootstrap, build, its Verification table, lint |
| Integration gate | before work merges to mainline | the integrator, one gate at a time | the merged tree: fencing, write-set, input check, the merged units' rows and those of dirty Verified units, the Test regression check, audit checks 1–3 |
| Audit (checks 1–7) | at preflight (without check 4: the re-check cascade runs the dirty units' rows instead), in the integration gate (checks 1–3), at the head when a run ends, and on demand | `vitrify audit` | drift between the spec, the files, and the recorded state |

## The Integration Gate

The integration gate is where work lands or is rejected. It runs one gate at a time, in the integration checkout, and takes every unit that is waiting as one batch. Follow these steps to see the order in which it checks the batch:

1. **Fencing check** — the branch token `c<seq>` equals the unit's current `claim_seq`. A superseded token is rejected outright, even if the branch is green.
2. **Write-set check** — every path the branch changed since it forked lies in the unit's write-set: the paths it `owns`, plus the `status`, `built`, and `trace` fields of its own entry. Anything else — tests included — rejects the work before it merges.
3. **Merge** — `--no-ff`, one merge per unit, so history keeps the unit boundary. With the write-set already checked, a conflict means mainline changed paths the unit may write, and it escalates.
4. **Input check** — the trace recorded when the unit was stamped Built equals the hashes of its inputs at this head. A unit built from text that changed while it was being built is rebuilt from the current text, through no fault of the work.
5. **Commands** — the SDD's §5 Bootstrap and Build; the Verification rows of the merged units and of the SDD's **dirty** Verified units, which include every dependent of the merged units; the §5 Lint command; the §5 Test command as a regression check; then audit checks 1–3. Rows of clean units do not run, because their recorded checks still hold; see [Build State](build-state.md#the-two-cascades) for what makes a unit dirty. An SDD without a Test command re-runs every Verified unit's rows in full.
6. **Publish** — on green, one commit stamps every unit of the batch Verified, with its journal events and the regenerated ledger, and the merges and that commit are published by compare-and-swap or push.

Semantic conflicts between parallel units are caught at step 5, where the rows of every unit the merge can reach run against the merged tree. On red, nothing is published. Every gate starts from a pristine checkout, described below.

## Gate Behavior

These rules govern how the gate reaches and keeps its verdict.

### Red Batches Are Retried One Unit at a Time

A batch that fails is integrated again one unit at a time, so one bad unit never holds back the rest: the units that are sound still land, and the one that is not is named.

### The Test Command Is a Regression Check

Lint must pass from the start; the §5 Test command need not. If Test passed at the head before the merge, it must pass after it; while it already fails there, it judges nothing. A build is Complete only when it passes. This closes the gap between "every Verification row is green" and "the suite is green": rows select tests one by one and cannot see a module that no longer collects, or a test another unit broke.

### Verification Rows Run Side by Side

Each distinct Verification command runs once per gate, up to `run.gate_jobs` at a time — one per CPU core by default. A row that fails while others were running is run again alone before it counts, so rows that merely disturb each other cannot turn a gate red. Each command gets a `TMPDIR` of its own. Bootstrap, Build, Lint, and Test always run on their own. Set `gate_jobs = 1` for an SDD whose rows share state.

### Every Gate Starts from a Pristine Checkout

Before every gate, re-check, preflight, and culprit probe, the checkout is reset to the commit being checked and every untracked or ignored file is removed: environments, build outputs, and caches inside the checkout do not survive. A result therefore never depends on what an earlier run left behind; most order-dependent flaky tests are fixed by cleaning shared state between runs [luo-flaky]. Only caches outside the checkout persist, so the cost is a Bootstrap and Build from scratch at each gate.

### Only New Failures Reject a Batch

When rows of an already Verified unit that is not in the batch fail at a gate, or rows fail in a re-check, they run again on the base, the commit the batch was merged onto, in the lane's own pristine base checkout. A failing row of a unit in the batch is always the batch's.

- **They pass on the base:** the batch broke them, and it is rejected.
- **They fail on the base too:** the batch did not break them. Their units are **rotten**, and the batch is judged on the other rows.

This is the Test command's rule, applied row by row. If the base itself cannot be set up, nothing can be said about the rows there, and the batch is blamed for all of them.

### Rotten Units and Culprits

A rotten unit is reported, never regenerated: its rows fail after a change upstream, with no spec change of its own, which is a contract question for a person. Either the upstream broke its contract, or the unit relied on more than the contract. The units that depend on it wait, in any SDD, its rows are left out of the lane's later gates while it is held, and everything else keeps building.

To find what broke it, vitrify runs flake-aware culprit finding off the critical path, in a probe checkout of its own. The search is a Bayesian noisy binary search over the commits on mainline since the unit's anchor. It keeps a probability for each commit being the culprit and one for no culprit, runs the failing row at the commit that best splits that probability, and updates on each pass or fail with an estimated flake rate [facf]. The prior is concentrated on the commits that touched a path the unit is checked against, and every other commit keeps a small share, so an undeclared dependency can still be found. It concludes a culprit commit, a flake, or nothing within its budget. The report line names the culprit commit and the unit it integrated or changed (`[api] U-004 row 2 broke at a1b2c3d (core U-002 integrated) — culprit found`), or names the flaky row (`… is flaky: no commit since its last check broke it — fix the row`), or says the search is inconclusive and names the most likely commit; a flaky unit stops being held rotten and is checked again. Plain bisection cannot conclude a flake, which is why vitrify does not use it.

### Re-checks Across SDDs

Within an SDD, dirty units are re-checked inside the integration gate. Across SDDs, a lane re-checks its dirty units after the upstream merge, in the dependent's own lane and in dependency order: once nothing a unit depends on has work pending, their rows run, then the SDD's §5 Test command. Passing units are recorded in one commit, `sdd: re-check U-001, U-002 at <sha>`, with `verified` events by actor `recheck`, which move their anchors, and `suite: pass` when the Test command passed; failing rows are judged by the rule above, and a failing row of a unit nothing in the batch touched makes the unit rotten.

### The Gate Discards What Its Commands Write

Whatever the gate's commands write or rewrite in the integration checkout is discarded before the integration commit; only the merges and that commit are published. A passing gate prints nothing.

### Protocol Traffic Does Not Void a Verdict

While a gate runs, the same run keeps claiming, renewing, and releasing, and each is a mainline commit. Those commits change journals, ledgers, and the managed fields of unit entries — nothing a build, a test, or a lint reads — so when the head has moved only by such commits, the merges are replayed on it and published without running the gate again. Any other change voids the verdict, and the batch is gated again from the new head. The same holds for any commit that changed nothing the gate checked or read — another lane's integration, say: the files of every unit whose rows ran and of every unit those depend on — not another SDD's test paths, which no gate of this SDD reads —, the SDD's unmanaged files, the paths outside every SDD's scope it takes as input (all of them, or its `shared_inputs`), and, when it ran the §5 Test command, everything in the SDD's scope. The merges are replayed on the new head, the fencing, write-set and input checks are made again there, and the result is published without another gate.

## Lint and Audit

Lint holds a plan-time spec to its rules before any work starts; the audit compares the spec with the files and the recorded state at any time.

- **Lint** — L1–L8 hold the SDD to itself: the dependency graph, ownership, parallel groups, contract references, the reading-set budget, requirement coverage, pre-build state, and open questions. L9–L12 hold it to what it names outside itself: other SDDs' contracts, its changelog, its TDD, and the units of other SDDs it waits on through pins. `vitrify lint --built` drops L7 — the check that no build state exists yet — so that a partly built SDD can still be held to the rest; it is what every run preflights with.
- **Audit** — 1 orphans, 2 ghosts, 3 contract references, 4 rotten Verified (re-runs every Verified unit's Verification commands; a run re-checks only the dirty units instead), 5 trace mismatch, 6 projection consistency, and 7 lapsed leases. It only reports by default; `--fix-leases` and `--apply-stale` write fixes into the working tree.

[Design Rationale](../design-rationale.md#audit-and-lint) explains why each check exists.

## Limitations

These are the boundaries of what gates can see.

- vitrify never reads source code. Cross-unit imports that bypass contracts are invisible to it unless the SDD runs a boundary checker.
- No gate observes tests-first. A failing run before implementation is an instruction to the worker, not a recorded check.
- A passing gate leaves no output, and a command's output is not echoed while it runs; see [Inspect the Gate](../../user-guides/monitor-a-run.md#inspect-the-gate).
- Gate strictness belongs to the installed version: a release may reject work another release merged. See [Versioning](../../reference/formats.md#10-versioning).
- The audit sees one SDD. A pin behind its authority is lint's finding (L9), not the audit's; `vitrify check` runs both.

## Practical Guidance

- Make every §5 command and Verification row non-interactive, runnable from the repository root, with a deterministic exit code.
- Keep rows hermetic — their own temp directories and ports, no shared scratch state. If they cannot be, say so in Ground Rules and set `gate_jobs = 1`.
- Let the Bootstrap command bring the environment up to date, so that rows only read it.
- Run `vitrify lint` until it exits 0 before setting `status: Active`, and `vitrify lint --built` on an SDD that already has state.
- Use `vitrify audit --skip-rotten` only right after a gate has run the Verification commands; plain `vitrify audit` re-runs them all.
- Pin a boundary checker for your language when tests or code must reach other units only through contracts.
