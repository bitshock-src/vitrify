# Glossary

> Definitions for vitrify's spec-driven development terms: documents, units, contracts, build state, gates, and runs.

Use this page when a term in another page is unfamiliar. These definitions are the shared vocabulary of the documentation; each entry points to the page that explains the idea in full. The normative definitions of the documents themselves are the embedded schemas, printed by `vitrify schema show` and `vitrify schema show --tdd`.

## A

**Active**
: The SDD status in which it is built and maintained. An SDD goes Active only after `vitrify lint` exits 0, and only against the current revision of an Approved TDD. See [Specs: TDD and SDD](../about/concepts/specs.md).

**Audit**
: The seven mechanical drift checks run by `vitrify audit`: orphans, ghosts, contract references, rotten Verified, trace mismatch, projection consistency, and lapsed leases. See [Gates](../about/concepts/gates.md).

**Authored baseline**
: A unit that ships Verified before the build starts — typically tests written at authoring time — stamped with vitrify's own state verbs, never by hand. See [Test Posture](../about/concepts/test-posture.md).

**Authority**
: The contract a pin stands for: a contract owned by another SDD in the same repository. See [Contracts](../about/concepts/contracts.md).

## B

**Blocked**
: One of the two ways a run ends: work remains, and every unfinished unit's blocker is named — an open question, a failing gate command, an attempt limit, or a dependency chain. Exit code 2. See [Runs](../about/concepts/runs.md).

**Brief**
: An operative procedure for one task, printed on demand by `vitrify brief <task>` in the installed binary's version. There are seven: `onboard`, `authoring`, `change`, `run`, `kickoff`, `build-unit`, and `integrate`. `vitrify brief` with no task prints the index of them, each with when to use it. Briefs are embedded in the binary; a coding agent learns vitrify from `vitrify --help` and the briefs.

**Built**
: The unit status after its worker passes the unit gate and stamps the unit with `vitrify trace record --unit U-### --stamp built`. Built work waits for the integration gate.

**Bundle**
: The SDD shape in which each region is its own file under `specs/<slug>/`, with one file per unit and per contract, so parallel workers never write the same file. Compare **Single file**.

## C

**Change apply**
: `vitrify change apply`: records an SDD's last change entry in one step. For a PATCH entry it re-records the traces the edit moved; for MINOR or MAJOR it marks those units Stale. It appends the entry's journal events, regenerates the ledger, and prints the entry's `Staleness applied` line. See [Change a Spec](../user-guides/change-a-spec.md).

**Change entry**
: A `CH-###` heading in the SDD's changelog that records a version bump, what was added, modified, or removed, the staleness marks applied, and the resulting codebase delta. An Active SDD changes only through change entries. See [Change a Spec](../user-guides/change-a-spec.md).

**Check**
: `vitrify check`: the read-only gate that says whether an SDD, or with `--all` the whole repository, is consistent. It runs lint, the audit, the journal replay, the ledger check, and the pin states, and names each finding's fix. See [Command Reference](../cli/command-reference.md#vitrify-check).

**Claim**
: An atomic mainline commit that takes a lease on a unit: it sets `status: Claimed`, the holder, and the lease expiry, and increments `claim_seq`. See [Build State](../about/concepts/build-state.md).

**Claim sequence (`claim_seq`)**
: The unit's fencing token: a counter that every claim increments and nothing resets. It is carried in the work branch name `sdd/<sdd>/<unit>/c<seq>` and checked at the integration gate.

**Claimable**
: A derived predicate, never stored: every `depends_on` unit is Built or Verified, and the unit is Ready, Stale, or Claimed with a lapsed lease. `vitrify locate --units` shows it.

**Commands (§5 Commands)**
: The four commands an SDD's Technology Decisions name — Bootstrap, Build, Test, and Lint — run from the repository root by every gate. Test may be `none`.

**Complete**
: The run outcome in which every non-Retired unit is Verified at the current SDD version, the audit is clean, the journal replays consistently, and the §5 Test command, if the SDD has one, passes. Exit code 0.

**Consumer**
: A unit that lists a contract in its `consumes` front matter. A contract's `Consumers:` line must name exactly its consumers (lint L4).

**Config file (`config.toml`)**
: `~/.vitrify/config.toml`: every setting, the harness role profiles, and the probe record. `vitrify init` writes it with every default; a repository's `.vitrify/config.toml` may override any of it but the probe record, for that repository. See [Configuration](../cli/configuration.md).

**Content**
: The briefs, templates, and schemas embedded in the binary and materialized to `~/.vitrify/` by `vitrify init`. A file on disk overrides the embedded copy. See [Configuration](../cli/configuration.md).

**Contract**
: A `C-###` entry that freezes one cross-unit surface. Units reach each other's code only through contracts. See [Contracts](../about/concepts/contracts.md).

## D

**Decision (`D-###`)**
: A local choice recorded in the SDD's Decision Log, cited instead of being restated.

**Decision-complete**
: The property of an SDD whose every technical choice is already made, so that building needs only bounded reasoning. A sentence that leaves the implementer a choice is a defect.

**Decision Log**
: The SDD region that holds decisions (`D-###`) and questions (`Q-###`): `decisions.md` in a bundle, the `## Decisions & Open Questions` section in a single file.

**Draft**
: The SDD status while it is being authored. Also the initial status of a unit that is not yet ready to build.

## E

**EARS**
: Easy Approach to Requirements Syntax: the requirement sentence templates (ubiquitous, event, state, option, and unwanted behavior) that units use for their requirements.

**Event log (`events.jsonl`)**
: `.vitrify/logs/<job>/events.jsonl`: one JSON object per line for each thing a run does — dispatches, gate steps, outcomes, its end — appended as it happens and never rewritten. `vitrify run status --follow --since-start` replays it. Evidence, not authoritative. See [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status).

**Escalation**
: The one inference call the orchestrator makes when a unit's work keeps failing or its merge conflicts. The reply is a JSON verdict — `retry`, `release`, `block`, or `abort` — that the orchestrator, not the model, enforces.

**External contract**
: A contract whose `Producer:` line begins `external`: one SDD's stand-in for a surface it does not own. It is either a pin of another SDD's contract or a transcription of a source no SDD owns. See [Contracts](../about/concepts/contracts.md).

## F

**Fencing check**
: The first integration-gate step: the token `c<seq>` in the work's branch name must equal the unit's current `claim_seq`. Work with a superseded token is rejected even if it is green.

**Fencing token**
: See **Claim sequence**.

## G

**Gate**
: A fixed sequence of commands and checks that work must pass to advance, with no judgment involved. See [Gates](../about/concepts/gates.md).

**Ghost**
: A Built or Verified unit whose `owns` globs match no file (audit check 2).

**Ground Rules**
: The SDD region (§4) of rules every unit reads. Together with Technology Decisions it is the `global` input of every trace.

## H

**Harness CLI (harness)**
: A coding-agent CLI that vitrify drives headlessly: Claude Code (`claude`), Codex (`codex`), or OpenCode (`opencode`). See [Harness CLIs](../cli/harnesses.md).

**Holder**
: The identity that holds a claim. Under `vitrify run`, holders are named `<job>-w<i>`.

## I

**Init**
: `vitrify init`, which sets a machine up once: it creates `~/.vitrify/`, writes `config.toml` with every setting at its default, materializes content, and records a first probe. No other command creates `~/.vitrify/`, and every other command refuses until it exists. See [Installation](../getting-started/installation.md).

**Input check**
: The integration-gate step after the merge: the trace recorded when the unit was stamped Built must equal the hashes of its inputs at the mainline head. A unit built from text that has since changed is rebuilt.

**Integration checkout**
: The dedicated checkout, `<worktrees>/mainline`, in which the integration gate runs, one batch at a time.

**Integration gate**
: The serial gate that merges Built work and stamps it Verified: fencing check, write-set check, merge, input check, then bootstrap, build, Verification tables, lint, the Test regression check, and audit checks 1–3. See [Gates](../about/concepts/gates.md).

## J

**Job**
: One `vitrify run`, identified as `run-<UTC timestamp>-p<pid>`. The job ID names the run's holders, logs, and record.

**Journal**
: The SDD's append-only event log — `journal.ndjson` in a bundle, the fenced block under `## Journal` in a single file. Replaying it reproduces unit state. See [Build State](../about/concepts/build-state.md).

## K

**Kickoff**
: The interactive build procedure (`vitrify brief kickoff`) that a harness session follows. It is the state machine that `vitrify run` executes deterministically.

## L

**Lease**
: The time-bounded hold a claim gives its holder — 60 minutes by default — renewed while the work continues. Expiry is advisory; the fencing token is the correctness mechanism.

**Ledger**
: The derived table of unit statuses — `ledger.md` in a bundle, `## State Ledger` in a single file — regenerated from unit front matter and never edited by hand.

**Lint**
: The eleven mechanical authoring checks, L1–L11, run by `vitrify lint`. An SDD must pass them before it goes Active; every run preflights with all of them except L7.

## M

**Mainline**
: The branch vitrify publishes state to. Unit front matter on mainline is the state of the build. See [Command Reference](../cli/command-reference.md#vitrify-claim-renew-release) for how it is resolved.

## O

**Oracle**
: The behavioral oracle committed in this repository: the calc example (`fixtures/example/`), the golden corpus (`fixtures/golden/`), and the seeded fixtures (`fixtures/seeded/`). Where prose and the oracle disagree, the oracle is normative.

**Orphan**
: A file under `scope_roots` that git would commit and that no unit owns and no `unmanaged:` entry lists (audit check 1, lint L2).

## P

**Parallel group**
: A named set of mutually independent units — `[P]` in Execution Order, `parallel_group` in front matter. Groups are the author's plan, held to independence by lint L3; the scheduler uses `depends_on` alone.

**Pin (pinned reference)**
: An external contract that stands for a contract another SDD owns. It keeps its own header, carries `Pin: <sdd-slug> C-### @ <hash12>` — the hash of its authority's body — and has no body. Lint L9 checks it; `vitrify pin list` shows its state and `vitrify pin sync` rewrites a stale hash. See [Contracts](../about/concepts/contracts.md#external-contracts).

**PRD**
: Product Requirements Document: what to build, for whom, and how to tell it works, in your words — a file, or a few lines in a prompt to your agent. Authoring starts from one: the agent reads it against the TDD schema and interviews you for what it leaves open. vitrify reads no PRD itself; the TDD it becomes is the design authority. See [Author a Spec](../user-guides/author-a-spec.md#1-start-from-a-prd).

**Preflight**
: The checks `vitrify run` and kickoff perform before anything starts. Any failure refuses the start with exit code 2.

**Primary checkout**
: Your own checkout of the repository. vitrify never builds or gates in it; when it has mainline checked out, a run fast-forwards it commit by commit and never resets it.

**Probe**
: What `vitrify init` and `vitrify probe` do: run git and each harness CLI's one streaming headless spelling, and each role that adds flags, and record what works in the `[probe]` tables of `~/.vitrify/config.toml`. `vitrify probe` asks before it changes the record. See [Harness CLIs](../cli/harnesses.md).

**Producer**
: The unit that implements a contract's surface, named on the contract's `Producer:` line.

## Q

**Question (`Q-###`)**
: A gap recorded in the Decision Log instead of being guessed at. An `OPEN(Q-###)` marker keeps a unit from being Ready. A question is open unless its entry says `resolved`.

## R

**Reading set**
: The ordered, minimal spec text a worker reads to build a unit: Ground Rules, Technology Decisions, the unit's own entry, and its consumed contracts — and, for a pin, its authority's body. Budgeted at 65,536 bytes per unit (lint L5). See [Work Units](../about/concepts/units.md).

**Ready**
: The unit status in which it can be claimed once its dependencies are Built or Verified.

**Retired**
: The unit status for removed scope. Retired units are excluded from progress totals.

**Role**
: The profile a harness is invoked with: `worker` for a first attempt, `retry` for later attempts, and `escalation` for the judge. Set in the `[harness.<name>.<role>]` tables of `~/.vitrify/config.toml`.

**Rotten Verified**
: A Verified unit whose Verification command now exits non-zero (audit check 4).

**Run record**
: `.vitrify/run.json`, a snapshot of the latest run for people and scripts; once the run ends it carries the final report as `report`. vitrify never decides anything from it. See [Monitor a Run](../user-guides/monitor-a-run.md).

## S

**Schema version**
: The SDD schema version an instance declares in `schema_version`, in its core front matter. It is required, and this vitrify reads exactly `2.0`; a missing key or any other value is refused with exit 4, naming `2.0`. See [Versioning](../reference/formats.md#10-versioning).

**Scope roots (`scope_roots`)**
: The directories an SDD governs. Every file under them that git would commit must be owned by exactly one unit or listed as `unmanaged`.

**SDD**
: The buildable specification of a system or feature — the *what and exactly how*: pinned technology, file layout, frozen contracts, work units with executable checks, and the state of the build. Also short for spec-driven development, the method vitrify implements.

**Single file**
: The SDD shape held in one file, `specs/<slug>.sdd.md`, with units and contracts as `###` regions. Suited to about eight units or fewer; under `vitrify run` it forces a worker cap of 1. Compare **Bundle**.

**Stale**
: The unit status after one of its inputs changed under a Built or Verified unit. A Stale unit is claimable and gets rebuilt.

**Stage**
: Where an attempt in progress stands, as `vitrify run status` shows it: `reading`, `editing`, `stamped` or `exited` while the worker runs; `queued`, `gate` or `integrating` once it has reported built. Derived from the worker's worktree and the gate's progress, never recorded. See [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status).

**Stamp**
: The `built:` or `verified:` front-matter field — `YYYY-MM-DD @ SDD <version>, commit <short-sha>` — written with the status by `vitrify trace record --unit U-### --stamp built` (or `--stamp verified`).

**State verbs**
: The commands that write build state: `claim`, `renew`, `release`, `trace record`, `journal append`, `ledger regen`, `change apply`, and the fixing flags of `audit`. Stamps are never written by hand.

## T

**TDD**
: Technical Design Document: the design authority that records the *why* — problem, goals, architecture, rationale, and rejected alternatives. An SDD pins one Approved TDD revision. See [Specs: TDD and SDD](../about/concepts/specs.md).

**Technology Decisions**
: The SDD region (§5) that pins the language, runtime, and every dependency version, and lists the §5 Commands.

**Test posture**
: How a unit is verified — whether tests exist before the build, are written by the unit or by a test unit, or are not used — chosen per unit and expressed only through ownership and Verification tables. See [Test Posture](../about/concepts/test-posture.md).

**Trace (input trace)**
: A unit's `trace:` map of hashes — the first 12 hex characters of a SHA-256 per input: `unit`, each consumed `C-###`, and `global` — recorded when the unit is stamped. A mismatch makes the unit Stale.

**Transcription**
: An external contract whose `Producer:` line names no contract: the consumer's own transcription of a source no SDD owns, such as a vendor API. Nothing can check it, so `vitrify pin list` lists it as `unchecked`, never as `ok`. See [Contracts](../about/concepts/contracts.md#external-contracts).

## U

**Unit (work unit)**
: A `U-###` entry: the smallest buildable piece of an SDD, with exclusive ownership, EARS requirements, a decision-complete Implementation block, and a Verification table. One unit fits one worker session. See [Work Units](../about/concepts/units.md).

**Unit gate**
: The worker-side gate in the unit's worktree: bootstrap, build, the unit's Verification table, and lint, all exiting 0 before the unit is stamped Built.

**Unmanaged**
: Paths under `scope_roots` that no unit owns, listed under `unmanaged:` in System Layout & Ownership so that they are not orphans.

## V

**Verification table**
: The table in each unit that maps every requirement to an exact command. Gates run it, and a Verified unit's table keeps running at every later gate and audit.

**Verified**
: The unit status after its work passes the integration gate and lands on mainline. Verified is meaningful only at a version.

## W

**Worker**
: A headless harness session that builds one claimed unit in its own worktree and ends with a one-line JSON report.

**Worktree**
: A git worktree vitrify creates for one attempt at one unit, `<worktrees>/<sdd>-<unit>-c<seq>`, outside the repository's working tree — by default under `~/.vitrify/worktrees/<repo>-<hash>/`.

**Write-set**
: The paths a unit may change: whatever its `owns` globs match, plus the `status`, `built`, and `trace` fields of its own entry. The write-set check rejects any other change before merging.

## Related Documentation

- [vitrify Primer](../about/primer.md) — the five-minute model these terms belong to.
- [Specs: TDD and SDD](../about/concepts/specs.md), [Work Units](../about/concepts/units.md), [Contracts](../about/concepts/contracts.md), [Test Posture](../about/concepts/test-posture.md), [Gates](../about/concepts/gates.md), [Build State](../about/concepts/build-state.md), and [Runs](../about/concepts/runs.md) — the concept pages.
- [Data Model and Formats Specification](../reference/formats.md) — the exact formats behind the terms.
