//! Each command's `--help` block, printed after its options: `Reads:`,
//! `Writes:`, `Prints:`, `Exit status:`, `Config:`, `Examples:` and
//! `Procedure:`, in that order, the last three where they apply.

pub const LOCATE: &str = "\
Reads:   SDD discovery (specs/*/sdd.md, specs/*.sdd.md, then the files git would commit);
         with --units, the SDD's units and their front matter
Writes:  nothing
Prints:  one line per SDD: root, shape, slug, version, status, unit count
         --json: [{root, shape, sdd, title, version, status, scope_roots, units}]
         --units: one line per unit: status, parallel group, claim_seq, claimable
         --units --json: {sdd, shape, version, status, units: [{unit, status, parallel_group,
           depends_on, claimed_by, claim_expires, claim_seq, lease_live, claimable, file}]}
Exit status: 0 listed · 4 no SDD; with --units, several and no --sdd · 2 usage error
         An SDD of a schema version this vitrify does not read is still listed.
Examples:
  vitrify locate
  vitrify locate --units --sdd specs/calc
  vitrify locate --units --json";

pub const READSET: &str = "\
Reads:   the core file's Ground Rules and Technology Decisions; the unit's entry; each
         consumed contract, and the body a pinned contract stands for
Writes:  nothing
Prints:  one path per line in reading order, `  ::  <region>` for a region of a shared file
         --json: {unit, sdd, shape, consumes, readings: [{path, region}]}
Exit status: 0 resolved (and within budget) · 2 over the --budget limit, or a usage error
         · 4 no SDD, unknown unit, a missing section or contract, a pin that does not
         resolve or whose hash is not its authority's
Config:  lint.readset_budget_bytes (VITRIFY_LINT_READSET_BUDGET_BYTES), for a bare --budget
Examples:
  vitrify readset --unit U-002 --sdd specs/calc
  vitrify readset --unit U-002 --json
  vitrify readset --unit U-002 --budget
Procedure: vitrify brief build-unit (workers) · vitrify brief authoring (sizing a unit)";

pub const CLAIM: &str = "\
A protocol verb for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD.
Reads:   the mainline head (checked out into a throwaway .vitrify/tx-<pid>-<n> worktree);
         the unit's front matter there
Writes:  a published mainline commit `sdd: claim <unit> (c<seq>) by <holder>`: status Claimed,
         claimed_by, claim_expires = now + lease, claim_seq + 1; a claim event; the ledger.
         The primary checkout is fast-forwarded when it has mainline checked out.
Prints:  one JSON line: {unit, op, holder, claim_seq, claim_expires, commit, mainline}
Exit status: 0 published · 1 git failure · 2 usage error · 4 unit missing or not claimable
         (the reason is named) · 5 lost the publish race after 3 attempts
Config:  mainline (VITRIFY_MAINLINE), after --mainline; --lease-minutes defaults to 60
         and does not read run.lease_minutes
Examples:
  vitrify claim --unit U-003 --holder w1 --sdd specs/calc
  vitrify claim --unit U-003 --holder w1 --lease-minutes 90 --mainline main
Procedure: vitrify brief build-unit · vitrify brief kickoff";

pub const RENEW: &str = "\
A protocol verb for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD.
Reads:   the mainline head (throwaway .vitrify/tx-<pid>-<n> worktree); the unit's lease
Writes:  a published mainline commit: claim_expires = now + lease; a renew event; the ledger
Prints:  one JSON line: {unit, op, holder, claim_seq, claim_expires, commit, mainline}
Exit status: 0 published · 1 git failure · 2 usage error · 4 unit missing, or not Claimed
         by this holder · 5 lost the publish race after 3 attempts · 6 the lease has
         already expired (renew refused: claim again)
Config:  mainline (VITRIFY_MAINLINE), after --mainline; --lease-minutes defaults to 60
Examples:
  vitrify renew --unit U-003 --holder w1 --sdd specs/calc
Procedure: vitrify brief build-unit";

pub const RELEASE: &str = "\
A protocol verb for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD.
Reads:   the mainline head (throwaway .vitrify/tx-<pid>-<n> worktree); the unit's claim
Writes:  a published mainline commit: status Ready, holder and expiry cleared, claim_seq kept;
         a release event; the ledger
Prints:  one JSON line: {unit, op, holder, claim_seq, claim_expires, commit, mainline}
Exit status: 0 published · 1 git failure · 2 usage error · 4 unit missing, or not Claimed
         by this holder · 5 lost the publish race after 3 attempts
Config:  mainline (VITRIFY_MAINLINE), after --mainline
Examples:
  vitrify release --unit U-003 --holder w1 --sdd specs/calc
Procedure: vitrify brief build-unit · vitrify brief integrate";

pub const TRACE: &str = "\
Protocol verbs for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD.
Subcommands: record (write a unit's trace and stamp it Built or Verified) · verify (compare
         a unit's recorded trace with its current inputs)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify trace record --unit U-003 --stamp built --sdd specs/calc
  vitrify trace verify --unit U-003 --json";

pub const TRACE_RECORD: &str = "\
A protocol verb for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD. A change entry's re-stamps are vitrify change apply's.
Reads:   the unit's entry, its consumed contracts, Ground Rules and Technology Decisions;
         git HEAD for the default commit
Writes:  the working tree: the unit's trace map; built sets status Built and
         built: <date> @ SDD <version>, commit <sha>; verified sets status Verified and
         verified:, and clears claimed_by and claim_expires. No journal event, no ledger.
Prints:  one JSON object: {unit, recorded, stamp, file}
Exit status: 0 recorded · 2 usage error · 4 no SDD, the unit or a consumed contract missing
Examples:
  vitrify trace record --unit U-003 --stamp built --sdd specs/calc
  vitrify trace record --unit U-003 --stamp verified --commit 3f2a9c1
Procedure: vitrify brief build-unit · vitrify brief integrate";

pub const TRACE_VERIFY: &str = "\
Reads:   the unit's recorded trace and its current inputs
Writes:  nothing
Prints:  `U-###: trace matches`, or `U-###: STALE — mismatched keys: …`
         --json: {unit, match, mismatches: [{key, recorded, current}]}
Exit status: 0 matches · 3 a key differs · 4 no SDD, unknown unit, or no recorded trace
         · 2 usage error
Examples:
  vitrify trace verify --unit U-003 --sdd specs/calc
  vitrify trace verify --unit U-003 --json
Procedure: vitrify brief integrate";

pub const LEDGER: &str = "\
Subcommands: progress (the one-line progress summary) · regen (rewrite or --check the
         derived ledger; a protocol verb for the integrator and authored baselines, not
         run by hand while `vitrify run` builds the SDD)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify ledger progress --all
  vitrify ledger regen --check --sdd specs/calc";

pub const LEDGER_REGEN: &str = "\
A protocol verb for the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD.
Reads:   unit front matter; with --check, the ledger on disk
Writes:  the working tree: ledger.md (bundle) or the ## State Ledger section (single file);
         nothing with --check
Prints:  the progress line, and the written path on stderr
         --check: `ledger current (<path>)`, or the first differing line, derived and found
Exit status: 0 written, or current · 3 --check found a difference, or no ledger · 4 no SDD,
         or several and no --sdd · 2 usage error
Examples:
  vitrify ledger regen --sdd specs/calc
  vitrify ledger regen --all --check
Procedure: vitrify brief integrate";

pub const LEDGER_PROGRESS: &str = "\
Reads:   unit front matter
Writes:  nothing
Prints:  Progress: <V>/<total> Verified · <n> <Status> · … (Retired not in the total);
         with --all one line per SDD, prefixed with its slug
Exit status: 0 printed · 4 no SDD, or several and no --sdd · 2 usage error
Examples:
  vitrify ledger progress --sdd specs/calc
  vitrify ledger progress --all";

pub const JOURNAL: &str = "\
Subcommands: append (add one event; a protocol verb for workers, the integrator and
         authored baselines, not run by hand while `vitrify run` builds the SDD) · replay
         (check the journal against front matter)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify journal replay --all
  vitrify journal append --event built --unit U-003 --sdd specs/calc";

pub const JOURNAL_APPEND: &str = "\
A protocol verb for workers, the integrator and authored baselines; not run by hand while
`vitrify run` builds the SDD. A change entry's events are vitrify change apply's.
Reads:   the journal's last seq; the SDD's version
Writes:  the working tree: one line appended to journal.ndjson (bundle) or the ## Journal
         block (single file), created on first use; seq = last + 1, at = now (UTC)
Prints:  the appended event line
Exit status: 0 appended · 2 usage error, or a kind other than change or audit without --unit
         · 4 no SDD, or several and no --sdd
Examples:
  vitrify journal append --event built --unit U-003 --actor w1 --claim-seq 2 --sdd specs/calc
  vitrify journal append --event audit --findings 0
Procedure: vitrify brief integrate · vitrify brief kickoff";

pub const JOURNAL_REPLAY: &str = "\
Reads:   the journal (journal.ndjson, or the ## Journal block) and unit front matter
Writes:  nothing
Prints:  `journal consistent (N events)`, or one `order: …` or
         `mismatch U.field: journal projects …, front matter says …` line per problem;
         --all prefixes each line with the slug, `calc: no journal yet` where none
         --json: {events, consistent, order_findings, mismatches, projection};
         --all: an array of the same, each led by sdd and root
Exit status: 0 consistent · 3 an order finding or mismatch · 4 no SDD, several and no --sdd,
         or no journal (an SDD with no journal does not fail --all) · 2 usage error
Examples:
  vitrify journal replay --sdd specs/calc
  vitrify journal replay --all --json
Procedure: vitrify brief kickoff";

pub const AUDIT: &str = "\
Reads:   the SDD, its journal, the working tree under scope_roots; check 4 runs Verified
         units' Verification commands (sh -c from the repository root)
Writes:  nothing by default. --fix-leases, --apply-stale and --journal edit the working tree:
         unit front matter, journal events, the ledger
Prints:  `audit clean (7/7 checks)`, or one `[check N] subject: cause → remediation` line
         per finding; --all prefixes each line with the slug
         --json: {clean, findings: [{check, subject, cause, remediation}]};
         --all: an array of {sdd, root, clean, findings}
Exit status: 0 clean · 2 a finding (a fixing run still exits 2: re-run to confirm), or a
         usage error · 4 no SDD, or several and no --sdd
Config:  run.gate_jobs (VITRIFY_RUN_GATE_JOBS), for check 4
Examples:
  vitrify audit --sdd specs/calc
  vitrify audit --all --skip-rotten --json
  vitrify audit --apply-stale --journal --sdd specs/calc
Procedure: vitrify brief kickoff · vitrify brief integrate";

pub const BRIEF: &str = "\
Reads:   ~/.vitrify/briefs/<task>.md ($VITRIFY_HOME; an override on disk wins); build-unit
         with --unit --holder also reads the SDD, the unit's reading set and its claim
Writes:  nothing in the repository; restores or refreshes its copy under ~/.vitrify/
Prints:  no task: the procedure index (task, what it is for, who follows it)
         a task: that procedure, filled with this vitrify's version and values
         build-unit --unit --holder: the whole worker prompt vitrify run hands a worker
Exit status: 0 printed · 2 --unit and --holder not given together, or not to build-unit,
         or a usage error · 4 unknown unit, no SDD, or several and no --sdd
Config:  run.status_check_seconds (VITRIFY_RUN_STATUS_CHECK_SECONDS), in brief run;
         run.worktree_dir (VITRIFY_RUN_WORKTREE_DIR); run.lease_minutes
         (VITRIFY_RUN_LEASE_MINUTES), in the worker prompt; VITRIFY_HOME
Examples:
  vitrify brief
  vitrify brief change
  vitrify brief build-unit --unit U-003 --holder w1 --sdd specs/calc";

pub const INIT: &str = "\
Sets up this machine, once: no other command creates ~/.vitrify/, and every other command
refuses until this has run. Not a repository's step: run it in your own terminal.
Reads:   git and the harness CLIs on PATH (claude, codex, opencode: one real model call
         each, and one per role that adds flags)
Writes:  ~/.vitrify/config.toml — every setting at its default, the harness role profiles,
         and the probe record (the working streaming spellings); materializes the briefs,
         schemas and templates under ~/.vitrify/
Prints:  one `status: text` line per check, status ok, FAIL, untested or note, ending
         `ok: recorded the probe in ~/.vitrify/config.toml`
         --json: {green, checks: [{status, text}], harnesses: [{name, present, version,
           headless, failure}]}
Exit status: 0 set up, nothing failed · 2 a FAIL (git missing, a harness that does not
         accept its spelling, a role probe that fails), or ~/.vitrify/config.toml exists
         (vitrify probe updates it)
Config:  VITRIFY_PROBE_TIMEOUT_SECS (per probe, default 60); VITRIFY_HOME
Examples:
  vitrify init
  vitrify init --json";

pub const PROBE: &str = "\
Probes the machine again — after installing or upgrading vitrify or a harness CLI, or editing
a role profile — and brings the probe record in ~/.vitrify/config.toml up to date. Asks
before it changes the file (-y answers yes); changes nothing else in it.
Reads:   ~/.vitrify/config.toml (the role profiles and the probe record); git and the harness
         CLIs on PATH (one real model call each, and one per role that adds flags)
Writes:  the [probe] tables of ~/.vitrify/config.toml, when there is a change and you agree;
         restores or refreshes the briefs, schemas and templates under ~/.vitrify/
Prints:  one `status: text` line per check, status ok, FAIL, untested or note; then one
         `change: <key>: <recorded> → <found>` line per difference, and either
         `ok: ~/.vitrify/config.toml: no changes`, `ok: updated ~/.vitrify/config.toml (N
         changes)`, or `note: ~/.vitrify/config.toml: nothing changed`
         stderr: the question, `apply N changes to ~/.vitrify/config.toml? [y/N]`
         --json: {green, checks, harnesses, changes: [{key, from, to}], applied}
Exit status: 0 the record is up to date, nothing failed · 2 a FAIL, the changes not applied
         (declined, or no answer on stdin), or ~/.vitrify/config.toml does not parse
Config:  VITRIFY_PROBE_TIMEOUT_SECS (per probe, default 60); VITRIFY_HOME
Examples:
  vitrify probe
  vitrify probe -y
  vitrify probe --json --assume-yes";

pub const RUN: &str = "\
Builds one SDD until it is Complete or Blocked: claims, dispatches workers, integrates,
publishes. Interrupt it (Ctrl-C, kill -INT <pid>), never its workers; running it again resumes.
Reads:   the SDD on mainline; ~/.vitrify/config.toml (the recorded harness spelling and
         the role profiles); the §5 commands it runs
Writes:  published mainline commits (claims, renewals, releases, each Verified unit);
         .vitrify/run.json, the latest run's snapshot, its report null until it ends;
         .vitrify/logs/<job>/: run.json, gate.json, events.jsonl (one line per event),
           report.txt (the final report), worker-<holder>.out, .err and .prompt.md,
           escalation-<n>.json;
         unit worktrees and the integration checkout under run.worktree_dir.
         --dry-run publishes nothing and moves no file.
Prints:  stdout: the job header (SDD build job — <sdd> @ <version>  [<job>], Done-condition,
         Mainline, Plan, Workers, Roles, Worktrees, Parameters), then, before anything is
         dispatched:
           Watch: vitrify run status --job <job> [--follow] · once it ends, the same command
           prints the report · procedure: vitrify brief run
         then one line per event, then one final report: Complete. or Blocked., the progress
         line, each blocker, the journal tail (replay: exit 0 (consistent)).
         --dry-run: the header and any claim it would recover, ending
           Start: vitrify run <the flags given, without --dry-run>
         stderr: worker reports, gate failures, notes, warnings, and each
           preflight failed: <cause> → <fix>
Exit status: 0 Complete. (--dry-run: preflight passed) · 2 Blocked., a preflight failure,
         an abort verdict, or a usage error · 1 an error, its message last on stderr.
         An interrupted run cleans up and ends by the signal it received.
Config:  mainline (VITRIFY_MAINLINE) · run.worker_cap (VITRIFY_RUN_WORKER_CAP, --cap)
         · run.lease_minutes · run.renew_fraction · run.abort_failed_waves
         · run.worker_timeout_minutes · run.gate_jobs (--gate-jobs) · run.poll_seconds
         · run.worktree_dir · harness.preferred · lint.readset_budget_bytes;
         each key's variable is VITRIFY_<SECTION>_<KEY> (VITRIFY_RUN_GATE_JOBS, …)
Examples:
  vitrify run --dry-run --sdd specs/calc
  vitrify run --sdd specs/calc
  vitrify run --cap 2 --gate-jobs 1 --wait
Procedure: vitrify brief run";

pub const RUN_STATUS: &str = "\
Read-only: writes no file, takes no lock, runs git only with --no-optional-locks; point it at
a live run from another terminal.
Reads:   .vitrify/run.json (the latest job) or the --job's .vitrify/logs/<job>/: run.json,
         gate.json, events.jsonl, report.txt, the worker logs; process liveness; each
         worker worktree's git state and front matter
Writes:  nothing
Prints:  the run's state (alive, died or ended), the progress line, one line per worker
         with its stage, the gate's step; once the run has ended, its final report under
         Report:; warnings as `! <text>` lines.
         Stages: reading · editing · stamped · exited · ? (unknown) while running;
           queued · gate · integrating while integrating; else the outcome itself
         Warnings: an empty log after --quiet-after; no output for --quiet-after; 90% of
           worker_timeout_minutes used; a run that died, with a kill -9 -- -<pid> line for
           each worker process group still running
         --json, in order: job_id, pid, sdd, harness, started_at, at, state, terminated,
           report, progress, params, workers, gate, warnings, logs
         --follow: the view once, then one line per change until the run ends;
           --json one event object per line; --since-start first replays events.jsonl
Exit status: without --follow: 0 a run is shown, whatever its state · 4 no run recorded, or
         --job names none · 2 usage error.
         --follow exits with the followed run: 0 complete · 2 blocked · 1 error, interrupted,
         or died · 4 no run recorded. A reader that goes away (| head) stops it with 0.
Examples:
  vitrify run status
  vitrify run status --follow --job <job>
  vitrify run status --json
Procedure: vitrify brief run";

pub const LINT: &str = "\
Reads:   the SDD; the other SDDs it pins (L9); its changelog (L10); its TDD (L11)
Writes:  nothing
Prints:  `lint clean (L1–L11)`, or one `[L#] subject: cause → remediation` line per finding;
         --all prefixes each line with the slug
         --json: {clean, findings: [{check, subject, cause, remediation}]};
         --all: an array of {sdd, root, clean, findings}
Exit status: 0 clean · 2 a finding, or a usage error · 4 no SDD, or several and no --sdd
Config:  lint.readset_budget_bytes (VITRIFY_LINT_READSET_BUDGET_BYTES), for L5
Examples:
  vitrify lint --sdd specs/calc
  vitrify lint --all --built --json
Procedure: vitrify brief authoring (before Active) · vitrify brief change (after)";

pub const CHECK: &str = "\
Runs every read-only check over one SDD, or with --all the repository: lint (--built once
any unit has build state or a journal exists), audit (check 4 unless --skip-rotten), journal
replay (where a journal exists), ledger regen --check (where a ledger or journal exists) and
the pin states. Runs no §5 command; vitrify run --dry-run does.
Reads:   each SDD, its journal and ledger, the SDDs it pins, its changelog and TDD
Writes:  nothing
Prints:  one `<slug>: [<source> <check>] subject: cause → remediation` line per finding,
         then `<slug>: note: Q-### is open (blocks …)` notes, then
         `check clean: N SDDs`, or `check: N findings in M of K SDDs`
         --json: {sdd, root, clean, findings: [{source, check, subject, cause, remediation}],
           notes}; an array with --all. source: lint, audit, journal, ledger or pin;
           check: \"L4\", \"5\" or null
Exit status: the worst component (an error, 1, wins; else the highest): 0 clean · 2 a lint,
         audit or pin finding, or a usage error · 3 a journal or ledger mismatch · 4 no SDD,
         several and no --sdd, a schema version this vitrify does not read
Config:  lint.readset_budget_bytes (VITRIFY_LINT_READSET_BUDGET_BYTES) · run.gate_jobs
         (VITRIFY_RUN_GATE_JOBS), for audit check 4
Examples:
  vitrify check --sdd specs/calc
  vitrify check --all --skip-rotten
  vitrify check --all --json
Procedure: vitrify brief change";

pub const CHANGE: &str = "\
Subcommands: apply (a change entry's bookkeeping in one step: re-stamp or mark Stale, the
         journal, the ledger)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify change apply --sdd specs/calc
  vitrify change apply --all --dry-run";

pub const CHANGE_APPLY: &str = "\
For each Active SDD whose last change entry has no change event yet: PATCH re-records the
moved units' traces (status and stamps kept) and appends change --entry CH-###; MINOR or
MAJOR appends change --entry CH-### --stale <units> and marks each Stale; then the ledger is
regenerated. Re-running is safe: a recorded entry is skipped.
Reads:   the changelog's last entry, front matter version and updated, the journal, unit
         traces (audit check 5), the pin states
Writes:  the working tree: unit front matter, journal events, the ledger (the commit is
         yours); nothing with --dry-run
Prints:  per SDD, label-prefixed: `calc: CH-004 1.2.0 → 1.3.0 (minor): U-002 marked Stale`
         and `calc: - Staleness applied: U-002 → Stale`, or why nothing applies (already
         recorded, Draft, no change entry). stderr: `next: copy the Staleness applied line
         into CH-### (<changelog>), then run vitrify check`; with --dry-run
         `dry run: nothing written`
         --json: {sdd, root, entry, from, to, bump, action, restamped, stale,
           staleness_line, dry_run}; an array with --all. bump: patch, minor, major or
           null; action: applied, already-recorded, draft or no-entry
Exit status: 0 applied, or nothing to do · 2 refused (version or updated does not match the
         entry, a stale pin, a single file's entry outside ## Changelog), or a usage error
         · 4 no SDD, or several and no --sdd · 1 error
Examples:
  vitrify change apply --sdd specs/calc --dry-run
  vitrify change apply --all
  vitrify change apply --sdd specs/calc --commit 3f2a9c1 --json
Procedure: vitrify brief change";

pub const PIN: &str = "\
Subcommands: list (every external contract, its kind and state) · sync (rewrite stale pins'
         hashes)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify pin list --all
  vitrify pin sync --sdd specs/beta";

pub const PIN_LIST: &str = "\
Reads:   the SDD's external contracts (Producer: external); for each pin, its authority's
         body in the SDD that owns it
Writes:  nothing
Prints:  one line per contract: `beta C-001  ok  pin of alpha C-001  <file>`, a transcription
         reading `transcription`, then `N external contracts: 2 ok · 1 stale · …`
         States: ok · stale · missing-authority · invalid · unchecked (a transcription)
         --json: {clean, contracts: [{sdd, contract, file, kind, state, authority, bytes,
           consumers, detail}], counts}. kind: pin or transcription; authority:
           {sdd, contract, file, bytes}, or null
Exit status: 0 nothing stale, missing-authority or invalid · 2 one is, or a usage error
         · 4 no SDD, or several and no --sdd
Examples:
  vitrify pin list --sdd specs/beta
  vitrify pin list --all --json
Procedure: vitrify brief change";

pub const PIN_SYNC: &str = "\
Reads:   as pin list; the consumers' unit traces
Writes:  the working tree: each stale pin's hash, in its contract file. No status, journal
         event or version: the change entry, vitrify change apply and the commit are yours.
         Nothing with --dry-run
Prints:  `synced beta C-002 ← alpha C-002  <file>` per pin; per consumer SDD
         `beta: - Staleness applied: U-001 → Stale (consume C-002)`; then the count line.
         stderr: `not synced: …` for what it cannot put right, and per consumer
         `next: write beta's change entry around its line, then run vitrify change apply
         --sdd specs/beta`; with --dry-run the same lines, and on stderr
         `dry run: nothing written` in place of the next: lines
         --json: {synced, staleness, unresolved, dry_run}
Exit status: 0 every stale pin synced · 2 something could not be synced (a missing
         authority, an invalid declaration), or a usage error · 4 no SDD, or several and
         no --sdd
Examples:
  vitrify pin sync --sdd specs/beta
  vitrify pin sync --all --dry-run
  vitrify pin sync --all --json
Procedure: vitrify brief change";

pub const QUESTIONS: &str = "\
Reads:   the Decision Log (decisions.md, or ## Decisions & Open Questions) and the units'
         OPEN(Q-###) markers
Writes:  nothing
Prints:  one line per question: `Q-001  open  blocks U-003, U-004  <text>`, then
         `N open, M resolved`; --open lists only the open ones
         --json: {open, resolved, questions: [{sdd, q, status, blocks, noted, text}]}
Exit status: 0 listed, whatever is open (lint L8 is the check that fails) · 4 no SDD, or
         several and no --sdd · 2 usage error
Examples:
  vitrify questions --sdd specs/calc --open
  vitrify questions --all --json
Procedure: vitrify brief change (answer one) · vitrify brief run (a blocker naming Q-###)";

pub const NEW: &str = "\
Reads:   ~/.vitrify/templates/ ($VITRIFY_HOME; an override on disk wins)
Writes:  the working tree, in the current directory: tdd → NAME.md (default tdd.md);
         sdd → NAME/ bundle with sdd.md, units/, contracts/, changelog.md, decisions.md;
         sdd --single → NAME.sdd.md (default NAME scope). An SDD declares
         schema_version: 2.0. Never overwrites.
Prints:  the created path; stderr: `next: fill <path> per vitrify schema show, then run
         vitrify lint --sdd <path>` (sdd), `next: fill <path> per vitrify schema show --tdd`
         (tdd)
Exit status: 0 created · 2 the target exists, or a usage error
Config:  VITRIFY_HOME
Examples:
  vitrify new tdd calc-tdd
  vitrify new sdd calc
  vitrify new sdd --single calc
Procedure: vitrify brief authoring";

pub const SCHEMA: &str = "\
Subcommands: show (print the normative SDD schema, or the TDD schema with --tdd)
Exit status: 2 without a subcommand; see each subcommand's --help
Examples:
  vitrify schema show
  vitrify schema show --tdd";

pub const SCHEMA_SHOW: &str = "\
Reads:   ~/.vitrify/schemas/ ($VITRIFY_HOME; an override on disk wins)
Writes:  nothing in the repository; restores or refreshes its copy under ~/.vitrify/
Prints:  the SDD schema (schema_version 2.0), or with --tdd the TDD schema
Exit status: 0 printed · 1 the file cannot be read · 2 usage error
Config:  VITRIFY_HOME
Examples:
  vitrify schema show
  vitrify schema show --tdd
Procedure: vitrify brief authoring";

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(&str, &str); 31] = [
        ("LOCATE", LOCATE),
        ("READSET", READSET),
        ("CLAIM", CLAIM),
        ("RENEW", RENEW),
        ("RELEASE", RELEASE),
        ("TRACE", TRACE),
        ("TRACE_RECORD", TRACE_RECORD),
        ("TRACE_VERIFY", TRACE_VERIFY),
        ("LEDGER", LEDGER),
        ("LEDGER_REGEN", LEDGER_REGEN),
        ("LEDGER_PROGRESS", LEDGER_PROGRESS),
        ("JOURNAL", JOURNAL),
        ("JOURNAL_APPEND", JOURNAL_APPEND),
        ("JOURNAL_REPLAY", JOURNAL_REPLAY),
        ("AUDIT", AUDIT),
        ("BRIEF", BRIEF),
        ("INIT", INIT),
        ("PROBE", PROBE),
        ("RUN", RUN),
        ("RUN_STATUS", RUN_STATUS),
        ("LINT", LINT),
        ("CHECK", CHECK),
        ("CHANGE", CHANGE),
        ("CHANGE_APPLY", CHANGE_APPLY),
        ("PIN", PIN),
        ("PIN_LIST", PIN_LIST),
        ("PIN_SYNC", PIN_SYNC),
        ("QUESTIONS", QUESTIONS),
        ("NEW", NEW),
        ("SCHEMA", SCHEMA),
        ("SCHEMA_SHOW", SCHEMA_SHOW),
    ];

    #[test]
    fn every_block_has_exit_status_and_examples() {
        for (name, text) in ALL {
            assert!(!text.trim().is_empty(), "{name} is empty");
            assert!(text.contains("Exit status:"), "{name} lacks Exit status:");
            assert!(text.contains("Examples:"), "{name} lacks Examples:");
        }
    }

    #[test]
    fn headings_come_in_order() {
        let order = [
            "Reads:",
            "Writes:",
            "Prints:",
            "Exit status:",
            "Config:",
            "Examples:",
            "Procedure:",
        ];
        for (name, text) in ALL {
            let mut last = 0;
            for heading in order {
                if let Some(at) = text.find(heading) {
                    assert!(at >= last, "{name}: {heading} out of order");
                    last = at;
                }
            }
        }
    }

    #[test]
    fn every_example_is_a_vitrify_line() {
        for (name, text) in ALL {
            let examples = text
                .split("Examples:\n")
                .nth(1)
                .unwrap_or_else(|| panic!("{name}: no Examples: line"));
            let lines: Vec<&str> = examples
                .lines()
                .take_while(|l| l.starts_with("  "))
                .collect();
            assert!(!lines.is_empty(), "{name}: no example");
            for l in lines {
                assert!(l.starts_with("  vitrify "), "{name}: {l}");
            }
        }
    }
}
