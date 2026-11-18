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

In a repository with several SDDs, name the SDD: `--sdd <root>` on `run`,
and build SDDs one at a time, in dependency order.

## 1. Preflight

```
vitrify run --dry-run --sdd <root>
```

Exit 0 means ready; its last line, `Start:`, is the command that starts the
run it checked. Exit 2: each `preflight failed:` line ends `→ <fix>`. Do
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
nohup vitrify run --sdd <root> > run.out 2> run.err &
```

To start it from the session, use the harness's background execution, which
notifies on completion: Claude Code's background Bash, OpenCode's background
shell calls, Codex's background terminals. **Never a foreground call**: the
run outlasts the tool's time limit.

## 3. The header

The run's first lines name the job (`run-<UTC>-p<pid>`), the plan and the
parameters, then `Watch: vitrify run status --job <job> [--follow]`. Note
the job id; it names the run's logs, `.vitrify/logs/<job>/`.

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

`Blocked.` lists what is wrong with mainline itself first — fix that
first — then one line per unfinished unit:

| Blocker line | Action |
| --- | --- |
| `open Q-###: …` | hand it to the human (`vitrify questions --open` lists the units it holds up); the answer lands through `vitrify brief change` |
| `its reading set does not resolve: …` | a spec fix through `vitrify brief change` (`vitrify pin sync` for a pin whose authority moved) |
| `set aside after N attempts that did not land; …` | read why: the attempt's branch commit message, `.vitrify/logs/<job>/`; a spec cause goes through `vitrify brief change`, anything else to the human |
| ``failing gate `<command>`: …`` | run the command yourself from the repository root; fix the spec through `vitrify brief change`, or hand an environment cause back |
| `N attempt(s); last outcome <outcome> — <why>` | act on the reason it gives |
| `dependency chain blocked: U-### (…) ← …` | act on the unit at the root of the chain |
| `not attempted this run (status …)` | check it with `vitrify locate --units` |

`!` warnings in `vitrify run status`:

| Warning | Action |
| --- | --- |
| `U-### cN: no output …` | a quiet worker: read its log `.vitrify/logs/<job>/worker-<job>-w<i>.out`; if every worker is quiet and nothing lands, hand it back: usually harness permissions, which the human grants and re-probes with `vitrify probe` |
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
* Start a second run of the same SDD.
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
