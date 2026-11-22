# Contracts

> Understand contracts — the frozen surfaces units share — and the external contracts through which an SDD consumes a surface it does not own.

Use this page to learn how units depend on each other's code without reading it. A **contract** (`C-###`) freezes one cross-unit surface — an API, a data shape, an error vocabulary — so that its producer and its consumers can be built in parallel from the contract alone. An **external contract** is one SDD's stand-in for a surface it does not own: a pin of a contract another SDD owns, or a transcription of a source no SDD owns. For the procedure of writing and syncing external contracts, see [Work With Several SDDs](../../user-guides/multiple-sdds.md).

## Why Contracts Exist

Unwritten cross-unit assumptions are the canonical multi-writer bug: two workers building in parallel each make a reasonable decision about a shared surface, and the decisions conflict. Contracts remove the choice. Every cross-unit surface is frozen in exactly one contract before its consumers go Ready, and consumers code against the contract — never against the producer's design or code. In the TDD, every cross-component edge becomes exactly one contract.

## What a Contract Represents

A contract has a header and a body. This is the start of calc's token-stream contract:

```markdown
# C-002 — Token stream

- Producer: U-003
- Consumers: U-001, U-004, U-005
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface

    Module: calc/tokenizer.py  (import as calc.tokenizer)
    …
```

- **Header** — the bullet lines between the title and the first section heading: `Producer:` (the unit that implements the surface), `Consumers:` (the units that use it), `Stability:`, and, on a pin, `Pin:`.
- **Body** — everything from the `Surface` heading to the end: the surface itself and its semantics, written as exact shapes, names, and codes.

## Producers and Consumers

A unit lists the contracts it reads in its `consumes` front matter. Each contract's `Consumers:` line must name exactly those units — its producer apart, which may consume its own contract (lint L4). A consumed contract must exist, and a contract may name only units that exist (audit check 3).

## How Contracts Stale Their Consumers

Every consumed contract is part of its consumer's reading set and so of its trace. Editing a frozen contract therefore moves the trace of every stamped consumer, and `vitrify audit` names each with the cause; the change entry, recorded with `vitrify change apply`, marks them Stale — the reading set and the staleness set are the same list by construction. In calc, appending one line to C-002 stales U-001, U-003, U-004, and U-005 once they are built, and leaves U-002 alone because it does not consume C-002. See [Change a Spec](../../user-guides/change-a-spec.md).

## Tests and Contracts

When a test owned by one unit imports a module another unit owns, the test's unit must consume a contract naming that module — which also stales the tests when the contract changes. vitrify never reads code, so nothing in vitrify checks this; pin a boundary checker for your language and run it as the Lint command or as a Verification row.

The calc example shows the pattern. Its evaluator tests build tokens by shape, following C-002's structural-typing rule, and never import the tokenizer — which is what lets the tokenizer and evaluator units build and gate in parallel.

## External Contracts

Everything vitrify does is scoped to one SDD. A unit's `consumes` names contracts of its own SDD, its reading set and its trace are built from that SDD's files, and unit IDs are per SDD — every one has a `U-001`. So a unit that codes against a surface another SDD owns consumes a contract of **its own** SDD that stands for the other's: an *external contract*, one whose `Producer:` line begins `external`. A repository with several coupled SDDs has such stand-ins by construction — one fifteen-bundle build had 339 of them — and a stand-in nobody checks is exactly the unchecked copy of a living document that vitrify's first [design position](../design-rationale.md#design-positions) rules out. So vitrify owns the check. An external contract takes one of two forms.

| Form | Stands for | Holds | When the source changes |
| --- | --- | --- | --- |
| **Pin** | a contract another SDD owns (its *authority*) | its own header with `Pin: <sdd-slug> C-### @ <hash12>` — the hash of the authority's body — and no body | `vitrify pin sync` rewrites the hash |
| **Transcription** | a source no SDD owns — a vendor API, a file format, another team's service | its own header, whose `Producer:` names no contract, and the consumer's own transcription as its body | the author edits the body; nothing can check it |

A worker whose unit consumes a pin reads the pin's own entry and then the authority's body, which its reading set follows the pin to. vitrify hands that body out only against a matching hash, so a unit is never built from text its trace does not cover. The exact rules are in the [Data Model and Formats Specification](../../reference/formats.md#8-external-contracts).

`vitrify pin list` shows every external contract with its kind (`pin` or `transcription`) and its state. Use this table to read the states.

| State | Meaning | Fix |
| --- | --- | --- |
| `ok` | the pin's hash matches its authority's body | — |
| `stale` | the authority's body changed since the pin was written | `vitrify pin sync --sdd <root>` |
| `missing-authority` | the SDD or contract the pin names does not exist | correct the `Pin:` line, or the producer |
| `invalid` | a pin that still carries a body; a contract whose `Producer:` names another SDD's contract but has no `Pin:` line; an authority with no `Surface` heading; a slug two SDDs share | as the cause says — for a missing `Pin:` line, write it and let `vitrify pin sync` fix the hash |
| `unchecked` | a transcription; there is no authority to compare | — |

### Pins Carry Order Too

A pin is the one handle between SDDs, and it does two jobs. Listed in a unit's `consumes`, it puts the authority's body in the unit's reading set, so a change reaches the unit as staleness; that orders nothing. Listed in the unit's `depends_on` as well (schema 2.1), it also makes the unit wait until the units that produce the authority are built, and makes the unit re-checked when their code changes. Pick the pin's role by what the unit needs:

| The unit needs | Where the pin goes | Effect |
| --- | --- | --- |
| the **code** behind the surface: it imports or links it, installs its package, or runs its CLI in its checks | `consumes` and `depends_on` | the unit waits for the producers, and its gates run again when their code changes |
| only the **interface** or **data format**: it codes against the shape, or parses the data | `consumes` only | nothing is ordered; the unit builds while the producers build |

Pins routinely run both ways between SDDs that read each other's data, which is why consuming alone orders nothing. A cycle through `depends_on` is refused; see [Work Units](units.md#one-graph-across-sdds).

### Package Boundaries Are Contracts

An SDD whose code other SDDs install or import publishes a contract for its package surface, and its foundation unit produces it. A consuming SDD's foundation unit lists that pin in `depends_on`. Because every unit depends on its SDD's foundation, a change to the foundation's files — a lockfile, build configuration, scaffolding — re-checks every unit of that SDD and every dependent in other SDDs.

### What a Pin Saves

A pin saves the copied text and the diff that would repeat in every consumer on every producer edit. It saves no reading: the authority's body counts against the consumer's reading-set budget. And it removes no change entry: a producer's change still reaches each consumer as that consumer's own entry. Adding a pin to `depends_on` to record a need the unit's built code already has is an editorial (PATCH) change.

### Checked by Lint, So by Preflight

L9 fails on a pin that no longer matches its authority, and `vitrify run` preflights with lint: from the moment a producer's contract changes until a consumer syncs, that consumer does not start a run. That is lockstep, and it is the point — the audit cannot catch it: the consumer's own files have not changed, so its units still match their traces. The sync is what delivers the producer's change to the consumer's staleness. A producer edit that lands while a consumer builds stops the pin's units at the next dispatch, since the authority's body no longer matches the hash. `vitrify check` reports the same states under `[pin]`.

### Sync Writes Contract Text and Nothing Else

`vitrify pin sync` rewrites stale pins' hashes in the working tree and prints, per consumer SDD, the units whose trace it moved — the `Staleness applied` line — because vitrify can compute that and a script outside it cannot. The change entry and the version bump stay the author's (SDD schema §12); the consumer's bookkeeping is then `vitrify change apply --sdd <consumer>`. Transcriptions stay unverifiable, and stay listed as such.

## Limitations

These are the boundaries of what vitrify checks about contracts.

- The audit sees one SDD. A pin that is behind its authority is no audit finding — the SDD's own files are consistent with each other, which is all its traces can say. That check is lint's (L9), and `vitrify check` runs it.
- A transcription cannot be verified. `vitrify pin list` shows it as `unchecked`, never as `ok`, and it is never a lint finding.
- A contract cannot be shared by symlinking a consumer's file to its authority: the authority's `Producer:` and `Consumers:` lines name the producer's units, which are other units in the consumer's SDD. A pin needs a header of its own; only the body is shared.

## Practical Guidance

- Freeze a contract before any of its consumers goes Ready.
- Keep each `Consumers:` line exactly equal to the units that consume the contract.
- Write contract bodies as exact shapes — field names, types, error codes — so that two workers cannot read them two ways.
- Consume a contract another SDD owns through a pin; write a transcription only for a source no SDD owns.
- After any producer change, run `vitrify pin sync --all`, write each consumer's change entry around its printed line, run `vitrify change apply --sdd <consumer>`, gate with `vitrify check --all`, and commit.
