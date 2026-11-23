# Data Model and Formats Specification

> The normative SDD instance data model, input-trace hash recipe, reading set, journal, ledger, external-contract, and worker report formats that vitrify reads and writes.

Use this page when you need byte-level precision: to write tooling that reads vitrify's files, to review a change to vitrify itself, or to understand exactly why a unit went Stale. It specifies what vitrify implements on top of the documents you author. Those documents are defined by the embedded SDD and TDD schemas (`vitrify schema show`, `vitrify schema show --tdd`), which this page does not restate. For the ideas behind these formats, start with [Build State](../about/concepts/build-state.md) and [Contracts](../about/concepts/contracts.md).

> [!NOTE]
> Bracketed keys such as `[mokhov]` cite [Sources](sources.md). `RA-n` refers to a [rejected alternative](../about/design-rationale.md#rejected-alternatives). The key words MUST, MUST NOT, SHOULD, and MAY are normative in their uppercase-only meaning.

## 1. Overview

This specification covers the formats vitrify parses, hashes, appends, and derives.

### 1.1 Normative Sources

Use this list to find which source governs a question.

- **The embedded schemas** — `crates/vitrify-core/assets/schemas/sdd-schema.md` and `crates/vitrify-core/assets/schemas/tdd-schema.md` — define the documents' regions, rules, and lifecycles.
- **This page** defines how vitrify parses, hashes, records, and derives state from those documents.
- **The oracle** fixes the exact bytes: the calc example instance (`fixtures/example/`), the golden corpus (`fixtures/golden/`), and the seeded fixtures (`fixtures/seeded/`). Where this prose and the oracle disagree, the oracle is normative — the approved output is the specification [approvals] [oracle].
- **`vitrify-core::trace`** is the only hashing and masking implementation (§4).

### 1.2 Schema Version

Every SDD declares `schema_version` in its core front matter — the bundle's `sdd.md`, or the single file. This vitrify reads three versions: `2.0` (written `2`, `2.0`, or `2.0.x`), `2.1` (written `2.1` or `2.1.x`) and `2.2` (written `2.2` or `2.2.x`). In 2.1 a unit's `depends_on` may name a pin of its own SDD, to wait for the units that produce the pinned contract in another SDD; every 2.0 document means the same under 2.1, and a pin in a 2.0 SDD's `depends_on` is a lint L1 finding. In 2.2 the SDD's §6 System Layout & Ownership may list its tests under `tests:` (§3.6); every 2.0 and 2.1 document means the same under 2.2, and a `tests:` list below 2.2 is a lint L2 finding. A missing key or any other value is refused with exit 4, and the message names the versions this vitrify reads; `vitrify locate` still lists the refused SDD, so it is seen. Templates that `vitrify new sdd` materializes declare `2.2`. When the version changes is set out in [§10 Versioning](#10-versioning).

## 2. Instance Shapes and Discovery

These rules decide what an SDD looks like on disk and how vitrify finds it.

Two instance shapes [oracle]: **bundle** — `specs/<slug>/sdd.md` + `units/U-*.md` + `contracts/C-*.md` + `ledger.md` (derived) + `journal.ndjson` + `changelog.md` + `decisions.md`; **single** — `specs/<slug>.sdd.md` with units/contracts as `###` regions, the ledger as a section, the journal as a fenced block. Discovery order: `specs/*/sdd.md`, `specs/*.sdd.md`, then a repo-wide search over what git would commit (outside a repository, every file except under `.git` and `.vitrify`, tested on the root-relative path); a file counts only if its front matter has an `sdd` key; multiple SDDs without `--sdd` is exit 4.

## 3. Front Matter and Entries

These rules govern how vitrify reads the structured parts of an SDD.

### 3.1 Yamlish, Not YAML

**Yamlish, not YAML.** Front matter is parsed by an exact minimal-subset parser (scalars, inline and block lists, one-level nested maps, `#` comment stripping) — masking operates on raw lines, and full-YAML normalization would silently change the bytes the hashes cover (RA-7). All line splitting on the hashing/masking path is universal-newline (LF, CR, CRLF each terminate a line), rejoined with LF.

### 3.2 Unit Fields

Unit front matter carries: `unit`, `title`, `status` (`Draft|Ready|Claimed|Built|Verified|Stale|Retired`), `claimed_by`, `claim_expires` (UTC), `claim_seq` (fencing token, never reset), `depends_on`, `parallel_group`, `owns` (exclusive write-set globs), `consumes` (contract IDs), `built`/`verified` stamps (`YYYY-MM-DD @ SDD <version>, commit <short-sha>`), `trace` (hash map). Empty-marker set: `{"", "-", "—", "~", "null", "None"}`.

The volatile fields — `status`, `claimed_by`, `claim_expires`, `claim_seq`, `built`, `verified`, and `trace` — are the fields vitrify writes. The trace recipe masks them (§4), and the reading-set budget leaves them out (§5).

### 3.3 SDD Front Matter: `shared_inputs`

The core front matter may carry one optional key beyond the unit fields' own: `shared_inputs`, a list of repository-relative paths or globs. It declares which paths outside every SDD's `scope_roots` the SDD's commands read, and so which of them re-check its units. A missing key means every outside path is an input of every unit; a present key, even an empty list, means only the outside paths it matches are. The key is read by the re-check cascade alone (§6); it is not a trace input, and no lint check reads it. Why a repository declares it is in [Build State](../about/concepts/build-state.md#what-a-unit-is-checked-against).

```yaml
shared_inputs: [tools/, ci/shared.env]
```

### 3.4 Contracts, Change Entries, and Questions

A contract's header is the bullet lines between its title and its first section heading: `Producer:`, `Consumers:`, `Stability:`, and, on a pin, `Pin:`. Its body runs from its `Surface` heading to its end. A change entry is a heading that begins `CH-###`, with its date in parentheses and the version it goes to after `→`; a question is a `- **Q-###** (…)` entry of the Decision Log, open unless that parenthesis says `resolved`.

### 3.5 Claimable Predicate

**Claimable predicate** (derived, never stored): every `depends_on` entry is built — a unit entry is Built or Verified, and a pin entry (schema 2.1) stands for the units named on its authority's `Producer:` line, each of which must be Built or Verified — AND (status ∈ {Ready, Stale} OR (Claimed with lease lapsed)). Lease expiry is advisory; the fencing token is the correctness mechanism [locking].

### 3.6 SDD Layout List: `tests`

From schema 2.2, §6 System Layout & Ownership may list the SDD's test files under `tests:`. The list is read from the `System Layout & Ownership` section of the core file: a line whose trimmed text is `tests:`, then each following line that begins with whitespace and whose trimmed text begins with `- `, the item being the text after `- `, trimmed. The list ends at the first other line. It may sit inside a fenced block, as `unmanaged:` may.

```
tests:
  - tests/checkout/
```

The list is read only in a document that declares 2.2 or later (§1.2); below that it is ignored in every behavior except lint. It is not a trace input. It is read by the re-check cascade (§6), by the orphan checks, by lint L2 and by the write-set check. What the list means for ownership and re-checking is in [Test Posture](../about/concepts/test-posture.md).

## 4. Trace Hash Recipe

Every recorded stamp is checked against this recipe: one masking-rule drift silently stales every existing unit, so a change to any rule below raises `schema_version` and regenerates the golden corpus (§10).

- SHA-256 over the UTF-8 bytes of the input text; store the first 12 lowercase hex chars.
- Key set and order: `unit`, each consumed contract ID in listed order, then `global`.
- `unit` input: the unit's entry text with volatile fields masked (volatile keys: `status, claimed_by, claim_expires, claim_seq, built, verified, trace`; for `trace` the indented continuation lines drop too). Bundle shape re-assembles `---\n` + masked inner + `---\n` + body; single shape masks the metadata block inside the `### U-###` region.
- Contract input: the full text of the first match of sorted `contracts/C-###-*.md`, else `contracts/C-###.md` (single: the region). Always the consumer's own entry: a pin's entry holds its pin — the hash of the body it stands for — so the same recipe covers that body without reading it.
- `global` input: the `## Ground Rules` section + `"\n"` + the `## Technology Decisions` section of the core file.
- No other normalization. Stamps touch only masked fields, so `record` is stamp-invariant by construction. `verify` compares per key over the union of key sets on the shorter prefix (minimum 12); any mismatch is named; exit 3.

These are verifying traces in the Mokhov sense — "record `(key, Hash v)` per dependency; if any of the hashes is different, we rebuild" — hash equality *is* the up-to-date check, which is why the recipe must not drift [mokhov]. `vitrify-core::trace` is the **only** hashing/masking implementation; nothing may reimplement it.

## 5. Reading Set

A unit's reading set is what its worker reads and what its trace covers. It is, in order:

1. `## Ground Rules` of the core file;
2. `## Technology Decisions` of the core file;
3. the unit's own entry (bundle: its unit file; single: the `### U-###` region);
4. each consumed contract in `consumes` order (bundle: the first sorted `contracts/C-###-*.md`, else `contracts/C-###.md`; single: the `### C-###` region) — and, directly after a pin (§8.1), the body it stands for, as `<its file>  ::  Surface onward`.

The budget totals these bytes against `lint.readset_budget_bytes` (default 65,536). A pinned body counts at its own size. The unit's own entry counts as authored: the volatile fields (§3.2) are left out, exactly as the trace hash leaves them out, so the total is the same before a unit is claimed and after it is Verified. A missing required section, an unknown unit, a missing contract, or a pin that does not resolve or no longer matches its authority is an error (exit 4): a pinned body is handed out against a matching hash or not at all.

## 6. Journal

**Journal:** append-only NDJSON, one compact object per line (separators `(",", ":")`, non-ASCII unescaped), fixed key order `seq, at, event, [unit], [actor], [claim_seq], [entry], [stale], [cause], [findings], [suite], sdd, [commit]`; `event` is the closed vocabulary `{claim, renew, release, built, verified, stale, retired, change, audit}`; appended only inside commits that land on mainline, never rewritten. Two optional marks keep the vocabulary closed: a `verified` event may carry actor `recheck` (a unit that was already Verified had its Verification rows run again and passed; replay is unchanged), and an `audit` event may carry `suite` (`pass`: the §5 Test command passed at that commit). The commit of a unit's latest `verified` event, and of the latest `suite: pass` audit event, are the anchors the re-check cascade measures changes from. Replay projects events to expected unit states and `vitrify journal replay` compares the projection against front matter — event-sourcing replay as the audit: "discard the application state completely and rebuild it by re-running the events" [fowler].

Use this table to read an event's fields, which appear in the fixed order listed.

| Field | Present | Meaning |
| --- | --- | --- |
| `seq` | always | the previous event's `seq` + 1; strictly increasing |
| `at` | always | the UTC time of the append, ISO 8601 |
| `event` | always | one of `claim`, `renew`, `release`, `built`, `verified`, `stale`, `retired`, `change`, `audit` |
| `unit` | every kind except `change` and `audit` | the unit ID |
| `actor` | when given | the holder, `audit`, `recheck` (a `verified` event of a re-check), or another named actor |
| `claim_seq` | when given | the claim number the event belongs to |
| `entry` | `change` | the change entry, `CH-###` |
| `stale` | `change` | the units the change entry marks Stale |
| `cause` | `stale` | why the unit went Stale |
| `findings` | `audit` | the audit's finding count |
| `suite` | `audit`, optional | `pass`: the SDD's §5 Test command passed at the event's `commit`; it follows `findings` and precedes `sdd` |
| `sdd` | always | the SDD's version when the event was appended |
| `commit` | when given | the relevant commit, as a short SHA |

A re-check that passes is recorded in one commit, `sdd: re-check <units> at <commit>` (`sdd: re-check the suite at <commit>` when only the Test command ran). The commit appends a `verified` event with actor `recheck` and the checked commit for each unit that passed, rewrites each such unit's `verified:` stamp, regenerates the ledger, and, when the Test command passed, ends with an `audit` event that carries `suite: pass`. A unit whose rows failed gets nothing, and neither does an interrupted batch; failures are never recorded. The stamp is a volatile field, so no trace moves (§4). The `verified` event is the unit's anchor.

Replay projects each event onto unit state: `claim` → Claimed with its `claim_seq`; `release` → Ready; `built`, `verified` (a re-check's included: Verified stays Verified), `stale`, and `retired` → that status; `change` → Stale for each listed unit; `renew` and `audit` change nothing. Units with no events must be Draft or Ready, and unknown events and non-increasing `seq` values are findings. An example event:

```json
{"seq":41,"at":"2026-10-24T09:12:00Z","event":"claim","unit":"U-009","actor":"imp-7","claim_seq":3,"sdd":"1.4.0","commit":"8c1d2e9"}
```

## 7. Ledger

**Ledger:** derived, never hand-edited — header with the derived date, the progress line (`Progress: <V>/<total> Verified · …`, total excludes Retired), and a column-aligned unit table; golden comparison normalizes only the `(derived YYYY-MM-DD)` token.

The header is `# Ledger — <sdd> @ SDD <version> (derived <date>)`. The progress line lists statuses in the order Verified, Built, Claimed, Ready, Stale, Draft, Retired, with zero counts omitted. The table's columns are Unit, Status, Group, Built @ and Verified @ (the SDD version in each stamp), and Note (`lease to <expiry>` for a Claimed unit, `awaiting gate` for a Built one). A single-file SDD keeps the same content under `## State Ledger`.

## 8. External Contracts

An external contract is a contract whose `Producer:` line begins `external`: it stands, in one SDD, for a surface that SDD does not own. It takes one of two forms — a **pin**, when another SDD of the repository owns the contract (its *authority*), or a **transcription**, when no SDD does. Lint L9 holds each one to its source, and `vitrify pin list` and `vitrify pin sync` list and sync them. Why they exist, and how they gate a build, is explained in [Contracts](../about/concepts/contracts.md#external-contracts).

### 8.1 Pins

**A pin** keeps the consumer's own header and has no body: its `Pin:` line names the authority and the hash of the authority's body.

```markdown
# C-012 — Pool API: open, submit, admit (pinned)

- Producer: external — pool, C-010
- Consumers: U-005
- Stability: Frozen by its producer
- Pin: pool C-010 @ 4f2a91c03d7e
```

- **`Pin: <slug> C-### @ <hash>`** — the slug is the producing SDD's `sdd:` value and the hash is the trace's own digest (the first 12 hex characters of a SHA-256) of the authority's body. Written without a hash, the line is a pin `vitrify pin sync` has yet to record.
- **A `Producer:` line names an authority by slug and id**: the producing SDD's slug, then — however many words later — the contract's id. Other text may surround them (a path, an edge name); of several slugs before an id, the nearest names the SDD. It is resolved through discovery, so a repository lays its SDDs out as it likes. A line that names an authority makes the contract a pin. A line with a contract id and no SDD of the repository before it names an authority that cannot be found — `missing-authority`, a finding, never filed under "nothing to check": a producer whose slug changes must not turn its consumers' pins into text lint has stopped looking at. A line with no contract id names no authority: the contract is a transcription (§8.2).
- **The authority's body runs from its `Surface` heading to the end of the contract** — of its file in a bundle; of its `### C-###` region in a single file, where the blank lines after the region are its separator, not its text. An authority with no `Surface` heading has no body to pin, and the pin is `invalid`.
- **A pin has no body.** A contract with a `Pin:` line and a `Surface` heading is `invalid`. A contract whose `Producer:` line names another SDD's contract and that has no `Pin:` line is `invalid` too: write the `Pin:` line, and `vitrify pin sync` records the hash.
- **Readers follow the pin.** The reading set lists the pin's own entry and then, as a reading of its own, the body it stands for, read from the authority's file; `brief build-unit` inlines both; the budget counts both. The authority's header — its producer's units, its other consumers — is not read.
- **A body is handed out only against a matching hash.** If the pin no longer matches, `readset` and `brief` exit 4 and `run` sets the unit aside: a unit is never built from text its trace does not cover.
- **The trace recipe covers the pin, not the body.** A unit's `C-###` key hashes the consumer's own entry. That entry holds the pin and the pin holds the body's hash, so the trace covers the body through it, and the entry changes exactly when the pin does — which is what stales its consumers.

### 8.2 Transcriptions

**A transcription** is the consumer's own text of a source no SDD owns — a vendor API, a protocol, a file another team keeps. Its `Producer:` line begins `external` and names no contract id; its body, from the `Surface` heading on, is written and kept by the consumer.

```markdown
# C-020 — Payment gateway API (transcription)

- Producer: external — Acme Payments REST API v3, transcribed by U-007's owner
- Consumers: U-007
- Stability: follows the vendor's published reference

## Surface
…the consumer's transcription of the source…
```

vitrify cannot verify a transcription and says so: it is listed `unchecked`, never `ok`, and is no lint finding.

### 8.3 States

Use this table to read the state `vitrify pin list` gives each external contract. Lint L9 reports every state that is a finding.

| State | Kind | Meaning | Finding |
| --- | --- | --- | --- |
| `ok` | pin | the recorded hash matches the authority's body | no |
| `stale` | pin | the authority's body has moved; `vitrify pin sync` rewrites the hash | yes |
| `missing-authority` | pin | the slug or the id names nothing the repository holds | yes |
| `invalid` | pin | a pin that carries a body; a `Producer:` line naming another SDD's contract with no `Pin:` line; an authority with no `Surface` heading; a slug two SDDs share; a `Pin:` line that does not read `<slug> C-### @ <hash>` | yes |
| `unchecked` | transcription | no SDD owns the source, so nothing can be compared | no |

## 9. Worker Report Line and Escalation Verdict

These are the two lines of model output the orchestrator acts on.

**Worker report line**: the last non-empty line of the worker's final output — `{"unit":"U-###","outcome":"built|blocked|released","branch":"sdd/<sdd>/U-###/c<seq>","claim_seq":N,"q":"Q-### or null","notes":"<one line>"}`. A missing or unparseable report ⇒ treated as released, the claim freed, the attempt counted as failed. The orchestrator acts on the unit and claim it dispatched, not on what the line says they are; `branch` is the job block's, echoed. When the harness streams its session as events, the report is the last line of the last event that carries text.

The **escalation verdict** is a single JSON object, `{"verdict":"retry|release|block|abort","unit":…,"q":…,"reason":…}`, parsed and enforced by the orchestrator, not by the model. One malformed reply gets one re-ask; a second is an abort. Verdicts land in the run record and the logs; only their effects append journal events, so the journal's closed vocabulary (§6) has no verdict event.

## 10. Versioning

These rules say what a release may change and how the formats on this page tell versions apart.

- **vitrify is at 0.y.z.** Any release may change any command, flag, output, exit code, file format, or state file, with no deprecation period, no alias, and no step that carries an older file forward [semver-web] [unreleased-api]. Agents learn the interface from the installed binary in each session, so a release leaves no stale copy of it behind [poehnelt].
- **`schema_version` is required in every SDD** (§1.2). A vitrify reads the schema versions it names (this one: 2.0, 2.1 and 2.2) and refuses any other with exit 4, naming the versions it reads; `vitrify locate` lists the refused SDD.
- **A change to the document format or to the trace recipe raises `schema_version`** and regenerates the golden corpus in the same reviewed change. An unintended change to either is a bug, which the goldens catch.
- **State under `~/.vitrify/` belongs to the installed version.** Unmodified content refreshes through the manifest; a file this version cannot read is an error that names the command that rewrites it.
- **The [Release Notes](../about/release-notes.md) say briefly what each release changed.** No other page carries change history [ecs-changelog] [intel-spec-update].
- **At release status (1.0.0), this section is where a deprecation policy goes.** What such a policy needs is already kept: the `schema_version` each SDD declares, the refusal that names the version a release reads, and the content manifest that records which version wrote the files under `~/.vitrify/` [clig] [cargo-metadata].

In a repository, vitrify writes only what its verbs own — spec state, the templates `vitrify new sdd` and `vitrify new tdd` materialize, and `.vitrify/` (requirement R9 in [Design Rationale](../about/design-rationale.md#requirements-and-goals)).

## 11. What These Formats Are Not

Use this list to avoid reading more into the formats than they promise.

- **Not YAML.** Front matter is read by the yamlish parser (§3.1); a full YAML reader would normalize what the hashes cover (RA-7).
- **Not cross-SDD except through a pin.** A reading set leaves its own SDD only to read the body a pin stands for, and only against a matching hash (§8.1); nothing else in it, and nothing its trace covers, lies outside the SDD.
- **Not versioned per output.** JSON output carries no format version of its own; each verb's `--help` documents the shape the installed version prints (§10). A per-output version — cargo's "It is recommended to include the `--format-version` flag to future-proof your code" [cargo-metadata] — is the mechanism to weigh at release status, with the deprecation policy.
- **Not a run log.** `.vitrify/run.json`, the gate's progress file `.vitrify/logs/<job>/gate.json`, the event log `.vitrify/logs/<job>/events.jsonl`, the final report `.vitrify/logs/<job>/report.txt`, a project run's plan record `.vitrify/logs/<job>/project.json` and its per-lane records under `.vitrify/logs/<job>/<slug>/`, and the worker logs are observability only and are not part of this specification; vitrify never decides anything from them.
- **Not hand-editable.** The ledger is derived and the journal is append-only; change neither except through the state verbs.

## 12. Reference Implementations

Use this table to find the code that implements each format and the oracle files that fix it.

| Format | Implementation | Oracle |
| --- | --- | --- |
| Discovery, front matter, and entries (§2–§3) | `crates/vitrify-core/src/model.rs`, `yamlish.rs`, `newline.rs` | `fixtures/example/specs/calc/`, `fixtures/golden/calc/locate.json` |
| Trace (§4) | `crates/vitrify-core/src/trace.rs` | `fixtures/golden/calc/trace-U-*.json` |
| Reading set (§5) | `crates/vitrify-core/src/readset.rs` | `fixtures/golden/calc/readset-U-*.json`, `fixtures/golden/pin-multi/readset-*.json` |
| Journal (§6) | `crates/vitrify-core/src/journal.rs` | `fixtures/golden/calc/replay.json` |
| Anchors, dirty sets, and `shared_inputs` (§3.3, §6) | `crates/vitrify-core/src/cascade.rs` | `fixtures/seeded/project-cascade/`, `crates/vitrify-core/tests/cascade.rs` |
| The unit graph across SDDs (§1.2, §3.5) | `crates/vitrify-core/src/graph.rs` | `fixtures/seeded/project-pipeline/`, `fixtures/seeded/project-cycle/`, `fixtures/seeded/project-schema20/` |
| Ledger (§7) | `crates/vitrify-core/src/ledger.rs` | `fixtures/golden/calc/ledger.md`, `fixtures/golden/calc/progress.txt` |
| External contracts (§8) | `crates/vitrify-core/src/contract.rs` | `fixtures/seeded/pin-multi/`, `fixtures/golden/pin-multi/` |
| Audit findings | `crates/vitrify-core/src/audit.rs` | `fixtures/golden/calc/audit.json`, `fixtures/seeded/audit-*/` |
| Lint findings | `crates/vitrify-cli/src/cmd/lint.rs` | `fixtures/seeded/lint-L*/`, `fixtures/seeded/lint-tests/` |

## Related Topics

- [Build State](../about/concepts/build-state.md) — what traces, the journal, and the ledger are for.
- [Contracts](../about/concepts/contracts.md) — why external contracts exist and how they gate a build.
- [Command Reference](../cli/command-reference.md) — the verbs that read and write these formats, and their exit codes.
- [Release Notes](../about/release-notes.md) — what each release changed.
- [Sources](sources.md) — the citations on this page.
