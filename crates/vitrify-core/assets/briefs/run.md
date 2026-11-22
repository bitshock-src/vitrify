# Run — start, watch and read a `vitrify run` build

Operate a `vitrify run` build from an agent session: check it can start,
have it started where it survives, wait for it without spending turns, and
act on how it ended. The run builds; you watch and report.

Use when: an Active SDD is ready to build (or rebuild after a change), or a
run the human started needs watching or reading.
Not for: driving the build loop yourself (`vitrify brief kickoff`), building
one unit (`vitrify brief build-unit`), or fixing the spec
(`vitrify brief change`).
Version: vitrify {version} — every command below is this binary's.

With one SDD, `vitrify run` builds it. With several, a plain `vitrify run`
is a **project run**: every SDD, one unit graph, a lane per SDD. A unit
starts when the units it depends on are built, in its own SDD or another
(a pin in `depends_on` stands for the units behind it). Free worker slots go
to the claimable unit, in any SDD, with the longest chain of work behind it
(critical path first), over one worker pool and one gate pool. After a
change a run regenerates only the units whose spec inputs changed,
re-checks only those whose code inputs changed, and keeps every result it
can still trust.

* `--sdd <root>` builds that one SDD alone, as a single-SDD run; add
  `--upstream` for the SDDs it depends on too, each a lane of one project
  run. Units that wait on SDDs outside the selection end as blockers that
  name them.
* `--recheck` distrusts every recorded check: every Verified unit's rows
  and every SDD's §5 Test command run again. It regenerates nothing.
* `--dry-run` prints the plan of a project run: what regenerates, what is
  adopted, what is re-checked and why, the lane states and the critical
  path. A critical path that dominates means a larger `--cap` will not
  shorten the build.
* A lane is `idle` (waits on units elsewhere), `build`, `recheck` (rows
  and the Test command, no workers), `up to date`, `complete` or
  `blocked`. A lane whose own preflight fails is `blocked` from the start
  (`[<slug>] preflight failed: …`); the other lanes go on.
* A 2.0 SDD whose units consume pins is refused (exit 4) naming it:
  migrate it first (`vitrify brief change`, §6).

## 1. Preflight

```
vitrify run --dry-run
```

Exit 0 means the preflight passed; its last line, `Start:`, is the command
that starts the run it checked. In a project run a lane whose own checks
fail does not fail the preflight: it is listed `blocked` in the plan, with
a `[<slug>] preflight failed: …` line on stderr, so read those lines too.
Exit 2: each `preflight failed:` line ends `→ <fix>`. Do
the fixes that are mechanical (a stale pin, an unrecorded change entry,
uncommitted spec files) and re-run; hand design questions back, and machine
setup — a fix that names `vitrify init` or `vitrify probe` — too.

## 2. Who starts it

A build can take hours, and a run started from a session ends with the
session: harnesses stop background commands when the task stops or the
harness exits. vitrify stops cleanly on a signal and the next run resumes,
so nothing is lost, but a long build is best started by the human, in their
own terminal:

```
nohup vitrify run > run.out 2> run.err &
```

To start it from the session, use the harness's background execution, which
notifies on completion: Claude Code's background Bash, OpenCode's background
shell calls, Codex's background terminals. **Never a foreground call**: the
run outlasts the tool's time limit.

## 3. The header

The run's first lines name the job (`run-<UTC>-p<pid>`), what it will do
and its parameters, then a `Watch:` line: `vitrify run status --job <job>
[--follow]` for one SDD, `vitrify run status --follow` for a project run,
whose header is `Project build job — <n> SDDs: <lane states>  [<job>]`,
`Plan:`, `Critical path:` and `Capacity:`. Note the job id; it names the
run's logs, `.vitrify/logs/<job>/`, with each lane's under `<slug>/`.

## 4. Wait without spending turns

Every check re-sends the whole conversation, so check rarely:

* Rely on the harness's completion notification when you started the run.
* For a run the human started, run `vitrify run status --follow` in the
  background: it exits when the run ends, with the run's status.
* Where the harness cannot notify, run `vitrify run status` at most once
  every **{status_check_seconds} seconds** (`run.status_check_seconds`).
  Never poll in a loop or sleep in the foreground.
* Run `vitrify run status` whenever the human asks: the unit-by-unit
  snapshot, the gate, and `!` warnings.

## 5. The result

Once the run has ended, `vitrify run status` prints its final report under
`Report:` (also in `.vitrify/logs/<job>/report.txt`).

| Exit | Means |
| --- | --- |
| 0 | `Complete.` — every non-Retired unit Verified; audit, replay and §5 Test clean |
| 2 | `Blocked.`, a preflight failure, or an `abort` verdict |
| 1 | an error (its message is the last stderr line); from the follower also a run interrupted or dead |
| 4 | a 2.0 SDD whose units consume pins; the message names it |

In a project run the report is `Complete.` or `Blocked.` followed by one
line per SDD (its state, Verified count, what was built or re-checked, or
what blocks it), each further blocker indented below as `<slug> <unit> …`.
A blocked SDD's units that wait on another SDD's blocker say so: act on
the unit at the root. A single-SDD run prints the `Complete.` or
`Blocked.` report of that one SDD instead: the progress line, each
blocker, and the journal tail.

`Blocked.` lists what is wrong with mainline itself first — fix that
first — then one line per unfinished unit:

| Blocker line | Action |
| --- | --- |
| `open Q-###: …` | hand it to the human (`vitrify questions --open` lists the units it holds up); the answer lands through `vitrify brief change` |
| `its reading set does not resolve: …` | a spec fix through `vitrify brief change` (`vitrify pin sync` for a pin whose authority moved) |
| `set aside after N attempts that did not land; …` | read why: the attempt's branch commit message, the SDD's logs `.vitrify/logs/<job>/<slug>/`; a spec cause goes through `vitrify brief change`, anything else to the human |
| ``failing gate `<command>`: …`` | run the command yourself from the repository root; fix the spec through `vitrify brief change`, or hand an environment cause back |
| `N attempt(s); last outcome <outcome> — <why>` | act on the reason it gives |
| `dependency chain blocked: U-### (…) ← …` | act on the unit at the root of the chain |
| `waits on <sdd> U-###, outside this run …` | build that SDD too (`--upstream`, or `vitrify run --sdd <root>` on it), then run this one again |
| `rotten — row N: … fails on mainline too; <culprit>` (project run: `U-### rotten: <culprit>`) | something already on mainline broke the row, not this run: hand the culprit line (`… row N broke at <sha> (core U-004 integrated) — culprit found`) to the human; the unit is never rebuilt for it. |
| `waits on U-### (rotten)` | act on the rotten unit |
| `not attempted this run (status …)` | check it with `vitrify locate --units` |

While the run goes, a `<sdd> U-### row N is flaky: no commit since its last
check broke it` line is not a blocker: the row passes and fails with no
change behind it, the unit is checked again, and the row is for the human
to fix.

`!` warnings in `vitrify run status`:

| Warning | Action |
| --- | --- |
| `U-### cN: no output …` | a quiet worker: read its log `.vitrify/logs/<job>/<slug>/worker-<job>-w<i>.out`; if every worker is quiet and nothing lands, hand it back: usually harness permissions, which the human grants and re-probes with `vitrify probe` |
| `U-### cN: … of its … budget used` | nothing yet; the run stops the worker at its budget and retries the unit |
| `the run (pid N) is gone and recorded no end …` | run each printed `kill -9 -- -<pgid>` line, then start `vitrify run` again |

## 6. Stop and resume

Interrupt the run, never its workers: Ctrl-C, or `kill -INT <pid>` (the pid
is in the job id and in `vitrify run status --json`). The run stops its
workers and releases their claims. Running `vitrify run` again resumes from
what mainline says; nothing landed is lost.

## Never

* Write a unit's code, claim a unit, or edit status, traces, the journal or
  the ledger.
* Start a second run over the same SDD or SDDs.
* Report success before `Complete.`.
* Fix the spec outside `vitrify brief change` — a fix is a commit to
  mainline, which a running build picks up.

## Hand back to the human

* An open `Q-###` — design authority is theirs.
* A unit set aside after repeated attempts, when the cause is not a spec
  defect you can name.
* A §5 or gate command failing for a reason outside the spec (machine,
  network, credentials).
* Workers that make no progress — usually harness permissions.
* Machine setup: a preflight fix or an error that names `vitrify init` or
  `vitrify probe`.

## Report

The final status line (`Complete.` or `Blocked.`) and the exit status, the
progress line, and per blocker the action taken or needed (and by whom).
