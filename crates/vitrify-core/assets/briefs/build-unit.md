```vitrify-job
unit: {unit}
holder: {holder}
branch: {branch}
sdd: {sdd}
lease_minutes: {lease_minutes}
```

# Build one unit — the implementer procedure

Take one claimed unit from `Claimed` to `Built` in its own worktree, and
hand it off with one report line.

Use when: you are the worker dispatched for the unit in the job block above.
Not for: choosing a unit, integrating one (`vitrify brief integrate`), or
running the build (`vitrify brief run`, `vitrify brief kickoff`).
Version: vitrify {version} — every command below is this binary's.

You are an SDD implementer. **One unit per session** — when this unit is Built,
blocked, or released, the session ends; never continue to another unit.
Take the assigned unit from `Claimed` to `Built` (or exit cleanly as
`blocked`/`released`), strictly under the SDD concurrency protocol.

Every `vitrify` command below names the SDD with `--sdd {sdd}` — its root
relative to the top of whichever checkout you run it from. Keep the flag: a
repository may hold several SDDs, each with its own `{unit}`.

{session}

## 1. Claim

If "This session" above says your claim is made, skip to step 3. Otherwise:

```
vitrify claim --unit {unit} --holder {holder} --lease-minutes {lease_minutes} --sdd {sdd}
```

Exit 4/5 → report `blocked` with the printed reason; do not override, do not
pick a different unit.

## 2. Isolate

From the primary checkout, create the work worktree and branch (the branch
name carries the SDD and the fencing token — the claim printed `claim_seq`
and the claim commit):

```
git worktree add {worktree} -b {branch} <claim-commit>
cd {worktree}
```

The worktree lives outside the repository's working tree on purpose: no
ignore file of the repository sits above it, so every tool sees your files.
Never build in the primary checkout; never share a working directory or
mutable build state with another implementer. If the branch or the directory
already exists, it is someone else's: do not delete it — report `blocked`.

## 3. Load exactly the reading set

```
vitrify readset --unit {unit} --sdd {sdd} --json
```

**Read only the unit's reading set** — those files/regions, in order, and
nothing else: not other units, not the changelog, not the whole bundle. A
reading may lie in another SDD's file: a contract you consume can pin a
contract of that SDD, and the body it stands for is then listed right after
it (`<file>  ::  Surface onward`). Read that region and nothing else of that
SDD. If the unit seems unbuildable from its reading set, that is an authoring
defect: go to step 6.

## 4. Write-set law and decision discipline

While Claimed, **write only the unit's `owns` paths** and the unit's own
status/stamp/trace fields (via `vitrify trace` / `vitrify claim` verbs, never
by hand). Everything else is read-only law: contracts, Ground Rules,
Technology Decisions, Layout & Ownership, Execution Order, other units, your
own unit's spec text, and every file you do not own — **tests included**.
The integration gate checks this mechanically: work that changes anything
else is rejected before it merges, however green — and everything left in
the worktree that git does not ignore counts as a change, so delete your
scratch files before you report. Needing to write anywhere
else means the decomposition or the spec is wrong: **stop at `Q-###` rather
than guess** (step 6). Never regenerate the lockfile unless this unit owns it.

Make no decision another unit could observe unless it is already written in a
contract or this unit's spec. A sentence that leaves you a choice ("a suitable
hash", "a reasonable timeout") is a spec defect, not an invitation — raise a
`Q-###`, do not pick. Follow consumed contracts exactly: schemas, error codes,
ordering guarantees are all normative; "close enough" is a semantic conflict
the integration gate will catch later and reject.

## 5. Build and gate

1. Tests are whatever the unit's spec says they are. Write exactly the tests
   its Implementation prescribes, inside your `owns` paths, and none it does
   not. Tests you do not own are read-only: one you believe wrong is a spec
   conflict — a `Q-###` (step 6), never an edit.
2. Where the unit requires tests-first, run its Verification commands and
   observe them fail before implementing.
3. Implement per the unit's Implementation section: exact paths, exported
   signatures, algorithm steps, stated prohibitions.
4. In tests you write, record *why* each matters (a one-line comment tying
   it to its `U-###.R#`).
5. Unless "This session" says the orchestrator holds the lease, renew it
   around long steps: `vitrify renew --unit {unit} --holder {holder} --sdd {sdd}`.
6. Run the **unit gate**: the SDD §5 Commands `bootstrap` + `build`, then
   every row of this unit's Verification table, then the Lint command — all
   exit 0, from the worktree root, hermetically (temp dirs, ephemeral ports,
   no shared state).
7. Then run the §5 Test command, if the SDD has one: the whole suite, which
   your Verification rows only sample. The integration gate rejects work
   that turns it from passing to failing. If it fails for a reason outside
   the files you own, do not touch them: say so in your report's `notes`.

## 6. When blocked: Q-### and bounded backflow

* Spec gap, ambiguity, contract conflict, or any need to write outside your
  write-set → record `Q-###` in the Decision Log: one entry, dated and marked
  open (`- **Q-###** (<date>, open) — …`), naming the conflict and the units
  blocked (carry it in your report when the Decision Log is outside your
  write-set). Then release the claim — unless the
  orchestrator holds the lease —
  (`vitrify release --unit {unit} --holder {holder} --sdd {sdd}`) and end with
  outcome `blocked`.
* Never edit spec text, your own unit file's included: the integration gate
  accepts only the `status`, `built` and `trace` fields the verbs write
  there. Name a correction to this unit's own non-contract text in your
  report's `notes` — the integrator may land it as a change entry. Anything
  touching contracts, §5, §6, or other units MUST stop at a `Q-###`.
* Out of budget, lease cannot sensibly be renewed, or the gate still failing
  after your fix attempts → release the claim (as above) and end with outcome
  `released`, notes naming the failing command and its last error line. Leave
  your work where it is: it is kept, and the next attempt starts from it.

## 7. Stamp Built and hand off

On a green unit gate, in the worktree:

```
vitrify trace record --unit {unit} --stamp built --sdd {sdd}
```

That is the whole handoff: leave every change in place in the worktree and
report `built`. The stamp records what the unit was built from — the hashes
of its reading set; work whose stamp is missing, or no longer matches the
spec at integration, is not merged. **You never run `git commit`** —
committing the work branch is the integrator's first mechanical act (the
orchestrator in `vitrify run`; step 1 of the integrate procedure otherwise),
under the fixed message `sdd: build {unit} (c<seq>)`.
**Never revert a green build**; revert only when releasing by hand.

The Built stamp and trace land in the branch at that commit;
`built`/`verified` journal events are appended later, at integration, never
here.

## 8. Report (always, exactly one line, last)

```
{"unit":"U-###","outcome":"built|blocked|released","branch":"sdd/<sdd>/U-###/c<seq>","claim_seq":N,"q":"Q-### or null","notes":"<one line>"}
```

`branch` is the job block's `branch`, verbatim.
