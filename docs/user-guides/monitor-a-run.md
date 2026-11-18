# Monitor a Run

> Watch a build from a second terminal and read how it ended: run output, the run record, worker logs, the gate, progress, history, and health checks.

Use this guide while a build is running, or after it has ended. A build can run for hours. Everything it knows can be read while it runs — from a second terminal, with the shell, `git` and vitrify's own read verbs — and after it has ended, however it ended. There are four places to look: what the run prints, the record it keeps in `.vitrify/`, each worker's log, and the repository. Only the last is authoritative: the first three describe one run, while the state of the build is what is committed on mainline (see [Build State](../about/concepts/build-state.md)).

## Agent Prompts

Your coding agent can watch a build and read it for you, from a session of Claude Code, Codex, or OpenCode at the repository root. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| watch a build, and hear how it ends | `Use vitrify cli to watch the build and tell me how it ends.` |
| see where a build stands | `Use vitrify cli to tell me how the build is going.` |
| read how the last build went | `Use vitrify cli to tell me how the last build went.` |
| find why an attempt did not land | `Use vitrify cli to find out why an attempt at U-### didn't land.` |

The agent reads the sources this guide describes — `vitrify run status`, the run's logs, the attempt's branch — and quotes them. To watch, it runs `vitrify run status --follow` in the background and waits without spending turns, as [Watch From an Agent Session](#watch-from-an-agent-session) describes. It looks and does not touch: it stops no worker and edits nothing a run is building.

## Where to Look

Use this table to find the right source for a question.

| To see | Look at |
| --- | --- |
| what each unit is doing, and what the gate is running | [`vitrify run status [--follow]`](#check-a-runs-status) |
| what the run is doing now | [what it prints](#read-what-the-run-prints) · [`.vitrify/run.json`](#read-the-run-record) |
| how far the build is | [`vitrify ledger progress`, `locate --units`, `ledger.md`](#check-progress-and-state) |
| what one worker is doing | [its log and its worktree](#follow-a-worker-session) |
| what is in the gate | [`run.json`, the integration checkout, the process list](#inspect-the-gate) |
| why an attempt did not land | [the five records of it](#find-why-an-attempt-did-not-land) |
| what landed, and when | [`git log` and the journal](#read-the-history) |
| whether mainline is sound | [`check`, `journal replay`, `run --dry-run`](#run-health-checks) |
| whether the run is alive, and how it ended | [the record and the exit status](#tell-whether-a-run-is-alive) · the report under `vitrify run status` |

The examples use `<job>` for the run's ID (in the first line of its output: `run-<UTC timestamp>-p<pid>`), `<sdd>` for the SDD's slug and `<sdd-root>` for its directory (`calc` and `specs/calc`), `<worktrees>` for the worktree root (the header's `Worktrees:` line; by default `~/.vitrify/worktrees/<repo>-<hash>`), and `main` for the mainline branch.

> [!WARNING]
> Worker logs and prompts under `.vitrify/logs/<job>/` contain each worker's full prompt — including the spec text it read — and everything its session printed. Protect that directory as you would any agent transcript, and review it before sharing.

## Read What the Run Prints

stdout is the narration: a header, one line per event, one final report. stderr is everything said along the way: each worker's report, each gate failure, notes and warnings. Keep both:

```sh
vitrify run 2>&1 | tee run.log
```

To leave it running:

```sh
nohup vitrify run > run.out 2> run.err &
```

`nohup` does what it should — a signal the run was started ignoring stays ignored — and a reader that goes away (`vitrify run | head`) does not take the run down with it.

### The Job Header

The header comes first, before anything is dispatched:

```text
SDD build job — mini @ 1.0.0  [run-20261114T020735Z-p73116]
Done-condition: every non-Retired unit Verified @ 1.0.0; audit clean; replay clean; §5 Test passing
Mainline: main   Remote: none   Shape: bundle
Plan: [P:a] U-001, U-002
Workers: cap=2   Holder IDs: run-20261114T020735Z-p73116-w<i>   Harness: claude
Roles: worker model=opus effort=medium   retry model=opus effort=high   escalation model=best effort=xhigh
Worktrees: /Users/you/.vitrify/worktrees/repo-2da6985e   Branches: sdd/mini/<unit>/c<seq>
Parameters: worker cap 2 · gate jobs 8 (one per core) · lease 60m · renew at 0.25 remaining · escalate after 2 failed attempts of a unit, set it aside after 4
Watch: vitrify run status --job run-20261114T020735Z-p73116 [--follow] · once it ends, the same command prints the report · procedure: vitrify brief run
```

The ID in brackets names everything else the run writes: its holders are `<job>-w<i>` and its logs are in `.vitrify/logs/<job>/`. `Remote:` says how commits are published — `origin` means every one is pushed. `Plan:` is the authored order — parallel groups by name, units outside a group with their status — for orientation; it is not a schedule. `Parameters:` shows the values in effect, whatever set them. `Watch:` names the command that watches this run — add `--follow` to stream it — and that, once the run has ended, prints its final report; `vitrify brief run` is the procedure an agent follows to operate the run.

### Event Lines

Events follow on stdout, one line each, as they happen. Use this table to read them.

| Line | Means |
| --- | --- |
| `dispatch U-001 (c2) → w3  [retry]` | U-001 is claimed (its 2nd claim), has a worktree, and worker 3 has started. `[retry]`: an earlier attempt did not land; this worker has its files and the reason |
| `integrated U-001 (c2)` | the work passed the gate and is published: the unit is Verified |
| `stale U-002: <cause> — it will be rebuilt` | the spec changed under a Built or Verified unit |
| `U-001: its reading set changed — back in the pool` | a unit set aside or blocked can be tried again |
| `recovered U-001 (c1): its holder <holder> is gone — claim released` | a claim left by a run that died |
| `escalation verdict: retry (<reason>)` | the judge was asked about a unit that keeps failing |
| `waiting: the run resumes when the spec changes (interrupt to stop)` | `--wait`, with only blocked units left |
| `Interrupted (SIGTERM): workers stopped, claims released, …` | the run was signalled and has cleaned up |

### Diagnostics

Diagnostics go to stderr. Use this table to read them.

| Line | Means |
| --- | --- |
| `preflight failed: …` | a preflight gate; nothing was started (exit 2) |
| `report [w1] U-001: built — <notes>` | the worker's own closing line: `built`, `blocked` or `released`, with its notes |
| `committed handoff worktree for U-001 (c1)` | the worker's files are on its branch, ready for the gate |
| `gate red integrating U-001: <command>` | the gate command that failed; nothing was published |
| `write-set violation integrating U-001: <paths>` | the work changed files the unit does not own |
| `U-001: built from text that has since changed on main: unit — it is rebuilt from the current text` | the spec moved while the unit was being built |
| `worker <holder> on U-001 exceeded the hard budget — killed; claim released` | `worker_timeout_minutes` |
| `U-001: 4 attempts did not land (last: rejected-gate) — set aside for this run` | the unit is left for a spec fix |
| `U-002 (c1): not started — <why>; set aside until the spec changes` | the unit's reading set does not resolve at its claim commit — a pin whose authority has moved, a consumed contract that is gone; its claim is given back |
| `claim lost for U-001: …` | another holder has it, or mainline was busy; tried again |
| `note: …` | things that are in order but worth knowing: a red batch being gated one unit at a time, a lost publish race being redone, a Test command that already fails at the head |
| `warning: …` | things to act on: a primary checkout that could not follow mainline, an attempt that could not be saved to its branch, a question that could not be recorded |

### The Report

The report comes last, exactly once. A build that finished:

```text
Complete.
  Progress: 2/2 Verified
  audit: exit 0 (clean)
  replay: exit 0 (consistent)
  §5 Test: exit 0 (true)
  journal tail:
    {"seq":9,"at":"2026-11-14T02:07:39Z","event":"verified","unit":"U-001",…}
    {"seq":10,"at":"2026-11-14T02:07:39Z","event":"audit","actor":"audit","findings":0,…}
```

A build that cannot go further:

```text
Blocked.
  Progress: 0/2 Verified · 2 Ready
  U-001: open Q-007: stub blocked on a spec gap
  U-002: dependency chain blocked: U-001 (Ready)
  journal tail:
    …
```

`Blocked.` gives every unfinished unit one line, the most specific cause known: `open Q-###: …` (a worker's or the judge's question), `its reading set does not resolve: …`, `set aside after N attempts that did not land; last outcome …`, ``failing gate `<command>`: <its last lines>``, `N attempt(s); last outcome <outcome> — <why>`, `dependency chain blocked: U-001 (Ready) ← …`, or `not attempted this run (status …)`. Above them come whatever is wrong with mainline itself: audit findings, a §5 command or the Test command failing at the head, a journal that no longer replays. With `--wait` the same report is printed as `Blocked (waiting).`, once per mainline head. [Run a Build](run-a-build.md#resolve-a-blocked-build) maps each blocker to its fix.

The final report is kept: its lines — from `Complete.` or `Blocked.` on — are also written to `.vitrify/logs/<job>/report.txt` and to the run record as `report`. However the run's output was captured, or lost, `vitrify run status` prints the report once the run has ended (see [Check a Run's Status](#check-a-runs-status)).

## Check a Run's Status

`vitrify run status` shows what a run is doing, unit by unit. It is read-only — it writes no file, takes no lock, and runs git only in a way that never holds a worker's index lock — so run it from a second terminal as often as you like:

```text
$ vitrify run status
run-20261116T140212Z-p48113 · alive · 23m elapsed · specs/calc · claude
Progress: 1/5 Verified · 3 Claimed · 1 Ready

UNIT   TRY  STAGE    TIME     OUTPUT         WORK
U-003  c1   editing  41m/45m  4s ago  41 KB  7 files +210 -14
U-004  c2   reading  2m/45m   1s ago  6 KB   no changes (retry: c1 rejected-gate)
U-005  c1   stamped  18m/45m  9s ago  88 KB  5 files +164 -2
U-002  c1   gate     9m                      rows 3/7 · pytest tests/test_parse.py (42s) +1 more
U-006  c1   queued   6m                      waiting for the gate

! U-003 c1: 41m of its 45m budget used

Finished
U-001  c1  verified       8m
U-004  c1  rejected-gate  11m  failing gate: `pytest tests/test_parse.py`

Logs: .vitrify/logs/run-20261116T140212Z-p48113/
```

The header gives the job, whether it is `alive`, `died` or `ended`, the SDD and the harness; then the progress line. The table lists each attempt in progress: its claim (`TRY`), its stage, the time it has run against its budget, how long ago its log last grew and how big it is, and what it has changed in its worktree so far. Lines starting with `!` are warnings: a worker that has printed nothing for `--quiet-after` seconds (default 300), one near its budget, a run that died. `Finished` lists the attempts that ended, with why. Once the run has ended, `vitrify run status` also prints its final report under `Report:`, the same lines the run printed last. `--json` prints the same as one object — the report as `report`, a list of lines, after `terminated` — and `--job <job>` shows an earlier run; [`vitrify run status`](../cli/command-reference.md#vitrify-run-status) lists every field.

Use this table to read the stages.

| Stage | Means |
| --- | --- |
| `reading` | the worker is running and has changed nothing yet (files a retry carried over do not count) |
| `editing` | it has changed files in its worktree |
| `stamped` | it has stamped the unit `Built` in its worktree; the report comes next |
| `exited` | its process has ended; the run has not dealt with it yet |
| `?` | its worktree could not be read; the reason follows |
| `queued` | it reported `built`; the work is waiting for the gate |
| `gate` | its work is in the gate; WORK shows the gate step, the command running and for how long |
| `integrating` | it reported `built`, and the run is no longer alive to gate it |

To keep watching, add `--follow`. It prints the view once, then one line per change, until the run ends:

```text
14:25:40  U-003 c1  editing → stamped
14:25:52  U-003 c1  reported built → waiting for the gate
14:26:01  gate      rows · U-002, U-003
14:27:44  U-002 c1  verified  10m
14:28:02  U-007 c1  dispatched → w5
14:31:10  U-007 c1  reading → editing  2 files +40 -0
14:36:15  U-004 c2  ! quiet 5m (no output since 14:31:15)
14:37:02  U-004 c2  active again
14:54:30  run       ended: complete · Progress: 5/5 Verified · exit 0
```

The follower exits with the run's own status — 0 complete, 2 blocked, 1 for an error, an interrupt or a run that died — so it can be chained. To be told when a run ends:

```sh
vitrify run status --follow; osascript -e 'display notification "run ended" with title "vitrify"'
```

`--follow --since-start` first replays what the run recorded in its event log, `.vitrify/logs/<job>/events.jsonl`, from its start: dispatches, gate steps, outcomes, the end; a run with no event log replays nothing. With `--json` the follower prints one event per line instead, replayed ones marked `"replay": true`.

In a repository where no run has been recorded, `vitrify run status` exits 4: `error: no run recorded in this repository → start and watch one: vitrify brief run`.

### Watch From an Agent Session

A coding agent operating a run follows `vitrify brief run`. It relies on its harness's completion notification — for a run the human started, it runs `vitrify run status --follow` in the background, which exits with the run's status. Where the harness cannot notify, it checks `vitrify run status` at most once per `run.status_check_seconds` (270 seconds by default; see [Configuration](../cli/configuration.md)), because each check re-sends its whole conversation, and 270 seconds keeps each re-read inside a model provider's five-minute prompt cache.

## Read the Run Record

`.vitrify/run.json` is one JSON document describing the latest run in the repository. It is rewritten — whole, atomically — when the run starts and at every change: a dispatch, a worker ending, an integration, the end. A copy is kept per run at `.vitrify/logs/<job>/run.json`, so earlier runs stay readable. It is a snapshot for people and scripts: vitrify never decides anything from it. Use this table to read its fields.

| Field | Holds |
| --- | --- |
| `job_id`, `pid`, `started_at` | the run: its ID, its process, when it began |
| `sdd` | the SDD being built, as `--sdd` names it |
| `harness` | the harness the workers run, such as `claude` (`""` when there is none) |
| `params` | `cap`, `gate_jobs`, `lease_minutes`, `worker_timeout_minutes`, `abort_failed_waves` — as in effect |
| `worktrees` | the worktree root |
| `workers[]` | every worker dispatched, in order: `holder`, `pid` (also its process group), `unit`, `claim_seq`, `branch`, `worktree`, `started_at`, `ended_at`, `outcome`, `retry` (`true` when the attempt started from an earlier one's files), `detail` (why the attempt ended: the first line of the reason; `null` while it runs and for `verified`) |
| `reports[]` | each worker's report line, as received |
| `escalations[]` | `evidence_path`, `verdict` |
| `terminated` | absent while the run lives; then `state` (`complete`, `blocked`, `interrupted`, `error`), `at` and `detail` (for a Blocked run, its progress line and that the report names each blocker) |
| `report` | the final report's lines, from `Complete.` or `Blocked.` on; `null` until the report is printed |

A worker's `outcome` is where its attempt stands. Use this table to read it.

| Outcome | Means |
| --- | --- |
| `running` | the session is in progress |
| `integrating` | it reported `built`; the work is waiting for the gate, or in it |
| `verified` | the work landed |
| `rejected-gate` | a gate command failed on the merged tree |
| `rejected-write-set` · `rejected-not-built` | it changed files the unit does not own · it was handed over without a Built stamp |
| `stale-inputs` | the spec text it was built from changed |
| `blocked` | the worker stopped at an open question (`Q-###`) |
| `released` | the worker gave up, or left no report that could be read |
| `timeout` · `lease-lost` | stopped at the hard budget · its lease could not be renewed |
| `rejected-fencing` · `skipped` · `lost-race` · `ownership-conflict` | its claim was superseded · its lease ran out before the gate · mainline kept moving · the merge conflicted |
| `interrupted` · `aborted` · `error` | the run ended first: by a signal · by the judge's `abort` · by an error |

Query it with `jq`:

```sh
jq -r '.workers[] | "\(.unit) c\(.claim_seq)  \(.outcome)  \(.started_at)"' .vitrify/run.json
jq '[.workers[] | select(.outcome == "running" or .outcome == "integrating")]' .vitrify/run.json
jq '.terminated // "still running"' .vitrify/run.json
```

## Follow a Worker Session

Each run has a directory, `.vitrify/logs/<job>/`, holding its copy of the run record, its event log, the gate's progress, its final report, three files per worker and one per escalation. Use this table to find the file you need.

| File | Holds |
| --- | --- |
| `worker-<job>-w<i>.out` | the session's stdout, written by the harness itself as it goes. The last line of the session's final text is the report |
| `worker-<job>-w<i>.err` | its stderr |
| `worker-<job>-w<i>.prompt.md` | exactly what the worker was started with: the job block, the procedure, the "This session" note (what was already done for it; what an earlier attempt left and why it failed) and the inlined reading set |
| `events.jsonl` | the run's event log: one JSON object per line — dispatches, gate steps, outcomes, the end — appended as they happen. `vitrify run status --follow --since-start` replays it |
| `gate.json` | what the integrator is doing now: its step, the batch, the commands running. Rewritten at every change |
| `report.txt` | the final report, written when the run prints it: the status line, the progress line, each blocker, the journal tail |
| `escalation-<n>.json` | what the judge was shown — the unit, the failing command, the last 100 lines of its output, the journal tail — every reply it gave, and the verdict |

Follow a session as it happens:

```sh
tail -f .vitrify/logs/<job>/worker-<job>-w1.out
```

Every harness spelling the probe records streams the session — one JSON event per line, readable at any moment and complete up to the moment a worker stops, however it stops. For claude's stream, this prints what the worker says as it says it:

```sh
tail -f .vitrify/logs/<job>/worker-<job>-w1.out \
  | jq -r 'select(.type == "assistant") | .message.content[]? | select(.type == "text") | .text'
```

The work itself is in the worker's worktree, `<worktrees>/<sdd>-<unit>-c<seq>` (the `worktree` field of its `run.json` entry). See what it has written so far:

```sh
git -C <worktree> status --short
git -C <worktree> diff
```

> [!CAUTION]
> Look, do not touch: whatever is in that directory when the worker reports is committed as the unit's work.

`git worktree list` shows every checkout at once: the primary, the integration checkout, one worktree per unit being built, and the run's own throwaway checkouts under `.vitrify/tx-*`. vitrify never deletes logs: `.vitrify/logs/` keeps one directory per run until you remove it.

## Inspect the Gate

One gate runs at a time, in the integration checkout `<worktrees>/mainline`, and it prints nothing while it passes. `vitrify run status` shows its step (`rows 3/7`, `test`, `publish`, …), the batch, the command running and how long it has run, from `.vitrify/logs/<job>/gate.json`. These are the other things you can see:

- **Which units are in it or waiting for it:** the workers whose `outcome` is `integrating`. `integrated U-### (c<n>)` is the line that ends the wait.
- **The tree being judged:** `git -C <worktrees>/mainline log --oneline main..HEAD` lists the merges under test that mainline does not have yet.
- **The commands running now:** the run's child processes — `pgrep -P <run pid> -lf` (`-a` on Linux), the pid being `run.json`'s — are one harness process per running worker and one `sh -c <command>` per gate command in progress: up to `run.gate_jobs` at once for Verification rows, one at a time for the rest.
- **A command's output:** not echoed. If the command fails, its last lines are kept ([below](#find-why-an-attempt-did-not-land)); if it passes, nothing is. While it runs, its output accumulates in a file beside its temp directory, `<system temp>/vfy-<hash>/c<n>.out`, removed when the command ends — the place to look at a command that seems stuck.

## Find Why an Attempt Did Not Land

An attempt that does not land is recorded five times over, from terse to complete:

1. **The stderr line** when it happens: `gate red integrating U-001: <command>`, `write-set violation integrating …`, `report [w1] U-001: blocked — …`.
2. **`run.json`:** the worker's `outcome`, and its own account in `reports[]`.
3. **Its branch.** The work is kept on `sdd/<sdd>/<unit>/c<seq>` under a commit whose message is the reason — for a red gate, the command and the last 40 lines of its output. A branch whose work landed is deleted; what remains is what did not.
4. **The next worker's prompt** (`worker-…-w<i>.prompt.md`, under "This session"), which quotes that reason and lists the files carried over.
5. **`escalation-<n>.json`**, once the unit's work has failed `abort_failed_waves` times: the evidence the judge saw and its verdict.

Inspect the branches of attempts that were set aside:

```sh
git branch --list 'sdd/<sdd>/*'
git log -1 --format=%B sdd/<sdd>/U-001/c1
git diff main...sdd/<sdd>/U-001/c1 --stat
```

The first lists the attempts set aside and those in progress; the second prints the reason (`sdd: abandoned attempt U-001 (c1) — rejected-gate …`); the third shows what the attempt had written.

A `blocked` worker's question is also landed in the SDD's Decision Log (`decisions.md`, or `## Decisions & Open Questions` in a single file) as `- **Q-007** (<date>, open) — reported by <holder> on U-001: <notes>`, and the final report names every unfinished unit's blocker. [`vitrify questions --open`](../cli/command-reference.md#vitrify-questions) lists what is still open, with the units each question holds up.

## Check Progress and State

The state of the build is the units' front matter on mainline. These three commands give three views of it:

```sh
vitrify ledger progress [--sdd P]
vitrify locate --units [--sdd P]
cat <sdd-root>/ledger.md
```

`ledger progress` prints one line, such as `Progress: 1/2 Verified · 1 Claimed`. `locate --units` prints one line per unit, such as `U-001    Claimed   group=a   seq=1`, and marks each unit that could be started now as `claimable`; with `--json` it adds each dependency's status, the holder (`claimed_by`), the lease expiry and whether the lease is still live — who holds what, for a script. The ledger holds the table:

```text
| Unit  | Status   | Group | Built @ | Verified @ | Note                          |
| ----- | -------- | ----- | ------- | ---------- | ----------------------------- |
| U-001 | Claimed  | a     | —       | —          | lease to 2026-11-14T03:08:10Z |
| U-002 | Verified | a     | 1.0.0   | 1.0.0      |                               |
```

In the ledger, `Note` carries a Claimed unit's lease expiry; a single-file SDD keeps the same table under its `## State Ledger` heading.

These verbs read the checkout they are run in. Under `vitrify run` the primary checkout follows mainline commit by commit, so run them there, naming the SDD as you named it to `run`. If the checkout is on another branch, or is behind — the run says so when it cannot move it, and `.vitrify/primary-behind` exists for as long as it is — read mainline itself:

```sh
git show main:<sdd-root>/ledger.md
```

To keep an eye on it: `while sleep 60; do vitrify ledger progress; done`.

## Read the History

Every change of state is a commit on mainline, so `git log` is the timeline of the build:

```sh
git log --first-parent --format='%h %cd  %s' --date=format:%H:%M:%S main
```

Use this table to read the commit subjects.

| Subject | Written when |
| --- | --- |
| `sdd: claim U-001 (c1) by <holder>` | a worker is about to start on the unit |
| `sdd: renew U-001 (c1) by <holder>` | its lease is extended: a long session, or work waiting for the gate |
| `sdd: release U-001 (c1) by <holder>` | the attempt did not land; the unit is back in the pool |
| `sdd: integrate U-001 (c1)` | the unit's branch is merged (it carries `sdd: build U-001 (c1)`, the worker's files) |
| `sdd: verify U-001 (c1), U-002 (c1)` | the batch passed the gate: stamps, journal events and ledger, in one commit |
| `sdd: record Q-007 (blocked U-001)` | a worker's question, landed in the Decision Log |
| `sdd: audit --fix-leases --apply-stale` | housekeeping: lapsed leases released, units staled by a spec change marked |

The journal — `<sdd-root>/journal.ndjson`, or the block under `## Journal` in a single file — is the same history as data, one event per line: `claim`, `renew`, `release`, `built`, `verified`, `stale`, `audit`, each with its `seq`, time, unit, actor, claim number and commit. The final report ends with its last five lines. To follow it (`-F`, because the file is replaced at every commit):

```sh
tail -F <sdd-root>/journal.ndjson
```

## Run Health Checks

These checks are read-only and safe at any time, from the primary checkout:

- `vitrify check` — every check below that reads spec state, in one call: lint, the audit, the journal replay, the ledger check, and the pin states. Each finding names its fix; exit 0 when all are clean.
- `vitrify journal replay` — the journal and the front matter agree (exit 0; 3 names each mismatch).
- `vitrify audit --skip-rotten` — the drift checks that execute nothing. Plain `vitrify audit` also re-runs every Verified unit's Verification commands in that checkout — the commands the gate runs, so expect the load.
- `vitrify trace verify --unit U-###` — whether a Built or Verified unit's recorded inputs still match the spec; exit 3 names what changed (its own text, a contract, the global regions).
- `vitrify lint --built` — the authoring checks a run preflights with: everything but L7, so that an SDD that is being built can still be held to the rest — a pin in step with its authority, a `Consumers:` line that is true, a version that has its change entry.
- `vitrify pin list` — every external contract, and whether each pin still matches its authority.
- `vitrify ledger regen --check` — the ledger on disk is what unit front matter derives (exit 0; 3 names the first line that differs).
- `vitrify run --dry-run [--sdd P]` — what a run started now would do: the whole preflight, the plan, the claims it would recover from a run that died, lapsed leases, a primary checkout that is behind. It publishes nothing and moves no file, and is safe beside a run that is going.

`check`, `audit`, `lint`, `journal replay`, `trace verify`, `locate`, `pin` and `questions` each take `--json`. In a repository with several SDDs, `check`, `lint`, `audit`, `journal replay`, `ledger`, `pin` and `questions` take `--all`: every SDD in turn, each line under its SDD's name — the whole repository's gate in one command:

```sh
vitrify check --all
```

## Tell Whether a Run Is Alive

A run is in one of three conditions. Use these signs to tell which.

- **Alive:** `run.json` has no `terminated`, and its `pid` is a live process (`kill -0 <pid>`). A run also holds a lock on `.vitrify/jobs/<job>.lock` for as long as it lives — that, not the pid, is what the next run tests.
- **Died:** no `terminated`, and the process is gone (killed outright, power lost). Nothing needs repairing by hand: the next `vitrify run` releases its claims, keeps its workers' files on their branches, and names any worker that outlived it with the `kill` command that stops it.
- **Ended:** `terminated.state` and `detail` say how, and so does the exit status:

| Exit | Means |
| --- | --- |
| 0 | `Complete.` |
| 2 | `Blocked.`, a preflight failure, or an `abort` verdict |
| 1 | an error — the message is the last thing on stderr |
| killed by a signal | interrupted, after cleaning up; a shell shows 130 (SIGINT), 143 (SIGTERM) or 129 (SIGHUP) |

## What Is Not Shown

These are the limits of what a run exposes.

- A gate command's output is not shown while it runs, and a gate that passes leaves none. `vitrify run status` shows the gate's step and the command running; for its output, see [Inspect the Gate](#inspect-the-gate).
- The stages inside a unit (`reading`, `editing`, `stamped`), work counts and quiet spells are seen only while a follower is watching; `--since-start` cannot replay them.
- What the agent is thinking, or which tool it is using, is not shown by vitrify; it is in the harness's own log, `worker-<holder>.out`.
- `run.json` describes the latest run only; earlier ones are in `.vitrify/logs/<job>/run.json`.
- vitrify does not count tokens or cost. Whatever the harness itself reports about a session is in the last events of that worker's log.

## Next Steps

- [Run a Build](run-a-build.md#resolve-a-blocked-build) — act on a `Blocked.` report.
- [Troubleshooting Guide](../resources/troubleshooting.md) — find a symptom and its fix.
- [Runs](../about/concepts/runs.md) — what produces each line you see.
