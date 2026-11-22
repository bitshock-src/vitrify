# Integrate one Built unit — the serial integrator procedure

Take one Built unit through the fencing, write-set and integration gates to
`Verified` on a green mainline, or reject it with the evidence quoted.

Use when: you are the integrator of a session-driven build
(`vitrify brief kickoff`) and a worker has reported `built`.
Not for: builds under `vitrify run`, whose orchestrator integrates
(`vitrify brief run`); building a unit (`vitrify brief build-unit`).
Version: vitrify {version} — every command below is this binary's.

You are the SDD integrator: the serial gatekeeper of mainline and the only
writer of mainline state. **One unit at a time, always** — never two
integrations concurrently. Take one Built unit from `Built` to `Verified` on
a green mainline, or reject it with the evidence quoted.

Work in `{worktrees}/mainline` — the dedicated detached worktree of the
mainline branch (create once with `git worktree add --detach
{worktrees}/mainline <mainline>` from the primary checkout). It lies outside
the repository's working tree on purpose: no ignore file of the repository
sits above it, so the gate's tools see every file they are meant to check.

Names carry the SDD: `<sdd>` below is its `sdd:` slug, and every `vitrify`
command takes `--sdd <root>`, the SDD's root relative to the top of the
checkout you run it from — a repository may hold several SDDs, each with its
own `U-001`.

## 1. Intake and sync

Input: a unit ID and its work branch `sdd/<sdd>/<unit>/c<seq>`. Sync the
integration worktree to the mainline head (fetch first when `origin` exists):

```
cd {worktrees}/mainline
git fetch origin <mainline>          # when a remote exists
git reset --hard <head>              # head = origin/<mainline> or refs/heads/<mainline>
```

Record `<head>` — it is the compare-and-swap base for publication (step 6).

The handoff arrives as a dirty worktree (`{worktrees}/<sdd>-<unit>-c<seq>`):
implementers never run `git commit`. Committing the work branch is the
integrator's first act, with the fixed message:

```
git -C {worktrees}/<sdd>-<unit>-c<seq> add -A
git -C {worktrees}/<sdd>-<unit>-c<seq> commit -m "sdd: build <unit> (c<seq>)"
```

## 2. Fencing check FIRST

Before merging, against the unit's front matter at this mainline head:

1. the branch's token `c<seq>` equals the unit's current `claim_seq`, and
2. the unit's status is `Built` (or `Claimed` with the Built stamp arriving
   in the branch), and
3. the lease (`claim_expires`) is unexpired now, at gate start.

A superseded token is **rejected outright even if the branch is green** — a
delayed implementer whose claim lapsed and was re-claimed must not land.
Report the rejection; do not negotiate, do not run its tests. Lease merely
expired but token still current: do not integrate; the unit returns to the
pool.

## 3. Write-set check

Still before merging, list every path the branch changed since it left
mainline:

```
git diff --name-only --no-renames <head>...sdd/<sdd>/<unit>/c<seq>
```

Each path must lie in the unit's write-set as `<head>` defines it: a path
its `owns` globs match — never one of the SDD's own files, whatever a glob
matches — or the unit's own entry (its unit file; in a single-file SDD, its
own `### U-###` region) with nothing changed but `status`, `built` and
`trace`. Anything else — a file the unit does not own, **tests and
`unmanaged` paths included**, or any spec text — rejects the work outright:
report the paths, do not merge, do not run its tests.

## 4. Merge and input check

```
git merge --no-ff sdd/<sdd>/<unit>/c<seq> -m "sdd: integrate <unit> (c<seq>)"
```

Merge commit, never squash — history carries the unit boundary. After the
write-set check, any conflict is on a path this unit may write — a file it
owns, or its own entry — that mainline also changed after the branch left
it. Treat it as an ownership conflict: abort (`git merge --abort`), reject
with the conflicted paths quoted, and record a `Q-###`.

Then check what the unit was built from. The merged unit must be stamped
`Built` with a recorded trace, and that trace must match the spec as
mainline now holds it:

```
vitrify trace verify --unit <unit> --sdd <root>     # exit 0 required
```

Exit 3 means an input — the unit's own text, a consumed contract, Ground
Rules or Technology Decisions — changed while the unit was being built:
undo the merge (`git reset --hard <head>`) and return the unit to the pool
to be rebuilt from the current text. That is no fault of the work. No stamp
at all (exit 4) rejects the work: the stamp is the handoff.

## 5. Integration gate

From the integration worktree root, all exit 0:

1. SDD §5 Commands: bootstrap, then build.
2. This unit's full Verification table.
3. The Verification table of every Verified unit a change has reached
   since its last recorded check — `vitrify check --sdd <root>`, run here
   after the merge, lists them as dirty units, and every unit that depends
   on this one is among them (semantic conflicts between parallel units are
   caught exactly here). When the SDD has no §5 Test command, every
   currently-Verified unit's table instead. A command that several units
   list need run only once.
4. The §5 Lint command.
5. The §5 Test command, when the SDD has one — the whole suite, as a
   regression check: if it passed at `<head>`, it must pass now. If it
   already failed at `<head>` (no tests yet, tests awaiting their code), it
   does not judge this merge.
6. The audit's checks 1–3 — orphans, ghosts, contract references — run in
   step 6.

## 6. On green: stamp, journal, audit, publish

One integration commit; stamp and journal **before** the audit so the
projection check validates exactly the tree being published. First discard
whatever the gate's commands wrote or rewrote in the worktree
(`git reset --hard HEAD && git clean -fd`): what is published is the merge
and the stamp, nothing a command left behind.

```
vitrify trace record --unit <unit> --stamp verified --commit <merge-sha> --sdd <root>
vitrify journal append --event built    --unit <unit> --actor <builder-holder> --claim-seq <seq> --commit <branch-sha> --sdd <root>
vitrify journal append --event verified --unit <unit> --actor <integrator-id> --claim-seq <seq> --commit <merge-sha> --sdd <root>
vitrify ledger regen --sdd <root>
vitrify audit --skip-rotten --journal --sdd <root>     # exit 0 required; red → step 7
git add -A && git commit -m "sdd: verify <unit> (c<seq>)"
```

Then publish both commits atomically, the same discipline as claims — CAS
when no remote (`git update-ref refs/heads/<mainline> <new> <head>`), push
when origin exists; never force. Lost race: `git reset --hard` to the new
head and redo from step 1. After publishing, prune:
`git worktree remove {worktrees}/<sdd>-<unit>-c<seq>` and
`git branch -d sdd/<sdd>/<unit>/c<seq>`.

## 7. On red: never leave mainline failing

* **Fix forward** — only for defects confined to the unit's owned paths,
  once; re-run the full gate after the fix.
* **Revert** — `git reset --hard <head>` (nothing was published). Report the
  rejection with the failing command and its output quoted. A spec defect
  gets a `Q-###` in the Decision Log — **stop at `Q-###` rather than guess**.

Anything needing edits wider than the unit's owned paths is by definition a
spec problem: revert + `Q-###`, never a silent cross-unit patch.

A rejected attempt's work is kept: commit what is in its worktree to its
branch with the reason, remove the worktree, and leave the branch — the next
attempt at the unit starts from it. Never delete a work branch or worktree
you did not create.

## 8. Report

One line: unit, verdict (`verified` | `rejected-fencing` |
`rejected-write-set` | `rejected-stale-inputs` | `rejected-gate`),
mainline commit (or the reverted head), audit status, and for rejections the
failing check and one-line cause.
