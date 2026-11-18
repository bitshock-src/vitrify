# SDD Schema

This document defines the schema for an **SDD** — the buildable specification of a system or feature under spec-driven development. An SDD is decision-complete: every technical choice is already made, every requirement is testable, and implementation state is tracked inside the artifact itself. It is the second half of a two-document pair:

* The **TDD** (Technical Design Document, schema: [`./tdd-schema.md`](./tdd-schema.md)) records the *why*: problem, goals, architecture, rationale, rejected alternatives.
* The **SDD** records the *what and exactly how*: pinned technology, file-level layout, frozen interface contracts, work units with executable acceptance checks, and a live ledger of what has been built.

The TDD drives the SDD. An SDD is derived from an approved TDD and cites it for every consequential decision; the SDD never restates the TDD's rationale and never changes the TDD (§3).

This schema optimizes for five properties:

1. **Living** — spec and codebase are updated continuously; change is routine, not exceptional (§12).
2. **Concurrent** — multiple implementers can build simultaneously without colliding, via exclusive path ownership, frozen contracts, and a dependency DAG (§6, §7, §9, §13).
3. **Auditable** — one read of the ledger answers "how much of this spec is built"; one read of a change entry answers "what must now change in the code"; the journal preserves how the state got there (§11, §12).
4. **Agnostic** — an SDD is plain text in version control, self-contained, executable by any competent implementer, human or automated, inside or outside any framework (§4).
5. **Decision-complete** — the SDD absorbs all open-ended thinking at authoring time so that building requires only modest, bounded reasoning (§4, §14).

**How to read this schema.** The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative, in their conventional uppercase-only meaning. "SDD instance" (or just "SDD") means a document written to this schema. Each numbered section below defines one region of an SDD.

---

## 1. Shape & Layout

An SDD instance is either a **single markdown file** or a **file bundle** (a directory). Both shapes carry the same regions, in the same order, under the same headings; a bundle maps each region to a file so that concurrent writers rarely touch the same file.

| Region | Single-file heading | Bundle file | Mutable while building? |
| --- | --- | --- | --- |
| Metadata (§2) | front matter | `sdd.md` front matter | No — author only |
| Overview & TDD reference (§3) | `## Overview` | `sdd.md` | No |
| Ground rules (§4) | `## Ground Rules` | `sdd.md` | No |
| Technology decisions (§5) | `## Technology Decisions` | `sdd.md` | No |
| System layout & ownership (§6) | `## System Layout & Ownership` | `sdd.md` | No |
| Interface contracts (§7) | `## Interface Contracts` | `contracts/C-###-<slug>.md` | No |
| Work units (§8) | `## Work Units` | `units/U-###-<slug>.md` | Status fields only |
| Execution order (§9) | `## Execution Order` | `sdd.md` | No |
| Verification gates (§10) | `## Verification Gates` | `sdd.md` | No |
| State ledger (§11) | `## State Ledger` | `ledger.md` (derived) | Yes — status only |
| Build journal (§11) | `## Journal` (optional) | `journal.ndjson` | Append-only, mainline commits only |
| Changelog (§12) | `## Changelog` | `changelog.md` | Append-only |
| Decisions & open questions (§3) | `## Decisions & Open Questions` | `decisions.md` | Append-only |

Rules:

* An SDD governs exactly one **scope** — a whole small system, or one subsystem/feature of a larger one. Scopes of coexisting SDDs MUST NOT overlap in owned paths (§6).
* A repository SHOULD keep each SDD under a predictable root, e.g. `specs/<scope-slug>/` (bundle) or `specs/<scope-slug>.sdd.md` (single file), versioned alongside the code it governs.
* Single file SHOULD be chosen when the SDD has ≤ ~8 work units and one implementer at a time is expected; bundle otherwise. The mapping above MUST make the two shapes losslessly convertible.
* In bundle form, one work unit = one file. A unit's status lives in its own file's front matter, so two implementers never edit the same file to report progress.

---

## 2. Metadata

Every SDD begins with machine-readable front matter:

```yaml
---
sdd: checkout-core            # stable slug; never reused
title: Checkout Core Service
schema_version: 2.0           # required: the version of this schema
version: 1.4.0                # semver of this SDD document
status: Active                # Draft | Active | Deprecated
tdd: ./tdd/checkout-core.md   # the one TDD this SDD is derived from
tdd_version: 2.1 (Approved, 2026-09-14)
created: 2026-08-02
updated: 2026-10-25
scope_roots:                  # the governed tree (see §6)
  - src/checkout/
  - tests/checkout/
---
```

Rules:

* `version` MUST follow semantic versioning with these semantics: **MAJOR** — a contract (§7) or technology decision (§5) changed; **MINOR** — work units added, modified, or retired; **PATCH** — editorial only, no staleness implied. Every bump MUST have a matching change entry (§12).
* `status` lifecycle: `Draft` (being authored; units may be incomplete) → `Active` (living: built and maintained under §12) → `Deprecated` (retired; kept for history). There is no "Done": an Active SDD with all units Verified is simply fully built *at its current version*.
* `tdd` and `tdd_version` MUST identify a single TDD revision: `tdd` is the TDD's path, written from the directory of the SDD's core file; `tdd_version` begins with the revision it pins. An SDD is `Active` only against the current revision of an `Approved` TDD (TDD schema, Document Metadata). Re-pinning to a newer TDD revision is a change entry.
* `version` and `updated` MUST be those of the last change entry (§12): the version it goes to, and its date.
* `schema_version` is required, and names the version of this schema the instance is written to: `2.0`. A reader MUST refuse an instance that declares a version it does not read, or declares none, naming the version it reads.
* All dates are ISO `YYYY-MM-DD`.

---

## 3. Overview & TDD Reference

The Overview states, in at most a few paragraphs: what is being built, the scope boundary (in and out), and the pinned TDD. The TDD is the design authority; the SDD is its executable consequence.

Rules:

* Every consequential decision in the SDD MUST either cite a TDD section (e.g. "per TDD §3.1") or appear in the **Decision Log** as a `D-###` entry with a one-line local rationale. Decisions with neither are authoring defects.
* The SDD MUST NOT copy TDD rationale beyond one line per citation. The "why" lives in the TDD; duplicating it creates a second source of truth that will drift.
* The SDD MUST NOT prescribe, request, schedule, or imply changes to the TDD — no task, requirement, or note of the form "update the TDD to…". When building reveals a TDD gap or conflict, record an **Open Question** `Q-###` in the Decision Log and continue only with units it does not affect. Resolution arrives from outside the SDD as a new TDD revision; the SDD then re-pins via a change entry (§12).
* Alternatives rejected in the TDD (TDD §6) are closed. The SDD MUST NOT reopen or relitigate them.

Derivation map (guidance, not a required table): TDD §1 goals/non-goals → Overview scope; TDD §2 architecture → §6 layout and §7 contract skeleton; TDD §3 detailed design → §7 contract bodies and §8 implementation notes; TDD §4 cross-cutting concerns → §4 ground rules, §5 pins, §10 gate thresholds; TDD §5 rollout → §9 phase order.

Decision Log entry template:

```markdown
- **D-007** (2026-10-12, SDD 1.3.0) — TDD is silent on retry policy for the
  payment client. Decided: 3 attempts, exponential backoff 100ms base, jitter.
  Affects U-009.
- **Q-002** (2026-10-19, open) — TDD §3.2 schema omits currency on refunds;
  conflicts with C-004. Blocked: U-011. Building continues elsewhere.
- **Q-001** (2026-10-02, resolved 2026-10-09) — TDD §2.1 named no owner for
  the audit log. Resolved by TDD 2.1 (CH-004): U-006 owns it.
```

A question is **open unless its entry marks it resolved** — the word `resolved` in the parenthesis after its id. An entry that says nothing of its status is open. Resolving a question edits that parenthesis in place and appends the resolution to the entry; the entry is never deleted, and its `Q-###` never reused. A unit whose text depends on an open question carries its `OPEN(Q-###)` marker (§4), and every such marker MUST name a question that has an entry.

---

## 4. Ground Rules

Fixed conventions every reader and writer of the SDD obeys. This region is written once and MUST NOT change during building (changes are MAJOR).

**Normative language.** Requirement force uses the uppercase keywords MUST / MUST NOT / SHOULD / SHOULD NOT / MAY only. Lowercase "must/should/may" carry no normative weight.

**Definitions.**

* *Implementer* — any person or process that executes this SDD. The SDD addresses all implementers identically.
* *Unit* — the smallest independently claimable, buildable, verifiable increment (§8).
* *Contract* — a frozen cross-unit interface (§7).
* *Governed tree* — all paths under `scope_roots` (§2).
* *Mainline* — the integration branch of record. *Working copy* — an implementer's isolated checkout.
* *Gate* — an exact, non-interactive command sequence whose exit status decides a status transition (§10).

**Agnostic voice.** SDD text MUST be imperative ("Create `src/x.ts` exporting…") or EARS/requirement form (§8). It MUST NOT name coding assistants, orchestrators, IDEs, vendors, or any execution framework, and MUST NOT describe who or what the implementer is. The SDD MUST be self-contained: buildable offline from the SDD, its pinned TDD, and the repository; external links are provenance only, never required reading.

**Decision-completeness.** A region marked Ready contains zero unresolved choices. The only sanctioned placeholder is `OPEN(Q-###)`, and any region containing one cannot be Ready. A sentence that leaves the implementer a choice ("use a suitable hash", "pick a reasonable timeout") is a defect: resolve it or open a `Q-###`. `Q-###` does not license uncertainty — it quarantines it out of Ready work so this rule stays mechanically checkable (§14). A healthy SDD fires it rarely: the open-question count over time is an authoring-quality signal, managed to zero like a TBD/TBR register.

**Budget asymmetry.** The SDD is written once under maximal deliberation and read many times under bounded budgets. Every unit MUST be buildable from, at most: this Ground Rules region, §5 Technology Decisions, the unit's own §8 entry, and the contracts it `consumes`. If building a unit requires reading other units, the decomposition is wrong.

**Identifiers.** Units `U-###`, contracts `C-###`, decisions `D-###`, open questions `Q-###`, change entries `CH-###`; requirements are `U-###.R#`. IDs are assigned once and never reused, including after retirement.

---

## 5. Technology Decisions

Every technology choice, pinned exactly. Nothing here offers options; alternatives live in the TDD and are closed.

```markdown
| Dimension        | Decision                  | Pin                      | Authority |
| ---------------- | ------------------------- | ------------------------ | --------- |
| Language         | TypeScript                | 5.6.x, strict mode       | TDD §2.2  |
| Runtime          | Node.js                   | 22.11.0 LTS              | TDD §2.2  |
| HTTP framework   | fastify                   | 5.2.1                    | TDD §3.1  |
| Storage          | PostgreSQL                | 16.4; schema in C-003    | TDD §3.1  |
| Test framework   | vitest                    | 2.1.8                    | D-003     |
| Lint / format    | eslint 9.14.0 + prettier 3.3.3 | repo configs        | D-004     |
```

Rules:

* The table MUST cover at least: language(s) + version, runtime, every direct dependency with an exact version, storage, test framework, lint/format, and build system. Each row cites its TDD section or `D-###`. `none` is a valid decision for storage and for the test framework.
* The repository MUST carry lockfiles freezing the full transitive dependency graph; the lockfile is owned by a foundation unit (§9) and MUST NOT be regenerated by other units.
* A **Commands** block lists the exact, non-interactive commands for bootstrap, build, test, and lint, runnable from the repository root with deterministic exit codes:

```markdown
### Commands
- Bootstrap: `npm ci`
- Build:     `npm run build`          # exit 0, no warnings-as-errors
- Test:      `npm test`               # the whole suite; once green it stays green
- Lint:      `npm run lint`           # exit 0
```

* Bootstrap, Build and Lint MUST exit 0 from the repository root in the state the build starts from: building begins only from a green baseline, and the integration gate (§10) keeps all three green.
* Test runs the whole suite — every test that exists at that moment. It does not gate the start of a build: whether tests exist or pass before the build is the SDD's test posture (§10), so it MAY fail, or find nothing, at that point. From then on it is a regression check: once it passes on mainline no integration may turn it red, and the build is complete only when it passes (§10). It MUST therefore pass in every state in which the units built so far are correct — a test whose subject is not built yet skips, or is not collected, outside the Verification rows that demand it. Write `- Test: none` when no one command has that property, or the repository has no test command.

* A **Conventions** block states coding conventions as imperatives (naming, error policy, logging shape, directory idioms). Example: "All errors thrown across a contract boundary are `AppError` from C-001; never throw strings."
* The audit never reads source code, so whether one unit's code uses another's outside a contract (§7) is a language question it leaves to the SDD. To have it checked mechanically, pin a boundary checker for the language — a tool that fails when code crosses a unit boundary the contracts do not allow — and run it as the Lint command or a Verification row.
* Changing any row is a MAJOR change entry (§12) and marks every unit that depends on the changed dimension Stale.

---

## 6. System Layout & Ownership

The full file tree of the governed tree, with every path assigned to exactly one owning unit.

```markdown
| Path                          | Owner  | Notes                      |
| ----------------------------- | ------ | -------------------------- |
| package.json, package-lock.json | U-001 | foundation                 |
| src/checkout/index.ts         | U-001  | composition root           |
| src/checkout/cart/*           | U-004  |                            |
| src/checkout/payment/*        | U-009  |                            |
| tests/checkout/cart/*         | U-004  | tests live with their unit |
unmanaged:
  - vendor/
```

Rules:

* **Exclusive**: every path is owned by exactly one unit. Two units MUST NOT own the same path; glob patterns MUST NOT overlap.
* **Exhaustive**: any file under `scope_roots` that git would commit — tracked, or untracked and not ignored — and that is neither owned nor listed `unmanaged` is an **orphan** — an audit violation (§11). Files git ignores (dependencies, build output, caches) are outside the governed tree and need no entry.
* Units write only inside their owned paths (plus their own status fields, §8). Reading anything is allowed; *depending* on another unit's internals is not — cross-unit consumption goes through contracts only (§7).
* Test files are paths like any other: the unit that owns one may write it, and no other unit may (§13.3). Who owns the tests — and whether anyone does — is how an SDD states its test posture (§10).
* `unmanaged` is reserved for committed tool-generated or vendored paths that no unit edits by hand. No unit's work may change an `unmanaged` path either (§10, write-set check): tool output a build regenerates belongs in `.gitignore`.
* The ownership map is part of the stable core: changing it is a change entry, MINOR or MAJOR per §2.

---

## 7. Interface Contracts

A contract is the frozen, exact definition of one surface that crosses a unit boundary: exported types and function signatures, HTTP/RPC endpoints with schemas, database schemas, events, file formats, or CLI surfaces.

```markdown
# C-002 — Cart→Payment handoff

- Producer: U-004
- Consumers: U-009, U-012
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface
    export interface CheckoutIntent {
      cartId: string;          // UUIDv7
      amountMinor: number;     // integer, >= 0, currency minor units
      currency: "USD" | "EUR"; // ISO 4217 subset per TDD §3.2
    }
    export function submitIntent(i: CheckoutIntent): Promise<PaymentRef>;

## Semantics
- WHEN `amountMinor` is 0, THE SYSTEM SHALL resolve without contacting the
  payment provider and return `PaymentRef{kind:"noop"}`.
- Errors: rejects only with `AppError` (C-001), codes `CART_STALE | PAY_DECLINED`.
```

Rules:

* Every public surface that crosses a unit boundary MUST appear in exactly one contract. The audit checks contract references only (§11, check 3) — it never reads source code — so catching code that crosses a boundary without a contract is the job of a boundary checker the SDD pins for its language (§5).
* A unit MAY consume only the contracts listed in its `consumes` field — never another unit's internals, files, or private symbols.
* Contracts are written and frozen **before** their consumer units go Ready (contract-first). While any affected unit is Claimed or Built, a contract is read-only for implementers.
* Changing a contract happens only through a change entry: MAJOR if breaking, MINOR if strictly additive; the entry marks the producer and all consumers Stale.
* Contracts are exact: schemas, ranges, nullability, units, error codes, and ordering guarantees are all stated. "See implementation" is forbidden.
* The `Consumers:` line names exactly the units that list the contract in `consumes` — the producer apart, which MAY consume its own contract. The line restates the units' front matter and MUST be kept true to it.

### External contracts

A unit consumes only contracts of its own SDD, and its reading set (§4) and trace (§11) are built from that SDD alone. A surface that **another SDD owns** is therefore consumed through a contract of the consuming SDD that stands for it: an *external contract*, one whose `Producer:` line begins `external`. The contract it stands for is its *authority*; the authority stays the one contract the surface appears in.

It takes one of two forms: a **pin** of a contract another SDD of the repository owns, or a **transcription** of a source no SDD owns.

```markdown
# C-012 — Cart→Payment handoff (pinned)

- Producer: external — checkout-core, C-002
- Consumers: U-005
- Stability: Frozen by its producer
- Pin: checkout-core C-002 @ 9b11d2e07c44
```

* **Naming the authority.** After `external`, the line names the producing SDD by its `sdd` slug and then the contract's id (`checkout-core, C-002`). Other text MAY surround them. A line that names a contract id MUST name, before it, an SDD of the same repository that has that contract.
* **Pin.** The header (title, `Producer:`, `Consumers:`, `Stability:`) is the consuming SDD's own: its `Consumers:` are its own units. It also carries `- Pin: <slug> C-### @ <hash>`: the authority, and the hash of its body — from the `Surface` heading to the end of the contract, in a single file to the end of its `### C-###` region, the blank lines that separate it from what follows excepted (SHA-256, displayed truncated to 12 hex characters, as a trace hash is). A pin has no body of its own. A reader of it reads its own entry and then the authority's body, and MUST NOT be given that body unless it still hashes to the pin. A contract whose `Producer:` line names another SDD's contract MUST carry its `Pin:` line; without one it is invalid. A pin may be written without its hash (`- Pin: checkout-core C-002`); `vitrify pin sync` records it.
* **Transcription.** A `Producer:` line that names no contract id stands for a surface no SDD in the repository owns — a vendor API, a file format, another team's service. Its body is the consuming SDD's own transcription of that source. It names no authority, and nothing can check it: it is listed, never counted as in step.
* **Keeping in step.** A pin whose hash is not the hash of its authority's body is a defect of the consuming SDD: building MUST NOT start from it. Bringing it back in step (`vitrify pin sync` records the new hash) changes the consuming SDD's own contract, and so happens under that SDD's own change entry (§12), which marks its consumers Stale exactly as any other contract change does. Nothing of the producer's SDD changes.
* The consuming SDD never edits the authority, and its units read nothing of the producing SDD beyond that body.

---

## 8. Work Units

The unit is the atom of this schema: the smallest increment one implementer can claim, build, and verify alone, in one sitting.

Unit file template (bundle form; identical content under `### U-###` headings in single-file form):

```markdown
---
unit: U-012
title: Streaming record parser
status: Ready            # Draft | Ready | Claimed | Built | Verified | Stale | Retired
claimed_by: —            # lease holder identity while Claimed (§13)
claim_expires: —         # lease expiry, UTC; renewable (§13)
claim_seq: 0             # fencing token: increments on every claim (§13)
depends_on: [U-001, U-004]
parallel_group: B        # §9
owns:
  - src/checkout/ingest/parser.ts
  - tests/checkout/ingest/parser.test.ts
consumes: [C-001, C-002] # contracts only
built: —                 # e.g. 2026-10-20 @ SDD 1.3.0, commit 1a2b3c4
verified: —
trace: —                 # input hashes recorded at Built/Verified (§11)
---

## Requirements
- **U-012.R1** — WHEN the parser receives a line longer than 65,536 bytes,
  THE SYSTEM SHALL reject it with `AppError{code:"REC_TOO_LONG"}` without
  reading further input.
- **U-012.R2** — WHILE the input stream is open, THE SYSTEM SHALL emit parsed
  records in input order with at most 1 record of buffering.
- **U-012.R3** — IF the stream ends mid-record, THEN THE SYSTEM SHALL emit
  `AppError{code:"REC_TRUNCATED"}` naming the byte offset.

## Implementation
- Create `src/checkout/ingest/parser.ts` exporting
  `parseStream(src: Readable): AsyncIterable<Record>` (shape per C-002).
- Algorithm: single-pass byte scan; maintain a rolling buffer capped at 64 KiB;
  split on `\n`; decode UTF-8 strictly (throw on invalid sequences, R-style
  replacement is forbidden).
- Error handling: all failures through `AppError` (C-001); never partial records.
- Do not add dependencies; do not touch paths outside `owns`.

## Verification
| Covers        | Command                                   | Must satisfy        |
| ------------- | ----------------------------------------- | ------------------- |
| R1, R3        | `npm test -- parser.edge`                 | exit 0              |
| R2            | `npm test -- parser.order`                | exit 0              |
| all           | `npm run lint`                            | exit 0              |
```

Rules:

* **Requirements** use EARS forms — ubiquitous (`THE SYSTEM SHALL…`), event (`WHEN…`), state (`WHILE…`), option (`WHERE…`), unwanted behavior (`IF… THEN…`) — or Given/When/Then for scenario-shaped criteria. Each is singular, testable, and IDed.
* **Implementation** names exact paths, exported signatures, algorithm steps, edge cases, and error behavior. It MAY include code skeletons. It states prohibitions ("do not…") where implementers predictably stray.
* **Verification** maps every requirement ID to at least one exact command with a pass condition. A requirement with no check is a defect (traceability is total). A check need not be a test run: build output, a smoke run, and file or format checks are checks too (§10, Test posture). When the unit writes the tests its Verification runs, its Implementation MUST name each test and what it asserts — leaving that open is a choice left to the implementer (§4), who would then write its own exam.
* Sizing: a unit SHOULD carry 2–8 requirements and only artifacts within its owned paths. If a unit needs to edit another unit's path, the decomposition is wrong — fix it with a change entry, don't share the path.
* Status lifecycle: `Draft` (spec incomplete) → `Ready` (decision-complete, checks defined) → `Claimed` → `Built` (unit gate passed in working copy) → `Verified` (integration gate passed on mainline) → and from Built/Verified back to `Stale` when any recorded input-trace hash no longer matches the current content (§11) or a change entry declares it; `Retired` ends the line. Only the statuses, lease fields, stamps, and trace in front matter change after Ready; the spec body changes only via change entries.

---

## 9. Execution Order

The build plan over units: a dependency DAG plus named parallel groups.

```markdown
Phase 0 (foundation): U-001            # scaffold, lockfile, commands, CI config
Phase 1: group A — [P] U-003, U-004, U-005
Phase 2: group B — [P] U-009, U-012    # after A verified
Phase 3: U-015                          # integration slice; after B
```

Rules:

* `depends_on` edges MUST form an acyclic graph. Cycles are authoring defects.
* **Phase 0 is a foundation unit** (or units): repository scaffold, dependency installation and lockfile, the §5 Commands wired up, CI configuration. It blocks everything else; nothing is parallel with it.
* Units in the same parallel group (`[P]`) MUST be mutually independent: no dependency path between them, and — by construction of §6 — disjoint owned paths. Any implementer MAY take any unclaimed Ready unit from the active group.
* A unit is **claimable** when its status is Ready or Stale — or Claimed with a lapsed lease (§13) — and every unit in `depends_on` is Built or better.
* Groups SHOULD be ordered to produce a runnable end-to-end slice as early as possible (foundation → walking skeleton → breadth), not layer-by-layer completeness.

---

## 10. Verification Gates

Gates are exact command sequences; passing them is the only way a unit advances. No gate involves judgment.

* **Unit gate** (working copy; Claimed → Built): bootstrap + build + the unit's full Verification table + lint, all exit 0.
* **Write-set check** (before the merge): every path the work changed since its branch left mainline MUST lie in the unit's write-set (§13.3) — a path its `owns` globs match, or the unit's own entry changed only in its handoff fields `status`, `built` and `trace` (what stamping Built writes). Any other change rejects the work before it merges, even if otherwise green: a file the unit does not own (tests and `unmanaged` paths included), any other file of the SDD whatever an `owns` glob matches, or the unit's own spec text. The handoff commits every change git does not ignore, so build and test byproducts belong in `.gitignore`.
* **Input check** (after the merge, before the gate): the trace recorded when the unit was stamped Built MUST equal the hashes of its inputs as mainline now holds them (§11). Work with no Built stamp is rejected. Work built from text that has since changed is not merged either, through no fault of its own: the unit returns to the pool and is rebuilt from the current text.
* **Integration gate** (mainline; Built → Verified): after the unit's work merges: full build + the Verification tables of this unit **and every unit currently Verified** — which carry contract conformance — + the §5 Lint command + the §5 Test command as a regression check + drift-audit checks 1–3 (§11). The regression check: if Test passed on mainline before the merge, it MUST pass after it; while it already fails there — no tests yet, an earlier breakage — it judges no merge. A mainline that fails the integration gate MUST NOT be left failing: revert the merge or fix forward in the same integration session. Several Built units MAY be gated together — each merged on its own, the gate run once over the result — provided a red result is followed by gating them one at a time.
* **Fencing check.** The integration gate accepts work for unit U only if it presents U's **current** `claim_seq` and the lease is unexpired when the gate starts (§13). Work bearing a superseded token is rejected even if otherwise green — a delayed implementer whose claim lapsed and was re-claimed MUST NOT land.
* Gate outcomes are recorded as the `built` / `verified` stamps (date, SDD version, commit) and the input `trace` (§11) in the unit's front matter, and as `built` / `verified` events appended to the journal in the integration commit (§11); raw command output SHOULD be retained where the repository keeps build evidence.
* Verified is meaningful only at a version: a later change entry can return a Verified unit to Stale (§12).

### Test posture

Whether tests exist before the build, whether a unit's Verification runs existing tests, and whether building a unit includes writing tests are the author's choices, made per unit. This schema has no default. A posture is expressed only through regions defined elsewhere — ownership (§6), each unit's Requirements, Implementation and Verification (§8), and `depends_on` (§9) — and every gate treats a test file exactly like any other file.

| Posture | Who owns the tests (§6) | What the unit's Verification runs (§8) |
| --- | --- | --- |
| No tests | — | checks that are not test runs: build output, a smoke run, file and format checks |
| The unit writes its tests | the unit itself | the tests it writes, as its Implementation prescribes |
| A test unit writes them first | a test unit consuming the contracts its tests exercise; each implementation unit lists it in `depends_on` | the test unit's tests |
| Written before the build, governed by this SDD | a unit already `Verified` when the build starts (the authored baseline), consuming the contracts its tests exercise | those tests |
| Written before the build, outside this SDD | nobody: they lie outside `scope_roots` | those tests |
| Existing tests the SDD does not use | whoever owns them — nobody, outside `scope_roots` | nothing — and the §5 Test command leaves them out: then no gate runs them, and the build can break them unnoticed |

Postures mix freely across units — acceptance tests written before the build alongside unit tests each unit writes, for example. These rules hold in every posture:

* **Units are judged only by their gates.** A unit advances on the gates of this section — its Verification table plus each gate's fixed steps — and on nothing else. The §5 Test command is among those steps only as the integration gate's regression check: no unit has to make it pass, and no build has to start with it passing — but a unit may not break it, and a build is not complete while it fails.
* **Only a test's owner may change it.** The write-set check rejects every other unit's change to it, and a test no unit owns is read-only for the entire build. Edits made outside the build are caught only when they make a Verified unit's checks fail (§11, check 4).
* **Tests reach other units' code through contracts.** When a test owned by one unit imports a module another unit owns, the test's unit MUST consume a contract naming that module (§7) — which also stales the tests whenever the contract changes. The audit never reads code, so a boundary checker the SDD runs is what checks this (§5).
* **Ready and Stale units are rebuilt by implementers.** Any Ready or Stale unit can be claimed, and its implementer may rewrite every file the unit owns. Tests that only the author may write therefore belong to a unit that is Verified when the build starts — when a change entry stales that unit, the author updates and re-stamps it before building resumes — or lie outside `scope_roots`.
* **Verified checks keep passing.** A Verified unit's Verification commands run again at every build's preflight, at every later integration, and in the drift audit (§11, check 4), so they MUST pass in every later state — for a baseline, starting before the code its tests judge exists. A test unit or baseline whose tests judge other units' code is therefore verified with checks that hold both before and after that code exists (the tests load; they are lint-clean) — never with "the tests fail".
* **Absence fails.** A Verification command that runs tests SHOULD fail when the code it judges is absent: a test that skips instead lets a unit pass by not existing.
* **Checks stand alone.** Verification commands — one unit's, and different units' — MAY be run in any order and side by side. Each SHOULD therefore pass on its own, whatever else is running: it relies on nothing another command leaves behind and changes nothing another reads (its own temp directories and ports; no shared scratch state). An SDD whose checks cannot be made independent says so in Ground Rules (§4), and its gates are run one command at a time.
* **Tests-first is the unit's own instruction.** A unit MAY require tests-first: its Implementation says to run its Verification commands, observe them fail, then implement. The implementer carries this out; no gate observes or records the failing run.
* **State it where implementers read.** Ground Rules (§4) SHOULD state the SDD's test posture, so that it is in every unit's reading set.

---

## 11. State Ledger, Input Traces & Journal

Three artifacts carry implementation state. The **ledger** is the current table ("how much is built, what is stale" in one read). **Input traces** make staleness mechanical. The **journal** is the append-only history every state transition is recorded in.

### Ledger

```markdown
# Ledger — checkout-core @ SDD 1.4.0 (derived 2026-10-25)

Progress: 9/14 Verified · 1 Built · 1 Claimed · 2 Ready · 1 Stale

| Unit  | Status   | Group | Built @        | Verified @     | Note                        |
| ----- | -------- | ----- | -------------- | -------------- | --------------------------- |
| U-001 | Verified | —     | 1.0.0          | 1.0.0          | foundation                  |
| U-004 | Stale    | A     | 1.2.0          | 1.2.0          | trace: C-002 drift (CH-006) |
| U-009 | Claimed  | B     | —              | —              | lease to 2026-10-25T18:00Z  |
| U-012 | Built    | B     | 1.4.0          | —              | awaiting gate               |
```

* In bundle form, unit front matter is authoritative and `ledger.md` is **derived**: it MUST be regenerable from the unit files alone — and reproducible by replaying the journal — and MUST NOT be hand-edited except by regeneration. In single-file form the ledger table is authoritative.
* Every row carries the SDD version at build time; the version delta plus the changelog gives human orientation. The mechanical staleness authority is the trace below.

### Input traces

At Built (and re-stamped at Verified), the gate records in the unit's front matter a **verifying trace**: one hash per declared input of the unit.

```yaml
trace:                   # SHA-256, hex, MAY be displayed truncated to 12 chars
  unit:   4f2a91c03d7e   # this unit's file, volatile fields masked
  C-001:  9b11d2e07c44   # each consumed contract file
  C-002:  77aa01f3b9c2
  global: 5e30cc18aa09   # §4 Ground Rules + §5 Technology Decisions regions
```

* The trace covers exactly the §4 reading set — the unit file, its `consumes` contracts, and the global regions — so "what a unit is built from" and "what can stale it" are the same list by construction.
* Hashing is SHA-256 over UTF-8 bytes. The unit hash masks the volatile front-matter fields (`status`, `claimed_by`, `claim_expires`, `claim_seq`, `built`, `verified`, `trace`); everything else — including `depends_on`, `owns`, `consumes`, and the whole body — is content. No other normalization.
* **Staleness is a hash comparison**: a unit is Stale iff any recorded trace hash differs from the current content's hash. The mismatching key names the cause ("stale because C-002 changed"). Change-entry marks (§12) are declared intent; the audit cross-checks the two.
* A contract's hash is always over the consuming SDD's own entry for it. For a pin (§7) that is its own header with the `Pin:` line — and the pin is the hash of the authority's body, so the trace covers that body through it; for a transcription, its own text. A change to another SDD's contract therefore stales nothing until the pin is brought in step, which is why an out-of-step pin stops a build from starting rather than waiting for the audit.

### Build journal

Append-only, machine-readable history: `journal.ndjson` in bundle form (one JSON object per line), an optional `## Journal` section in single-file form.

```markdown
{"seq":41,"at":"2026-10-24T09:12:00Z","event":"claim","unit":"U-009","actor":"imp-7","claim_seq":3,"sdd":"1.4.0","commit":"8c1d2e9"}
{"seq":42,"at":"2026-10-25T11:02:14Z","event":"built","unit":"U-012","actor":"imp-2","claim_seq":5,"sdd":"1.4.0","commit":"f00dcafe"}
{"seq":43,"at":"2026-10-25T11:02:14Z","event":"verified","unit":"U-012","actor":"imp-2","claim_seq":5,"sdd":"1.4.0","commit":"f00dcafe"}
{"seq":44,"at":"2026-10-25T14:30:00Z","event":"change","entry":"CH-006","sdd":"1.4.0","stale":["U-004","U-009","U-012"],"commit":"77aa01f"}
```

* Event vocabulary: `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, `audit`. `seq` is a monotonically increasing integer; `at` is UTC ISO-8601.
* Entries are appended **only in commits that land on mainline** (claim/renew commits, integration commits, change commits). Because integration is serialized (§13), appends never race, lines are never partial, and the journal is a total order.
* The journal is never rewritten, compacted, or reordered. Current state (unit front matter, ledger) is a **projection** of it: replaying the journal from empty MUST reproduce the ledger exactly — that replay is an audit check below.

### Drift audit

Run at integration time and on demand; every check is mechanical:

1. Orphans: files under `scope_roots` that git would commit, not owned and not `unmanaged` (§6).
2. Ghosts: units Built or better whose owned artifacts are missing.
3. Contract references: a consumed contract that does not exist, or a contract naming a unit that does not exist (§7). Every check reads spec files and file paths only, never source code.
4. Rotten Verified: any Verified unit whose Verification table no longer passes.
5. Trace mismatch: any Built/Verified unit whose recorded trace differs from current content → Stale, cause named.
6. Projection consistency: journal replay reproduces the ledger; every Stale mark traces to a change entry or a trace mismatch; every change entry's declared marks were applied.
7. Lapsed leases: Claimed units whose `claim_expires` has passed are returned to claimable (§13).

Violations flag the unit (Stale or noted), are appended as `audit` events, and block integration until resolved.

Framing: the SDD is desired state, the codebase is actual state, the ledger is the state record, the journal is the transaction log, and the drift audit is the refresh that compares them.

---

## 12. Changelog & Change Management

An Active SDD changes only through change entries. The spec text is always **current truth** — readers never replay history to know what to build; history exists to compute impact.

Change entry template:

```markdown
## CH-006 — Cart line-item discounts (2026-10-23) — 1.3.0 → 1.4.0
- TDD pin: unchanged (2.1)
- Added:    U-013, U-014 (group C); contract C-007
- Modified: U-004 (R2 rewritten), C-002 (additive field `discountMinor`)
- Removed:  —
- Staleness applied: U-004 → Stale; U-009, U-012 → Stale (consume C-002)
- Codebase delta = build U-013, U-014; rebuild U-004, U-009, U-012
```

Rules:

* Every change entry MUST list: version bump, TDD pin status, units/contracts added–modified–removed, the staleness marks applied, and the resulting codebase delta in one line. Applying the marks — and appending the `change` event to the journal — is part of landing the entry: `vitrify change apply` does both from the entry, re-records the traces a PATCH entry re-stamps, and regenerates the ledger; `vitrify check` then says the SDD is consistent.
* Declared marks and mechanical staleness MUST agree: after an entry lands, the set of trace-mismatched units (§11) equals the set the entry marked Stale. A PATCH entry MAY instead **re-stamp** recorded traces to the new hashes without rebuilding — editorial changes don't invalidate builds; any mismatch not covered by a declared mark or re-stamp is an audit violation.
* Modified unit specs are edited **in place** (current truth); the previous text survives in version control and MAY additionally be archived beside the changelog.
* **No silent divergence.** Any integration that changes behavior beyond the spec without a change entry is an audit violation (§11.5). Spec and code move in the same change, not "code now, spec later".
* **Backflow.** When building reveals the spec wrong or incomplete, the implementer reports it and edits no spec text — the write-set check (§10) rejects work that does. A correction confined to the claiming unit's own non-contract spec text MAY be landed by the integrator with that unit's integration, as a PATCH/MINOR entry recorded in the same change; anything touching contracts, §5, §6, or other units MUST stop at a `Q-###` and wait for an authored version (§3, §7).
* **Changes that cross SDDs.** A change to a contract that other SDDs pin (§7) is a change entry of the producing SDD, and then — once each pin is brought in step (`vitrify pin sync`) — one change entry of each consuming SDD, with its own version bump and its own staleness marks. No SDD's entry marks another SDD's units.
* Removing scope: retired units get status `Retired`; the change entry states what artifacts are deleted, and the deletion is executed at integration of that entry.
* Growth is normal: new features arrive as new units and contracts under MINOR/MAJOR entries. The SDD is never "finished", only fully built at its current version.

---

## 13. Concurrency Protocol

How any number of implementers build one SDD at once, safely. The protocol is claim → isolate → build → gate → integrate → release.

1. **Claim = lease.** In the one file that owns the unit's status (the unit file in bundle form), set status Claimed, `claimed_by` (holder identity), `claim_expires` (now + lease duration), and increment `claim_seq` — the fencing token. At most one live claimant per unit; claim only claimable units (§9); the claim lands as a mainline commit appending a `claim` event. Lease duration SHOULD cover one build session; renew by extending `claim_expires` before it passes (a `renew` event). When a lease expires, the unit is simply claimable again — no cleanup negotiation; the next claim increments `claim_seq`, superseding the previous holder. Expiry timestamps come from wall clocks and are advisory; the token check at the gate (§10) is the correctness mechanism.
2. **Isolate.** Build in an isolated working copy of mainline taken at a commit where all `depends_on` are Built or better. Implementers never share a working directory or mutable build state. A working copy MUST NOT sit beneath a directory whose ignore files cover it: a checker that honours ignore files above its root would pass without reading a single file.
3. **Write-set discipline.** While Claimed, write only: the unit's owned paths, and the unit's own status/stamp fields. Contracts, §4–§6, §9, other units, the unit's own spec text, and every file the unit does not own — tests included — are read-only. The write-set check at integration (§10) enforces this mechanically. Needing to write elsewhere means stop → `Q-###`.
4. **Decision discipline.** Make no decision that another unit could observe unless it is written in a contract or this unit's spec. An unwritten cross-unit assumption is the canonical concurrency bug; raise `Q-###` instead.
5. **Gate and integrate serially, fenced.** Pass the unit gate, then merge to mainline through the integration gate, one integration at a time, presenting the unit's current `claim_seq` (§10) — a superseded token is rejected outright, which makes a lapsed-and-reclaimed unit safe against its former holder's late merge. Text conflicts are structurally rare (disjoint write-sets); **semantic** conflicts are expected occasionally and are exactly what the full integration gate exists to catch. Work that does not land is kept, not discarded: the next attempt at the unit starts from it.
6. **Hermetic verification.** Unit checks MUST be runnable concurrently with other implementers' checks: temp directories, ephemeral ports, no shared databases or global fixtures.

---

## 14. Authoring Checklist

Run before an SDD (or a new version of one) goes Active. Each line is pass/fail.

```markdown
- [ ] Derived from a pinned, Approved TDD; every decision cites TDD §n or D-###
- [ ] No OPEN(Q-###) inside any Ready region; no sentence leaves a choice open
- [ ] §5 covers language, runtime, every dependency (exact versions), storage,
      test, lint, build; Bootstrap, Build and Lint run clean from the repo
      root; Test need not pass yet, but is a command a correct build keeps
      green once it does (or `none`)
- [ ] §6 ownership is exclusive and exhaustive over scope_roots
- [ ] Test posture stated in Ground Rules and encoded in ownership and
      Verification (§10); tests only the author may write are owned by a unit
      Verified before the build, or lie outside scope_roots; a unit whose
      tests import other units' modules consumes their contracts
- [ ] Every cross-boundary surface appears in exactly one contract; every
      Consumers line names exactly the units that consume it
- [ ] Every pin matches the authority it names; every transcription
      names no contract, and says what it transcribes
- [ ] depends_on graph is acyclic; parallel groups are mutually independent
- [ ] Every requirement ID is covered by ≥1 exact Verification command
- [ ] Every unit is buildable from §4 + §5 + its own entry + its contracts alone
- [ ] Voice: imperative/EARS; no tool, vendor, or implementer-kind named
- [ ] Change entry written; version and updated are its own; staleness marks
      match trace mismatches (or re-stamps)
- [ ] Ledger regenerates clean; journal replay reproduces it
```

Authoring is the expensive step by design: the author spends deep, slow reasoning here so that every later build step is cheap, mechanical, and repeatable under a modest reasoning budget. When in doubt, move thinking from build time to author time.
