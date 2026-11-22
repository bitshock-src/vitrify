# Specs: TDD and SDD

> Understand the two documents vitrify builds from — the TDD that records why and the SDD that records what and exactly how — and how each one changes.

Use this page to learn what you write before vitrify builds anything. vitrify works from a pair of documents. A **TDD** (Technical Design Document) is the design authority. An **SDD** is the buildable specification derived from it: decision-complete, split into work units, and carrying the state of its own build. This page explains the split and the SDD's structure. The normative definitions are the two schemas embedded in the binary, printed by `vitrify schema show` (SDD) and `vitrify schema show --tdd` (TDD); for the step-by-step procedure, see [Author a Spec](../../user-guides/author-a-spec.md).

## Why Two Documents Exist

The pair splits authority, so that the design changes rarely and deliberately while the buildable spec changes as often as the build needs. Use this table to see what each document owns.

| | TDD | SDD |
| --- | --- | --- |
| Records | the **why**: problem, goals, architecture, rationale, rejected alternatives | the **what and exactly how**: pinned technology, layout, frozen contracts, units, gates, state |
| Changes by | revisions, human-reviewed and Approved | change entries, continuously, during building |
| Read by | reviewers and the SDD author, once, deeply | every implementer, many times, under a bounded budget |
| Carries state | none — a TDD never tracks progress | ledger, journal, input traces |

The TDD drives the SDD: an SDD is derived from one Approved TDD revision, cites it for every consequential decision, never restates its rationale beyond one line per citation, and never changes it. Keeping the why out of the SDD keeps what each worker reads small. [Design Rationale](../design-rationale.md#schema-grounding) records the long-lived-spec practice this split comes from.

## What an SDD Contains

An SDD is a fixed sequence of regions, each defined by one section of the SDD schema. Use this table to find where each kind of decision lives.

| Region (schema §) | Holds | Changes while building? |
| --- | --- | --- |
| Metadata (§2) | slug, version, status, TDD pin, `scope_roots`, `schema_version` | No — author only |
| Overview & TDD Reference (§3) | the scope boundary and citations of the TDD | No |
| Ground Rules (§4) | rules every unit reads, including the test posture | No |
| Technology Decisions (§5) | exact language, runtime, and dependency pins, and the four §5 Commands | No |
| System Layout & Ownership (§6) | the file tree, which unit owns what, and `unmanaged` paths | No |
| Interface Contracts (§7) | the frozen cross-unit surfaces | No |
| Work Units (§8) | one entry per unit | Status fields only |
| Execution Order (§9) | phases and parallel groups | No |
| Verification Gates (§10) | the gates and the test posture rules | No |
| State Ledger, Input Traces & Journal (§11) | the derived status table and the append-only event log | Yes, by vitrify |
| Changelog (§12) | change entries | Append-only |
| Decisions & Open Questions (§3) | `D-###` decisions and `Q-###` questions | Append-only |

The **§5 Commands** — Bootstrap, Build, Test, and Lint — run from the repository root at every gate. Bootstrap, Build, and Lint must pass before a build starts. Test need not: whether tests exist or pass before the build is the SDD's [test posture](test-posture.md), and the command may be `none`.

## SDD Shapes

An SDD is either a bundle or a single file. Both carry the same regions under the same headings; choose by size and by how many workers you expect at once.

| Shape | Layout | Choose when |
| --- | --- | --- |
| Bundle | `specs/<slug>/sdd.md`, `units/U-*.md`, `contracts/C-*.md`, `ledger.md` (derived), `journal.ndjson`, `changelog.md`, `decisions.md` | the default, and the shape for parallel builds |
| Single file | `specs/<slug>.sdd.md`, with units and contracts as `###` regions, the ledger as a section, and the journal as a fenced block | about eight units or fewer, built one at a time |

In a bundle, each unit is its own file, so parallel workers claim and stamp their units without touching the same file. Under `vitrify run`, a single-file SDD's lane takes one worker at a time, because every status edit lands in the same file. `vitrify new sdd` creates a bundle and `vitrify new sdd --single` creates a single file.

## SDD Lifecycle

An SDD's `status` moves from Draft to Active to Deprecated. There is no "Done".

- **Draft** — being authored; units may be incomplete.
- **Active** — living: built, and maintained through change entries. An SDD goes Active only when `vitrify lint` exits 0, and only against the current revision of an Approved TDD (lint L11).
- **Deprecated** — retired and kept for history.

An Active SDD with every unit Verified is fully built at its current version. The next change entry can make it partly built again, and that is routine.

## Decisions and Questions

A decision-complete SDD leaves the implementer no choices, because a choice made differently by two parallel workers is a conflict. These rules keep it that way.

- A sentence that leaves the implementer a choice is a defect — decide it, or open a `Q-###` and stop rather than guess.
- A local choice the TDD does not make becomes a `D-###` entry in the Decision Log, cited where it applies. The SDD never edits the TDD upward.
- A gap that needs design authority becomes a `Q-###`. Mark the affected unit text `OPEN(Q-###)`; lint L8 keeps such a unit from being Ready, and requires every marker to name a question the Decision Log holds.
- A question is open until its entry says `resolved`, as in `- **Q-002** (2026-10-19, resolved 2026-10-24) — …`.
- Workers follow the same rule at build time. A worker that meets a gap stops at a question; `vitrify run` lands it in the Decision Log and parks the unit until its reading set changes.

`vitrify questions --open` lists the open questions and the units each one blocks.

## Versions and Change Entries

The SDD's `version` is a semantic version with fixed meanings, so that a version number alone tells you what kind of rebuild to expect.

| Bump | When |
| --- | --- |
| MAJOR | a contract or a technology decision changed |
| MINOR | units were added, modified, or retired |
| PATCH | an editorial change that stales nothing |

Every bump has a `CH-###` change entry in the changelog, and the SDD's `version` and `updated` must be the last entry's (lint L10). The spec text is always current truth, edited in place; history exists to compute impact. See [Change a Spec](../../user-guides/change-a-spec.md).

## Schema Version

An SDD declares the schema version it is written to in `schema_version`, in its core front matter. The key is required, and this vitrify reads two versions: `2.0` (written `2`, `2.0`, or `2.0.x`) and `2.1` (written `2.1` or `2.1.x`). In 2.1 a unit's `depends_on` may name a pin, to wait for the units that produce the pinned contract in another SDD; every 2.0 document means the same under 2.1. A missing key or any other value is refused with exit 4 and a message naming both versions; `vitrify locate` still lists the refused SDD, so it is seen. `vitrify new sdd` writes `schema_version: 2.1`. The rule behind the read versions is in the [Data Model and Formats Specification](../../reference/formats.md#10-versioning).

## Several SDDs in One Repository

A repository can hold several SDDs, and vitrify scopes everything to one of them at a time. Each SDD has its own `sdd:` slug and disjoint `scope_roots`; unit IDs, reading sets, traces, branches, and worktrees all belong to one SDD. Name the SDD with `--sdd <root>` (`vitrify run` builds every Active SDD when it is not named), or pass `--all` to the verbs that check (`lint`, `check`, `audit`, `ledger`, `pin`, `journal replay`, `change apply`, and `questions`). When one SDD codes against another's surface, it does so through an [external contract](contracts.md#external-contracts). See [Work With Several SDDs](../../user-guides/multiple-sdds.md).

vitrify finds SDDs in a fixed order — bundles at `specs/*/sdd.md`, single files at `specs/*.sdd.md`, then a repository-wide search over the files git would commit — and counts only files whose front matter has an `sdd` key. The exact rules are in the [Data Model and Formats Specification](../../reference/formats.md#2-instance-shapes-and-discovery).

## Limitations

These are the boundaries of what vitrify checks about your documents.

- vitrify never edits a TDD. Lint L11 checks the TDD pin fully only where the TDD carries the metadata table of vitrify's own template; elsewhere it checks only that `tdd:` names a file.
- Lint checks structure — dependencies, ownership, requirement coverage, budgets, references, versions — not whether prose is truly decision-complete. That judgment stays with the author.
- Order between SDDs exists only where a unit's `depends_on` names a pin. A plain `vitrify run` builds every Active SDD of the repository together, and SDDs that read each other's contracts as interfaces build side by side.
- A change to the document format raises `schema_version`, and this vitrify reads `2.0` and `2.1`. See [Versioning](../../reference/formats.md#10-versioning).

## Practical Guidance

- Start from the templates — `vitrify new tdd <name>` and `vitrify new sdd <name>` — and keep `vitrify schema show` open while you write.
- Use the calc example in [`fixtures/example/`](../../../fixtures/example/README.md) as the worked reference for every region.
- Choose the bundle shape unless the SDD is small and will be built one unit at a time.
- State the test posture in Ground Rules, where every unit's reading set includes it.
- Prefer a `Q-###` to a guess at every level: author, worker, and integrator.
- Keep TDD section numbers stable across revisions; SDD text cites them permanently.
