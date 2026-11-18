# Troubleshooting Guide

> Troubleshoot vitrify setup, authoring checks, run preflight, gate rejections, build state, and recovery by the symptom you see.

Use this page when something does not work and you can name the symptom. Each section gives the likely cause, the narrowest check, and the fix — or the page that owns the fix. To watch a run while you diagnose it, see [Monitor a Run](../user-guides/monitor-a-run.md); for what each exit code means, see [Exit Codes](../cli/command-reference.md#exit-codes).

> [!WARNING]
> Worker logs, prompts, and escalation evidence under `.vitrify/logs/` contain spec text and complete agent sessions. Remove anything sensitive before you attach them to an issue or share them.

## Find the Relevant Check

Use this table to jump to the section for your symptom.

| Symptom | Start with |
| --- | --- |
| a command exits 4 with `error: vitrify is not set up on this machine` | [vitrify Is Not Set Up on This Machine](#vitrify-is-not-set-up-on-this-machine) |
| `vitrify init` or `vitrify probe` prints a `FAIL` line | [`vitrify init` or `vitrify probe` Reports FAIL](#vitrify-init-or-vitrify-probe-reports-fail) |
| `vitrify init` refuses, or `vitrify probe` changes nothing | [`vitrify init` Refuses, or `vitrify probe` Changes Nothing](#vitrify-init-refuses-or-vitrify-probe-changes-nothing) |
| a harness shows `untested` | [A Harness Shows `untested`](#a-harness-shows-untested) |
| a verb exits 4 about finding SDDs, or about `schema_version` | [A Verb Exits 4: No SDD, or Several](#a-verb-exits-4-no-sdd-or-several) |
| `vitrify run status` exits 4, or a run's output is lost | [No Run Recorded, or No Output Kept](#no-run-recorded-or-no-output-kept) |
| `vitrify check` prints findings | [`vitrify check` Reports Findings](#vitrify-check-reports-findings) |
| `vitrify change apply` refuses | [`vitrify change apply` Refuses](#vitrify-change-apply-refuses) |
| `vitrify lint` prints `[L#]` findings | [`vitrify lint` Reports Findings](#vitrify-lint-reports-findings) |
| `vitrify run` prints `preflight failed` | [`vitrify run` Exits 2 With `preflight failed`](#vitrify-run-exits-2-with-preflight-failed) |
| attempts are released, time out, or do nothing | [Workers Make No Progress, or Every Attempt Is Released](#workers-make-no-progress-or-every-attempt-is-released) |
| `vitrify run status` warns that a worker is quiet | [A Worker Is Quiet](#a-worker-is-quiet) |
| `gate red`, `write-set violation`, or `built from text that has since changed` | [The Gate Rejects Work](#the-gate-rejects-work) |
| rows fail only when run alongside others | [Verification Rows Fail Only When Run Together](#verification-rows-fail-only-when-run-together) |
| a checker passes in the gate but fails in your checkout | [A Checker Passes Without Checking Anything](#a-checker-passes-without-checking-anything) |
| `set aside for this run` or `not started` | [A Unit Is Set Aside](#a-unit-is-set-aside) |
| the run ends `Blocked.` | [The Run Ends Blocked](#the-run-ends-blocked) |
| a warning that the primary checkout could not follow | [The Primary Checkout Is Behind](#the-primary-checkout-is-behind) |
| the gate runs a long time with no output | [The Gate Seems Stuck](#the-gate-seems-stuck) |
| `journal replay`, `ledger regen --check`, or `trace verify` exits 3 | [Recorded State Does Not Match](#recorded-state-does-not-match) |
| `claim`, `renew`, or `release` exits 4, 5, or 6 | [A Claim Transaction Fails](#a-claim-transaction-fails) |
| `pin list` shows `stale`, `missing-authority`, or `invalid` | [An External Contract Is Out of Step](#an-external-contract-is-out-of-step) |
| a run was killed, or a worker outlived its run | [A Run Was Killed](#a-run-was-killed) |

## vitrify Is Not Set Up on This Machine

Every command but `vitrify init` — `lint`, `check`, `locate` and `brief` as much as `run` — refuses with exit 4 until `vitrify init` has created `~/.vitrify/config.toml`:

```text
error: vitrify is not set up on this machine: ~/.vitrify/config.toml not found → vitrify init
```

Run `vitrify init` once, in your own terminal; no other command creates `~/.vitrify/`. With `VITRIFY_HOME` set, the home is that directory instead: set it the same way for `init` and for every later command.

## `vitrify init` Refuses, or `vitrify probe` Changes Nothing

`vitrify init` refuses with exit 2 when `~/.vitrify/config.toml` already exists: the machine is set up, and `vitrify probe` brings its probe record up to date. `vitrify probe` asks before it changes the file: any answer but `y` or `yes` — or none, when stdin is closed, as under an agent — changes nothing, prints `note: ~/.vitrify/config.toml: nothing changed`, and exits 2. Answer `y`, or run `vitrify probe -y`.

## `vitrify init` or `vitrify probe` Reports FAIL

Each exits 2 when any check fails, and each `FAIL` line names what failed. Use this table to fix it, then run `vitrify probe`.

| `FAIL` line | Cause | Fix |
| --- | --- | --- |
| `git not found on PATH` | git is missing from the shell's `PATH` | Install git 2.30 or later. |
| `harness <name>: headless probe failed (<argv>)` | the CLI is installed but its streaming headless spelling did not exit 0 with output; the line names the spelling vitrify needs | Sign in to the harness, then run the printed command yourself with the prompt `Reply with exactly: OK` to see its error. A CLI too old for the named spelling needs upgrading. |
| `harness <name>: probe timed out after 60s (<argv>)` | the probe did not finish in time | Complete any interactive login first; on a slow connection, raise `VITRIFY_PROBE_TIMEOUT_SECS`. |
| `harness <name> <role>: probe failed (<argv>) — fix [harness.<name>.<role>] in ~/.vitrify/config.toml` | the CLI rejected the role's model, effort, or `args`, or the account cannot reach the model | Correct that table in `~/.vitrify/config.toml`; see [Harness CLIs](../cli/harnesses.md#configure-role-profiles). |
| an error naming `~/.vitrify/config.toml` | a role profile has an unknown key or a value of the wrong type | Fix the line the error names; the shipped defaults are in [`crates/vitrify-core/assets/config.toml`](../../crates/vitrify-core/assets/config.toml). |

## A Harness Shows `untested`

The CLI is not on the `PATH` of the shell that runs vitrify. This is not an error: `untested` harnesses are simply not used. If you want to build with it, install it or fix your `PATH`, then run `vitrify probe`.

## A Verb Exits 4: No SDD, or Several

A verb that acts on one SDD exits 4 when it finds none, or when it finds several and `--sdd` is absent. Check with `vitrify locate`, which lists every SDD discovery finds.

- **None found.** The refusal names where to start: `error: no SDD found in repository → start one: vitrify brief authoring (or vitrify new sdd inside specs/)`. An SDD counts only if its front matter has an `sdd` key. Discovery searches `specs/*/sdd.md`, then `specs/*.sdd.md`, then the files git would commit — so an SDD under an ignored path is never found.
- **Several found.** The refusal prints the candidates as `--sdd` values (`--sdd specs/alpha`, …). Pass one of them, or `--all` on the verbs that take it: `lint`, `check`, `audit`, `ledger`, `pin`, `journal replay`, `change apply`, and `questions`.
- **Found but refused.** This vitrify reads `schema_version: 2.0` only, and the key is required. An SDD that lacks it or declares another version is still listed by `locate`, and every verb that would read it exits 4 with a message naming `2.0`, such as `error: <file>: schema_version is missing; this vitrify reads 2.0 → vitrify schema show`. Write the SDD to the 2.0 schema and declare `schema_version: 2.0` in its core front matter; see [Versioning](../reference/formats.md#10-versioning).

## No Run Recorded, or No Output Kept

`vitrify run status` exits 4 with `error: no run recorded in this repository → start and watch one: vitrify brief run` when no `vitrify run` has started in this repository; `vitrify brief run` is the procedure for starting and watching one. Run it from the repository the run was started in.

A run's final report does not depend on its output being kept. Once a run has ended, `vitrify run status` (or `vitrify run status --job <job>` for an earlier run) prints the report under `Report:`, and the same lines are in `.vitrify/logs/<job>/report.txt`. See [Check a Run's Status](../user-guides/monitor-a-run.md#check-a-runs-status).

## `vitrify check` Reports Findings

`vitrify check` exits non-zero and prints one line per finding, each prefixed with the SDD and its source — `[lint L4]`, `[audit 5]`, `[journal]`, `[ledger]`, or `[pin]` — and ending ` → <remediation>`. Run the remediation, then `vitrify check` again. Exit 2 means a lint, audit, or pin finding; exit 3 a journal or ledger mismatch, which [Recorded State Does Not Match](#recorded-state-does-not-match) covers. Lines such as `calc: note: Q-007 is open (blocks U-002)` are notes, not findings; lint L8 is what fails on an open question that blocks a Ready unit.

## `vitrify lint` Reports Findings

Lint exits 2 and prints one `[L#] subject: cause → remediation` line per finding; the remediation is the fix to run first. Use this table for each check's usual cause; the [Command Reference](../cli/command-reference.md#vitrify-lint) states every check exactly.

| Check | Usual cause | Fix |
| --- | --- | --- |
| L1 | a `depends_on` names a missing unit, or the graph has a cycle | Correct the ID, or break the cycle. |
| L2 | two units' `owns` overlap, or a file under `scope_roots` is unowned | Narrow the globs; add the file to one unit or to `unmanaged:`. |
| L3 | a unit depends on a unit in its own parallel group | Move one of them out of the group. |
| L4 | a consumed contract is missing, or a `Consumers:` line disagrees with `consumes` | Restore the contract, or make the line name exactly the consumers. |
| L5 | a reading set is over budget, or does not resolve | Split the unit or trim what it reads; check with `vitrify readset --unit U-### --budget`. |
| L6 | a requirement is in no Verification row | Add the requirement to a row (`R<n>`, a range, or `all`). |
| L7 | a unit has build state before the build | Expected once units are built: use `vitrify lint --built`. |
| L8 | `OPEN(Q-###)` in a Ready unit, or a marker with no Decision Log entry | Settle the question first; add the missing `Q-###` entry. |
| L9 | a pin does not match its authority, names one that is not there, or is declared wrongly | Run `vitrify pin sync --sdd <root>` for a stale pin; see [An External Contract Is Out of Step](#an-external-contract-is-out-of-step). |
| L10 | `version` or `updated` is not the last change entry's, or a single file's entry is outside `## Changelog` | Make both match the last change entry; write a single file's entries as `### CH-###` under `## Changelog`. |
| L11 | `tdd:` names no file, or an Active SDD pins an unapproved or superseded TDD | Fix the path, or re-pin under a change entry. |

## `vitrify run` Exits 2 With `preflight failed`

Nothing was started. Each `preflight failed: <cause> → <fix>` line names one gate and its fix; `vitrify run --dry-run` reproduces the whole preflight without starting anything. [Runs](../about/concepts/runs.md#preflight) lists every check with its fix; use this table for the details.

| Failure | Fix |
| --- | --- |
| no SDD is found, or several and no `--sdd` | Start one with `vitrify brief authoring`, or pass one of the printed `--sdd` values. |
| the SDD is not Active, has uncommitted changes under its root, or shares its slug | Set `status: Active`, commit or stash the changes under its root, and give every SDD a unique `sdd:` slug. |
| lint findings | Fix them; preflight runs `vitrify lint --built`. |
| audit findings | Fix them; lapsed leases alone are released automatically. An unmarked trace mismatch needs its change entry applied with `vitrify change apply` — see [Change a Spec](../user-guides/change-a-spec.md). |
| a §5 Bootstrap, Build, or Lint command fails | Run it exactly as written from the repository root and fix it; it must be non-interactive. |
| no working harness is recorded | Run `vitrify probe` and resolve its `FAIL` lines. |
| a role profile does not parse | Fix the line the error names, in the file it names: `~/.vitrify/config.toml`, or the repository's `.vitrify/config.toml`. |
| the worktree root is unusable | Move `run.worktree_dir` outside any directory your repository's ignore files cover. |

## Workers Make No Progress, or Every Attempt Is Released

The usual cause is a headless session that is not allowed to do its job: nobody is there to answer a permission prompt, so whatever the harness would ask about is refused. Other causes are a harness that is signed out, a model the account cannot reach, and a session that ends without a parseable report line.

Check the worker's own account first: its `report [w<i>] …` line on stderr, its `outcome` in `.vitrify/run.json` (`released`, `timeout`), and the end of `.vitrify/logs/<job>/worker-<job>-w<i>.out` and `.err`.

Fix it by granting the `worker` and `retry` roles what they need through `args` in the role profiles of `~/.vitrify/config.toml` — see [Grant Worker Permissions](../cli/harnesses.md#grant-worker-permissions) — then run `vitrify probe`. A missing or unparseable report is treated as `released`; the report format is in the [Data Model and Formats Specification](../reference/formats.md#9-worker-report-line-and-escalation-verdict).

## A Worker Is Quiet

`vitrify run status` warns when a worker's log has not grown for `--quiet-after` seconds (300 by default). Every recorded harness spelling streams, so a quiet log means the session itself is quiet: a long-running command in the worktree, a long model turn, or a session waiting on something nobody will answer, such as a permission prompt. Look at the end of its log, `.vitrify/logs/<job>/worker-<job>-w<i>.out`, and at its worktree. A worker that stays quiet until its hard budget is stopped at `worker_timeout_minutes`, and its claim is released; if every worker does that, see [Workers Make No Progress](#workers-make-no-progress-or-every-attempt-is-released).

## The Gate Rejects Work

A rejected attempt is never lost: its work is kept on its branch, `sdd/<sdd>/<unit>/c<seq>`, under a commit whose message is the reason, and the unit's next worker starts from it. [Find Why an Attempt Did Not Land](../user-guides/monitor-a-run.md#find-why-an-attempt-did-not-land) lists the five records of it.

### `gate red integrating U-###: <command>`

A gate command failed on the merged tree; nothing was published. The branch's commit message holds the command and the last 40 lines of its output. Run the command yourself from the repository root on mainline plus the branch. If a Verified unit's check broke, the two units conflict semantically — the contract between them is incomplete, which is a spec fix.

### `write-set violation integrating U-###: <paths>`

The work changed paths the unit does not own — another unit's files, `unmanaged` paths, the SDD's own files, or its own spec text beyond its status, stamp, and trace. If the unit truly needs those paths, the spec's ownership is wrong: fix it under a change entry. If the paths are build or test byproducts, add them to `.gitignore`: the handoff commits every change git does not ignore.

### `U-###: built from text that has since changed`

The spec moved while the unit was being built. This is not a fault of the work: the unit is rebuilt from the current text.

### `rejected-fencing` or `rejected-not-built`

`rejected-fencing` means the attempt's claim was superseded — its lease lapsed and the unit was claimed again — so its work can never land, by design. `rejected-not-built` means the work was handed over without a Built stamp; the worker must run `vitrify trace record --unit U-### --stamp built` as its procedure says.

## Verification Rows Fail Only When Run Together

The gate runs Verification rows side by side — `run.gate_jobs`, one per CPU core by default — and a row that fails beside others is re-run alone before it counts. Rows that still fail share state: a fixed port, a shared directory outside their own `TMPDIR`, a database. Make the rows hermetic, or say in Ground Rules that they are not and set `gate_jobs = 1` (`--gate-jobs 1`) to run them one at a time.

## A Checker Passes Without Checking Anything

Some checkers honour ignore files above their own root and read nothing in a directory such a file covers — and then pass. That is why vitrify builds and gates in worktrees outside the repository's working tree, and refuses a `run.worktree_dir` that one of the repository's ignore files covers. If you moved the worktree root, move it back outside any ignored directory. This was reproduced with tach 0.35.3; see RA-18 in [Design Rationale](../about/design-rationale.md#rejected-alternatives).

## A Unit Is Set Aside

Two lines mean a unit will not be tried again in this run:

- `U-001: 4 attempts did not land (last: rejected-gate) — set aside for this run` — the unit's attempts keep failing. Read why (see [The Gate Rejects Work](#the-gate-rejects-work)), fix the spec or the environment, commit, and run again.
- `U-002 (c1): not started — <why>; set aside until the spec changes` — the unit's reading set does not resolve at its claim commit: a pin whose authority has moved, or a consumed contract that is gone. Sync the pin or restore the contract, and commit; the unit returns to the pool when its reading set changes.

## The Run Ends Blocked

`Blocked.` names every unfinished unit's blocker, and exits 2. [Resolve a Blocked Build](../user-guides/run-a-build.md#resolve-a-blocked-build) maps each blocker line to its fix; `vitrify questions --open` lists the open questions and the units each holds up.

## The Primary Checkout Is Behind

An uncommitted change or an untracked file in your checkout was in the way of a change the run published, so vitrify left the checkout exactly as it was. Signs: a `warning:` when it happens and again as the run's last word, the marker file `.vitrify/primary-behind`, and `git status` showing the missing commits as staged reversals.

Commit nothing from that checkout until it has caught up — a commit made from it would undo what was published. Clear what is in the way; it then catches up at the next commit vitrify publishes or when the next run starts, or at once with the command the warning prints (`git update-index -q --refresh && git read-tree -m -u <held> <head>`).

## The Gate Seems Stuck

A gate prints no output while it passes, but its progress can be seen. First run `vitrify run status`: the unit's line, or the `Gate:` line, shows the gate step, the command running and how long it has run. To see the processes themselves, list the run's child processes with `pgrep -P <run pid> -lf` (`-a` on Linux), using the `pid` in `.vitrify/run.json`. A command's output accumulates in `<system temp>/vfy-<hash>/c<n>.out` while it runs — the place to look at a command that seems stuck. See [Inspect the Gate](../user-guides/monitor-a-run.md#inspect-the-gate).

## Recorded State Does Not Match

These read-only checks exit 3 when the recorded state disagrees with itself.

- **`vitrify journal replay`** prints `mismatch U.field: journal projects …, front matter says …`, or `order: …` for ordering problems. The usual cause is a hand edit, or state-verb output committed without its partner. Find the commit that introduced it with `git log`; restore hand-edited lines from history, and record missing transitions with `vitrify journal append --event KIND`. A missing or empty journal exits 4, which is expected before an SDD's first event; `vitrify journal replay --all` prints `no journal yet` for such an SDD and does not fail.
- **`vitrify ledger regen --check`** names the first line that differs. Run `vitrify ledger regen` and commit; the ledger is never edited by hand.
- **`vitrify trace verify --unit U-###`** prints `STALE — mismatched keys: …`. An input changed without its change entry being applied: write the entry and run `vitrify change apply`, which marks the unit Stale for a behavioral change or re-records its trace for an editorial one — see [Change a Spec](../user-guides/change-a-spec.md).

## A Claim Transaction Fails

`claim`, `renew`, and `release` name the reason on failure. Use this table to act on the exit code.

| Exit | Means | Do this |
| --- | --- | --- |
| 4 | the unit is missing, not claimable, or not held by this holder; or `--sdd` is a path in no checkout of the repository | Check the unit with `vitrify locate --units`, and the path. |
| 5 | every publish attempt lost the race | Retry; if `origin` is involved, check that it is reachable. |
| 6 | the lease already expired, so renewal is refused | Claim the unit again. |

## An External Contract Is Out of Step

`vitrify pin list` shows each external contract's state, and lint L9 refuses the same problems. `stale` needs `vitrify pin sync`, the consumer's change entry around the line it prints, and `vitrify change apply --sdd <consumer>`; `missing-authority` needs the `Pin:` line corrected or the authority restored; `invalid` needs the declaration fixed — a pin carries no body, and a `Producer:` line that names another SDD's contract needs a `Pin:` line, whose hash `vitrify pin sync` then writes. See [Work With Several SDDs](../user-guides/multiple-sdds.md#3-check-external-contracts).

## `vitrify change apply` Refuses

`change apply` exits 2 and writes nothing when the entry cannot be recorded as it stands:

- **`version` or `updated` does not match the entry.** Set both in the SDD's front matter to the last change entry's, then run it again.
- **A pin is stale** (`error: calc: C-012's pin is stale → vitrify pin sync --sdd specs/calc first`). Run the sync it names first: the sync moves the consumer's traces too, and the entry must cover them.
- **An entry is outside `## Changelog`** (``error: gamma: CH-002: a change entry outside `## Changelog` is not read: …``). A single-file SDD reads its entries only under `## Changelog`, as `###` headings: a `## CH-###` heading ends that section. Write it as `### CH-### — …` under `## Changelog`.

When it prints `already recorded — nothing to apply`, `Draft — nothing to apply`, or `no change entry — nothing to apply`, nothing is wrong: running it again is safe.

## A Run Was Killed

Nothing needs repairing by hand. The next `vitrify run` finds the dead run's job lock free, releases its claims, keeps its workers' files on their branches, and names any worker that outlived it with the `kill` command that stops it. `vitrify run --dry-run` shows the claims it would recover without changing anything. If a run cannot publish because `origin` stopped answering, its claim or release fails and is reported — never put on a local branch — so restore the remote and run again.
