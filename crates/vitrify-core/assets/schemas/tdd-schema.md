# TDD Schema — the TDD→SDD Handoff

This document does **not** teach how to write a Technical Design Document; TDD practice is well established and your organization already has one. It defines the other half of the problem: **what [`sdd-schema.md`](./sdd-schema.md) consumes from a TDD**, section by section, and therefore what a TDD must contain — and keep stable — to be *SDD-ready*. A TDD that satisfies this schema can be mechanically pinned, cited, and derived from; one that doesn't will stall SDD authoring with open questions.

The division of labor is strict and total:

| | TDD | SDD |
| --- | --- | --- |
| Records | the **why**: problem, goals, architecture, rationale, rejected alternatives | the **what and exactly how**: pins, layout, frozen contracts, units, gates, state |
| Changes by | revisions, human-reviewed and Approved | change entries (SDD §12), continuously, during building |
| Read by | reviewers and the SDD author, once, deeply | every implementer, many times, under a bounded budget (SDD §4) |
| Carries state | none — a TDD never tracks progress | ledger, journal, input traces (SDD §11) |

The TDD drives the SDD: an SDD is derived from one Approved TDD revision, cites it for every consequential decision, never restates its rationale beyond one line per citation, and never changes it (SDD §3). In this document, **§n** means a TDD section defined below; **SDD §n** cites `sdd-schema.md`. MUST / MUST NOT / SHOULD / MAY are normative.

**The consumption map** (the reverse view of SDD §3's derivation map):

| TDD section | Consumed by | What the derivation extracts |
| --- | --- | --- |
| Metadata | SDD §2 front matter | the pin: path, revision, `Approved` status, date |
| §1 Context & Scope | SDD §3 Overview | the scope boundary (in and out); measurable goals → gate pass conditions |
| §2 High-Level Architecture | SDD §2 `scope_roots`, §6 ownership map, §7 contract skeleton | components → owned subtrees; every cross-component edge → exactly one contract |
| §3 Detailed Technical Design | SDD §7 contract bodies, §8 Implementation blocks | exact schemas, payloads, error codes; algorithms and edge cases |
| §4 Cross-Cutting Concerns | SDD §4 Ground Rules, §5 pins & Conventions, §10 gate thresholds | conventions as imperatives; technology rows; numeric thresholds |
| §5 Execution & Rollout Plan | SDD §9 Execution Order | phase order; migration ordering → `depends_on` edges |
| §6 Alternatives & Technical Debt | SDD §3 (closed doors), §8 prohibitions, future §12 change entries | what must not be relitigated; deferred work that will arrive as new units |

---

## Document Metadata — consumed as the pin

Whatever metadata block your TDD template uses, the SDD needs three things from it, because SDD §2 pins the TDD as `tdd: <path>` and `tdd_version: 2.1 (Approved, 2026-09-14)`:

* **An identifiable revision.** A version number (or an unambiguous date + status) the pin can name. A pinned revision is immutable: editing an Approved TDD produces a *new* revision, never a silent rewrite of the pinned one — otherwise the pin, every `per TDD §n` citation, and the staleness audit (SDD §11) lose their referent.
* **An `Approved` status.** An SDD MUST NOT go `Active` against a Draft or In-Review TDD (SDD §14: "Derived from a pinned, Approved TDD"). Deprecating a TDD signals its SDDs to re-pin to a successor or retire, via change entries.
* **Stable section numbering.** SDD text cites `TDD §3.1`-style anchors permanently. Section numbers MUST be stable across revisions — new material appends; removed material leaves a tombstone — the same discipline SDD §4 applies to IDs ("assigned once and never reused").

One SDD pins exactly one TDD revision. One TDD MAY drive several SDDs, provided their scopes don't overlap in owned paths (SDD §1). Re-pinning to a newer revision is always a change entry (SDD §2, §12) that names what it stales.

## 1. Context & Scope — consumed as the Overview boundary

Problem statement, goals, non-goals. The SDD consumes this as:

* **The scope boundary.** SDD §3's Overview states what is being built and what is in and out — that text is derived from §1's goals and non-goals. Non-goals are load-bearing: anything excluded here MUST NOT surface as an SDD work unit, and a unit with no grounding in §1's goals is an authoring defect waiting for the SDD §14 checklist.
* **Measurable outcomes → pass conditions.** A goal stated as a number ("p95 < 120 ms", "reduce API latency under 50 ms") becomes a Verification pass condition or gate threshold (SDD §8, §10). A goal the SDD author cannot turn into an exact check will come back as a `Q-###`.
* **Rationale that is cited, never copied.** The problem statement is the "why" the SDD points at with one-line citations (SDD §3's no-duplication rule). If the SDD author feels the need to copy paragraphs of §1, either the TDD buried a decision here or the SDD is drifting into rationale — both are defects.

## 2. High-Level Architecture — consumed as ownership and the contract skeleton

The system diagram and component inventory. The SDD consumes this structurally:

* **Components → owned subtrees.** Each component maps to a region of the SDD §6 ownership map (and informs `scope_roots`, SDD §2). The component boundary is where the SDD will draw exclusive path ownership, so a component whose responsibility is fuzzy here becomes overlapping `owns` globs there — an audit violation.
* **Edges → the contract list.** Every arrow that crosses a component boundary in §2's diagram MUST be enumerable, because each one becomes **exactly one** SDD §7 contract (SDD's rule: every cross-boundary surface appears in exactly one contract; cross-unit consumption goes through contracts only). The SDD author derives the contract *skeleton* — the list of C-### IDs, producers, consumers — from this section alone; §3 then fills the bodies. An edge the diagram implies but never names becomes code that crosses a boundary with no contract — which no audit check can see, since the audit never reads code; only a boundary checker the SDD pins for its language catches it (SDD §5, §7).
* **Hosting and framework notes → §5 pin candidates.** Component-level technology statements (per-component runtime, framework) feed rows of the SDD §5 table, cited back as `TDD §2.2`.

## 3. Detailed Technical Design — consumed as contract bodies and implementation notes

Data models, schemas, API specifications, algorithms. This is the most heavily consumed section, and the one with the strictest precision demand:

* **Schemas and data contracts (§3.1, §3.2) → frozen contract bodies.** The SDD §7 contract freezes *exactly* what this section specifies: field types, ranges, nullability, units, error codes, ordering guarantees. The SDD cites back (`shape per C-002`, `ISO 4217 subset per TDD §3.2`) but the SDD's copy is the buildable one — so §3 must be exact enough that freezing it requires no invention. "See implementation" is forbidden on the SDD side (SDD §7), which means "TBD at implementation time" is forbidden here.
* **Algorithms and edge cases → §8 Implementation blocks.** Design-level algorithm descriptions become the step-by-step Implementation text of work units, with exact paths and signatures added at SDD-authoring time.
* **The precision ladder.** Where §3 is *silent on a local choice* that doesn't contradict anything (a retry count, a buffer size), the SDD author decides it and records a `D-###` with a one-line rationale (SDD §3) — no TDD revision needed. Where §3 is *ambiguous, self-contradictory, or conflicts with a contract*, the SDD records a `Q-###`, blocks only the affected units, and waits: resolution arrives from outside the SDD as a new TDD revision (SDD §3). The more decisions §3 actually makes, the fewer D-###s and Q-###s the SDD accrues — move thinking into the TDD, not downstream.

## 4. Cross-Cutting Concerns — consumed as ground rules, pins, and thresholds

Security, observability, performance, compliance. The SDD consumes this in three streams:

* **Conventions → SDD §4 Ground Rules and §5 Conventions, as imperatives.** "Structured JSON logs with `trace_id`" becomes a Conventions bullet every unit obeys; an auth model becomes Ground Rules text plus contract error semantics. Anything stated here as a value ("TLS 1.3", "AES-256") is pinned verbatim.
* **Technology constraints → SDD §5 rows.** Each cross-cutting technology choice becomes a pinned row (exact version added at SDD time), cited as `TDD §4.x`.
* **SLAs → SDD §10 gate thresholds.** Latency targets, error budgets, and load figures become the numeric pass conditions of Verification commands and integration gates. A threshold given as a range or an aspiration ("fast enough") cannot become a gate — the SDD author will return a `Q-###` asking for the number.

## 5. Execution & Rollout Plan — consumed as phase order

Migration strategy, release sequence, rollback. The SDD consumes the *ordering*, not the operations:

* **Rollout phases → SDD §9 phases.** The release sequence shapes the Execution Order: what must exist before what, where the walking skeleton lands, what can go wide in parallel groups. SDD §9's own rules still govern (foundation first, acyclic `depends_on`, runnable slice early) — §5 informs the ordering; it doesn't override those invariants.
* **Migration ordering → dependency edges.** Backward-compatibility and zero-downtime sequencing become `depends_on` edges between units (schema unit before consumer units, flag unit before flagged behavior).
* **Rollback machinery → units, when the build must deliver it.** If rollback requires artifacts (feature flags, down-migrations), those are work units with requirements like any other. Runtime rollback *procedure* stays operational documentation — the SDD tracks what must be built, not how to operate it.

## 6. Alternatives Considered & Technical Debt — consumed as closed doors and future work

* **Rejected alternatives (§6.1) are closed.** The SDD MUST NOT reopen or relitigate them (SDD §3), and this is enforced in two directions: SDD §8 Implementation blocks cite a rejection where implementers predictably drift toward it ("do not introduce a cache here — rejected per TDD §6.1"), and no `Q-###` resolution or `D-###` may quietly resurrect one — resurrecting a rejected alternative requires a new TDD revision that un-rejects it. For this to work, each rejection MUST be stated as a findable, named decision, not a paragraph of vibes.
* **Accepted technical debt (§6.2) is deferred scope.** Each debt item is either visible in the SDD as later-phase units, or absent now and expected later: when the debt is paid, it arrives as a new TDD revision, and the SDD grows via a MINOR/MAJOR change entry (SDD §12) adding the units and contracts. Debt listed here is the forecast of the SDD's future changelog.

---

## The feedback loop

The interface runs in both directions, but never symmetrically:

1. **Forward (TDD → SDD):** author the SDD from an Approved revision; pin it (SDD §2); every consequential decision cites `TDD §n` or records a `D-###` (SDD §3, §14).
2. **Backward (SDD → TDD):** building reveals a gap or conflict → the SDD records `Q-###` in its Decision Log, blocks only the affected units, and continues elsewhere. The SDD MUST NOT prescribe, request, schedule, or imply changes to the TDD — no "update the TDD to…" anywhere in SDD text (SDD §3).
3. **Resolution (outside the SDD):** the TDD is revised and re-Approved through your organization's own process. The SDD then re-pins via a change entry (SDD §12) that closes the `Q-###`, applies the declared staleness marks, and states the codebase delta. The input-trace hashes (SDD §11) make the downstream effect mechanical: units whose reading set the change touched go `Stale` and get rebuilt; nothing else moves.

A local choice the TDD never needed to make goes the short way — `D-###`, one line, no revision. Only genuine design authority round-trips through the TDD.

## SDD-readiness checklist

Run before handing a TDD revision to SDD authoring. Each line is pass/fail:

```markdown
- [ ] Status is Approved; the revision has a pinnable identifier (version, date)
- [ ] Section numbering matches this schema (§1–§6) and is stable across revisions
- [ ] §1 non-goals are explicit; every measurable goal carries its number
- [ ] §2 names every component and every cross-component edge (the contract list
      is enumerable from the diagram alone)
- [ ] §3 is exact enough to freeze: schemas, fields, error codes, orderings —
      no "TBD at implementation time"
- [ ] §4 states thresholds as numbers a gate can check
- [ ] §5 gives an unambiguous ordering (what exists before what)
- [ ] §6.1 rejections are named, findable decisions with their reasons
- [ ] No implementation state anywhere: no task lists, no progress, no
      verification commands — those are SDD artifacts (SDD §8–§11)
```

When in doubt, the test for any sentence is: *could an SDD author turn this into a pin, a contract line, a requirement, or a threshold without asking anyone?* If yes, it belongs here and will be consumed. If no, either sharpen it or expect it back as a `Q-###`.
