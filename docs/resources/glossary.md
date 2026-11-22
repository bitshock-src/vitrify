# Glossary

> Definitions for vitrify's spec-driven development terms: documents, units, contracts, build state, gates, and runs.

Use this page when a term in another page is unfamiliar. These definitions are the shared vocabulary of the documentation; each entry points to the page that explains the idea in full. The normative definitions of the documents themselves are the embedded schemas, printed by `vitrify schema show` and `vitrify schema show --tdd`.

## A

**Active**
: The SDD status in which it is built and maintained. An SDD goes Active only after `vitrify lint` exits 0, and only against the current revision of an Approved TDD. See [Specs: TDD and SDD](../about/concepts/specs.md).

**Adoption**
: The reuse of an attempt that was Built but never landed: a run set it aside on its branch, so its worker is known to be gone; the branch carries a Built stamp; and its recorded trace still matches the unit's reading set. An attempt whose lease merely lapsed is not adopted, because its worker may still be writing, and neither is one the gate or its worker already judged (`rejected-gate`, `rejected-write-set`, `ownership-conflict`, `blocked`, `released`) or one that wrote outside the unit's write-set; those go to a worker. A new claim, with holder `<job>-a<n>`, takes a new fencing token and sends the work straight to the gate with no worker session; adoption skips the model, never the gate. See [Build State](../about/concepts/build-state.md#kept-progress).

**Anchor**
: The commit of a unit's latest `verified` journal event: its integration, or its latest re-check. It marks where the unit's checks last passed, and the re-check cascade asks whether a path the unit is checked against changed since it. See [Build State](../about/concepts/build-state.md#the-anchor).

**Audit**
: The seven mechanical drift checks run by `vitrify audit`: orphans, ghosts, contract references, rotten Verified, trace mismatch, projection consistency, and lapsed leases. See [Gates](../about/concepts/gates.md).

**Authored baseline**
: A unit that ships Verified before the build starts — typically tests written at authoring time — stamped with vitrify's own state verbs, never by hand. See [Test Posture](../about/concepts/test-posture.md).

**Authority**
: The contract a pin stands for: a contract owned by another SDD in the same repository. See [Contracts](../about/concepts/contracts.md).

## B

**Blocked**
: One of the two ways a run ends: work remains, and every unfinished unit's blocker is named — an open question, a failing gate command, an attempt limit, a rotten unit, or a dependency chain. In a project run the run is Blocked when any lane is. Exit code 2. Also a lane state: see **Lane state**. See [Runs](../about/concepts/runs.md).

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

**Closure**
: A unit's dependency closure: its `depends_on` units and pins' producer units, and everything those depend on in turn, in any SDD. The resolver computes each closure once; a unit is re-checked when a path owned by anything in its closure, or by the unit itself, changed. See [Build State](../about/concepts/build-state.md#what-a-unit-is-checked-against).

**Commands (§5 Commands)**
: The four commands an SDD's Technology Decisions name — Bootstrap, Build, Test, and Lint — run from the repository root by every gate. Test may be `none`.

**Complete**
: The run outcome in which every non-Retired unit is Verified at the current SDD version, the audit is clean, the journal replays consistently, and the §5 Test command, if the SDD has one, passes. In a project run it holds when no lane is blocked: each SDD is Complete, or Up to date when nothing changed since its last recorded state. Exit code 0.

**Consumer**
: A unit that lists a contract in its `consumes` front matter. A contract's `Consumers:` line must name exactly its consumers (lint L4).

**Config file (`config.toml`)**
: `~/.vitrify/config.toml`: every setting, the harness role profiles, and the probe record. `vitrify init` writes it with every default; a repository's `.vitrify/config.toml` may override any of it but the probe record, for that repository. See [Configuration](../cli/configuration.md).

**Content**
: The briefs, templates, and schemas embedded in the binary and materialized to `~/.vitrify/` by `vitrify init`. A file on disk overrides the embedded copy. See [Configuration](../cli/configuration.md).

**Contract**
: A `C-###` entry that freezes one cross-unit surface. Units reach each other's code only through contracts. See [Contracts](../about/concepts/contracts.md).

**Critical path**
: The longest chain of unfinished units in the unit graph, which no amount of capacity shortens. `vitrify run --dry-run` prints it. See [Design Rationale](../about/design-rationale.md#scheduling-and-capacity).

**Culprit**
: The commit that broke a rotten unit's row, named by flake-aware culprit finding over the mainline commits since the unit's anchor, together with the upstream unit it integrated. The search can also end in a **flake verdict** (the row is flaky) or be inconclusive. See [Gates](../about/concepts/gates.md#rotten-units-and-culprits).

## D

**Decision (`D-###`)**
: A local choice recorded in the SDD's Decision Log, cited instead of being restated.

**Decision-complete**
: The property of an SDD whose every technical choice is already made, so that building needs only bounded reasoning. A sentence that leaves the implementer a choice is a defect.

**Decision Log**
: The SDD region that holds decisions (`D-###`) and questions (`Q-###`): `decisions.md` in a bundle, the `## Decisions & Open Questions` section in a single file.

**Dirty unit**
: A Verified unit that a run would re-check, because a commit since its anchor changed a path it is checked against. `vitrify check` lists dirty units with the changed paths that made each one dirty. See [Build State](../about/concepts/build-state.md#what-a-unit-is-checked-against).

**Draft**
: The SDD status while it is being authored. Also the initial status of a unit that is not yet ready to build.

## E

**Early cutoff**
: The rule that stops the re-check cascade where a change makes no difference: a unit regenerated into byte-identical files changes no path, so nothing downstream is re-checked. See [Build State](../about/concepts/build-state.md#early-cutoff-and-fingerprints).

**EARS**
: Easy Approach to Requirements Syntax: the requirement sentence templates (ubiquitous, event, state, option, and unwanted behavior) that units use for their requirements.

**Event log (`events.jsonl`)**
: `.vitrify/logs/<job>/events.jsonl`: one JSON object per line for each thing a run does — dispatches, adoptions, gate steps, re-checks, rotten units and culprits, outcomes, its end — appended as it happens and never rewritten. The line of an event that belongs to one lane names it in a `lane` field. `vitrify run status --follow --since-start` replays it. Evidence, not authoritative. See [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status).

**Escalation**
: The one inference call the orchestrator makes when a unit's work keeps failing or its merge conflicts. The reply is a JSON verdict — `retry`, `release`, `block`, or `abort` — that the orchestrator, not the model, enforces.

**External contract**
: A contract whose `Producer:` line begins `external`: one SDD's stand-in for a surface it does not own. It is either a pin of another SDD's contract or a transcription of a source no SDD owns. See [Contracts](../about/concepts/contracts.md).

## F

**Flake verdict**
: The conclusion of culprit finding that no commit since a unit's last check broke its row, because the row itself is flaky. The report names the row, and the unit is re-checked again. See [Gates](../about/concepts/gates.md#rotten-units-and-culprits).

**Fencing check**
: The first integration-gate step: the token `c<seq>` in the work's branch name must equal the unit's current `claim_seq`. Work with a superseded token is rejected even if it is green.

**Fencing token**
: See **Claim sequence**.

## G

**Gate**
: A fixed sequence of commands and checks that work must pass to advance, with no judgment involved. See [Gates](../about/concepts/gates.md).

**Gate pool**
: The one pool of `run.gate_jobs` tokens that every §5 command and Verification row of every gate, re-check, head check, preflight, and culprit probe in a run takes from, served by priority and then by arrival; `run.gate_jobs = 0`, the default, is one token per CPU core. Each command holds one token; how it parallelizes inside itself is its project's business. See [Gates](../about/concepts/gates.md) and [Support Matrix](../reference/support-matrix.md#platforms).

**Ghost**
: A Built or Verified unit whose `owns` globs match no file (audit check 2).

**Ground Rules**
: The SDD region (§4) of rules every unit reads. Together with Technology Decisions it is the `global` input of every trace.

## H

**Harness CLI (harness)**
: A coding-agent CLI that vitrify drives headlessly: Claude Code (`claude`), Codex (`codex`), or OpenCode (`opencode`). See [Harness CLIs](../cli/harnesses.md).

**Holder**
: The identity that holds a claim. Under `vitrify run`, holders are named `<job>-w<i>` for a worker and `<job>-a<n>` for an adopted attempt.

## I

**Init**
: `vitrify init`, which sets a machine up once: it creates `~/.vitrify/`, writes `config.toml` with every setting at its default, materializes content, and records a first probe. No other command creates `~/.vitrify/`, and every other command refuses until it exists. See [Installation](../getting-started/installation.md).

**Input check**
: The integration-gate step after the merge: the trace recorded when the unit was stamped Built must equal the hashes of its inputs at the mainline head. A unit built from text that has since changed is rebuilt.

**Integration checkout**
: A lane's dedicated checkout, `<worktrees>/mainline-<slug>`, in which its integration gate runs, one batch at a time. It is recreated at the start of each run.

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

**Lane**
: One SDD's build state inside a project run, with its own integration checkout and lock, integrator, and lease keeper. All lanes share one worker pool, one gate pool, and one scheduler. `run.lane_cap` bounds how many hold their checkouts at once. See [Design Rationale](../about/design-rationale.md#lanes-and-pristine-checkouts).

**Lane state**
: Where a lane stands: `idle` (units remain, but none is claimable, in flight, or due a re-check; each waits on units elsewhere), `build` (a unit is claimable, adoptable, or in flight), `recheck` (dirty Verified units whose dependencies have no work pending), `up to date` (nothing to regenerate, re-check, or build), `complete` (its done-condition holds at the end of the run), or `blocked` (unfinished, set-aside, or rotten units remain at the end, or the lane's preflight failed). `project.json` and `vitrify run status` show them. See [Design Rationale](../about/design-rationale.md#lanes-and-pristine-checkouts).

**Lease**
: The time-bounded hold a claim gives its holder — 60 minutes by default — renewed while the work continues. Expiry is advisory; the fencing token is the correctness mechanism.

**Ledger**
: The derived table of unit statuses — `ledger.md` in a bundle, `## State Ledger` in a single file — regenerated from unit front matter and never edited by hand.

**Lint**
: The twelve mechanical authoring checks, L1–L12, run by `vitrify lint`. An SDD must pass them before it goes Active; every run preflights with all of them except L7.

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
: The checks `vitrify run` and kickoff perform before anything starts. A failure of the run as a whole refuses the start (exit code 2, or 4 for an SDD at schema 2.0 whose units consume pins); a failure of one lane's own checks refuses a run of one SDD, and in a project run blocks only that lane, as `[<slug>] preflight failed: <cause> → <fix>`, while the other lanes build. `vitrify run --dry-run` performs all of it.

**Primary checkout**
: Your own checkout of the repository. vitrify never builds or gates in it; when it has mainline checked out, a run fast-forwards it commit by commit and never resets it.

**Pristine checkout**
: A checkout reset to the commit being checked with every untracked and ignored file removed. Every gate, re-check, preflight, and culprit probe starts from one, and lane checkouts are recreated at the start of each run, so a result never depends on leftovers. See [Gates](../about/concepts/gates.md#every-gate-starts-from-a-pristine-checkout).

**Priority**
: The order in which claimable units are dispatched in a project run: a unit's Hu's label, the number of unfinished units on the longest chain from it to the end of the graph, itself included, so the unit with the most work behind it goes first. Ties break by SDD slug, then unit ID. `run.priority = "history"` weighs each unit by what its past worker sessions took instead of 1. See [Design Rationale](../about/design-rationale.md#scheduling-and-capacity) and [Configuration](../cli/configuration.md).

**Probe**
: What `vitrify init` and `vitrify probe` do: run git and each harness CLI's one streaming headless spelling, and each role that adds flags, and record what works in the `[probe]` tables of `~/.vitrify/config.toml`. `vitrify probe` asks before it changes the record. See [Harness CLIs](../cli/harnesses.md).

**Producer**
: The unit that implements a contract's surface, named on the contract's `Producer:` line.

**Project run**
: A plain `vitrify run` in a repository with several SDDs, or `vitrify run --sdd <root> --upstream` that selects more than one: one deterministic, concurrent, incremental build over one unit graph, with a lane per Active SDD (a lane for each SDD selected, with `--upstream`). Only a project run prints the `Project build job` header, `[<slug>]` event lines, and the per-SDD final report. `vitrify run --sdd <root>`, or a repository of one SDD, is a run of one lane. See [Design Rationale](../about/design-rationale.md#project-runs) and [Run a Build](../user-guides/run-a-build.md).

## Q

**Question (`Q-###`)**
: A gap recorded in the Decision Log instead of being guessed at. An `OPEN(Q-###)` marker keeps a unit from being Ready. A question is open unless its entry says `resolved`.

## R

**Reading set**
: The ordered, minimal spec text a worker reads to build a unit: Ground Rules, Technology Decisions, the unit's own entry, and its consumed contracts — and, for a pin, its authority's body. Budgeted at 65,536 bytes per unit (lint L5). See [Work Units](../about/concepts/units.md).

**Ready**
: The unit status in which it can be claimed once its dependencies are Built or Verified.

**Re-check**
: Running a Verified unit's Verification rows again, with no model call, because a path it is checked against changed since its anchor. A passing re-check appends a `verified` event with actor `recheck`; `vitrify run --recheck` distrusts every recorded check and re-checks everything. Compare **Regenerate**: a model call on a spec change. See [Build State](../about/concepts/build-state.md#the-two-cascades).

**Retired**
: The unit status for removed scope. Retired units are excluded from progress totals.

**Rotten unit**
: A Verified unit whose rows fail after a change upstream, and fail on the base commit too, so the change that was being checked did not break them. It is reported with its culprit, never regenerated, and the units that depend on it wait, in any SDD. Not the same as **Rotten Verified**, audit check 4. See [Gates](../about/concepts/gates.md#rotten-units-and-culprits).

**Role**
: The profile a harness is invoked with: `worker` for a first attempt, `retry` for later attempts, and `escalation` for the judge. Set in the `[harness.<name>.<role>]` tables of `~/.vitrify/config.toml`.

**Rotten Verified**
: A Verified unit whose Verification command now exits non-zero (audit check 4).

**Run record**
: `.vitrify/run.json`, a snapshot of the latest run for people and scripts (a copy is kept in `.vitrify/logs/<job>/`, beside `project.json`, which records each lane's state and the critical path as it ran); once the run ends it carries the final report as `report`. vitrify never decides anything from it. See [Monitor a Run](../user-guides/monitor-a-run.md).

## S

**`shared_inputs`**
: An optional list in an SDD's core front matter of the repository-relative paths or globs outside every SDD's `scope_roots` that its commands read. Without it every outside path is an input of every unit; with it, only the paths it matches are. See [Data Model and Formats Specification](../reference/formats.md#33-sdd-front-matter-shared_inputs) and [Build State](../about/concepts/build-state.md#what-a-unit-is-checked-against).

**Schema version**
: The SDD schema version an instance declares in `schema_version`, in its core front matter. It is required, and this vitrify reads `2.0` and `2.1` (2.1 lets `depends_on` name a pin, which a project run reads as a wait for the pin's producing units); a missing key or any other value is refused with exit 4, naming both. See [Versioning](../reference/formats.md#10-versioning).

**Scope roots (`scope_roots`)**
: The directories an SDD governs. Every file under them that git would commit must be owned by exactly one unit or listed as `unmanaged`.

**SDD**
: The buildable specification of a system or feature — the *what and exactly how*: pinned technology, file layout, frozen contracts, work units with executable checks, and the state of the build. Also short for spec-driven development, the method vitrify implements.

**Single file**
: The SDD shape held in one file, `specs/<slug>.sdd.md`, with units and contracts as `###` regions. Suited to about eight units or fewer; under `vitrify run` its lane runs one worker at a time. Compare **Bundle**.

**Stale**
: The unit status after one of its inputs changed under a Built or Verified unit. A Stale unit is claimable and gets rebuilt.

**Stage**
: Where an attempt in progress stands, as `vitrify run status` shows it: `reading`, `editing`, `stamped` or `exited` while the worker runs; `queued`, `gate` or `integrating` once it has reported built. Derived from the worker's worktree and the gate's progress, never recorded. See [Monitor a Run](../user-guides/monitor-a-run.md#check-a-runs-status).

**Stamp**
: The `built:` or `verified:` front-matter field — `YYYY-MM-DD @ SDD <version>, commit <short-sha>` — written with the status by `vitrify trace record --unit U-### --stamp built` (or `--stamp verified`).

**State verbs**
: The commands that write build state: `claim`, `renew`, `release`, `trace record`, `journal append`, `ledger regen`, `change apply`, and the fixing flags of `audit`. Stamps are never written by hand.

**Suite anchor**
: The commit of an SDD's latest `audit` journal event carrying `suite: "pass"`: where its §5 Test command last passed. The suite is judged against it as a unit is against its anchor, so a change that reaches none of the SDD's units leaves the recorded pass standing. A gate or re-check that passes the Test command records `suite: "pass"`. See [Build State](../about/concepts/build-state.md#the-anchor).

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

**Unit graph**
: The one graph a project run schedules: its nodes are the units of every SDD, and its edges are local `depends_on` entries plus the edges through a pin to the units its authority names as producers. It must be acyclic; the derived SDD graph, which may hold mutual pairs, is for display. `vitrify locate --graph` prints it. See [Design Rationale](../about/design-rationale.md#the-resolver).

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
