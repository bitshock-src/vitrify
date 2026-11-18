# Technical Design Document (TDD): calc — exact rational expression evaluator

> Structure per [`tdd-template.md`](../../../crates/vitrify-core/assets/templates/tdd-template.md); consumed by the SDD at
> [`../specs/calc/`](../specs/calc/) per [`tdd-schema.md`](../../../crates/vitrify-core/assets/schemas/tdd-schema.md). Section numbers
> §1–§6 are permanent anchors.

## Document Metadata

| Document Metadata | Details |
| --- | --- |
| **Author(s)** | Example Engineering |
| **Revision** | 1.0 |
| **Status** | Approved |
| **Created / Updated** | 2026-10-28 / 2026-10-28 |
| **Reviewers** | Example Engineering (self-contained demonstration project) |
| **Tracking** | — |

## 1. Context & Scope

### 1.1 Problem Statement

Ad-hoc arithmetic in scripts and shell pipelines is routinely done with floating-point tools, which
round (`0.1 + 0.2` ≠ `0.3`) and report malformed input inconsistently, making results untrustworthy
and errors un-scriptable. There is no dependency-free command that evaluates integer arithmetic
expressions exactly and rejects bad input with a stable, machine-readable error.

### 1.2 Goals (In Scope)

- Every grammatical expression evaluates to an **exact rational** result; floating-point arithmetic
  is never used, so no result ever carries rounding error.
- Every malformed input is rejected with a **stable error code** (a closed four-code registry, §3.1)
  and a **0-based input position** wherever a position is meaningful.
- **Zero third-party dependencies**: the implementation and its verification run on a stock CPython
  installation with one command each.
- A **scriptable CLI**: results on stdout only, diagnostics on stderr only, disjoint exit codes
  (0 success, 2 usage, 3 expression error).

### 1.3 Non-Goals (Out of Scope)

- Decimal or scientific literals (accepted debt, §6.2) — integer literals only in this revision.
- Variables, functions, or an interactive REPL.
- Performance or latency targets: correctness only; no SLA exists for this tool.
- Localization: all output is plain ASCII English.
- Floating-point output modes of any kind.

## 2. High-Level Architecture

### 2.1 System Diagram

```
  argv ──> [CLI entry point] ──(text)──> [Tokenizer] ──(token stream)──> [Evaluator]
               │                             │                               │
               │<────────(Fraction result)───┼───────────────────────────────┘
               │                             │
               └──────── raises ──> [Shared error type] <── raises ──────────┘

  [Acceptance harness] ── exercises ──> every public surface above (CLI, tokenizer,
                                        evaluator, error type)
```

Every named edge is one interface contract: the shared error surface, the token stream, the
evaluation function, and the CLI surface (consumed by the acceptance harness).

### 2.2 System Components

- **Shared error type:** the single exception crossing all component boundaries, carrying a code
  from a closed registry, a human message, and an optional position. CPython ≥ 3.10, standard
  library only.
- **Tokenizer:** pure function from expression text to a token list; single forward scan; no state.
- **Evaluator:** pure function from a token sequence to an exact rational value
  (`fractions.Fraction`); recursive descent over the §3.3 grammar; consumes tokens by *shape*, not
  by class (§3.1), so it has no import-time dependency on the tokenizer.
- **CLI entry point:** argument handling, the tokenize→evaluate pipeline, canonical result
  printing, error formatting, exit codes. Runnable as a module (`python3 -m calc`).
- **Acceptance harness:** the verification suite, authored together with this design **before any
  component is implemented** (§3.4). It exercises only the public surfaces above and is the
  conformance evidence for all of them.

All components: CPython ≥ 3.10 (< 4), standard library only, no build step beyond byte-compilation
sanity (stdlib `compileall`). Tests run under stdlib `unittest`.

## 3. Detailed Technical Design

### 3.1 Data Model & Schema

**Error model.** One exception type, `CalcError(code, message, pos)`:

| Field | Type | Meaning |
| --- | --- | --- |
| `code` | str | one of the closed registry below; any other value is a programming error (reject with `ValueError` at construction) |
| `message` | str | human-readable prose; **non-normative** — only `code` and `pos` carry contractual meaning |
| `pos` | int or none | 0-based position in the input; absent (`None`) where no position is meaningful |

`str(error)` yields exactly `message`. The code registry is closed:

| Code | Raised by | `pos` meaning |
| --- | --- | --- |
| `TOKEN_INVALID` | tokenizer | index of the offending character |
| `EXPR_EMPTY` | evaluator | none — there is no position in empty input |
| `EXPR_SYNTAX` | evaluator | the offending token's position; none when caused by end of input |
| `DIV_ZERO` | evaluator | the position of the offending `/` token |

**Token model.** A token is a record of three attributes:

| Attribute | Type | Meaning |
| --- | --- | --- |
| `kind` | str | `"NUM"`, `"OP"`, `"LPAREN"`, or `"RPAREN"` |
| `value` | int or str | the base-10 integer for `NUM`; the literal character (`+ - * / ( )`) otherwise |
| `pos` | int | 0-based index of the lexeme's first character |

**Structural token typing (design decision).** Token consumers depend only on the attribute shape
(`kind`, `value`, `pos`), never on the producing class's identity. The tokenizer defines the
concrete token class; the evaluator and the harness accept any object exposing the shape. This
removes every import-time edge between evaluator and tokenizer, so the two can be built and
verified independently and in parallel.

### 3.2 API Specifications & Data Contracts

**Tokenizer.** `tokenize(text: str) -> list[Token]` — lexes per §3.3; raises `CalcError`
(`TOKEN_INVALID`) on any unsupported character; returns `[]` for empty or blank input.

**Evaluator.** `evaluate(tokens) -> fractions.Fraction` — accepts any sequence of §3.1-shaped
tokens; returns the exact rational value; raises `CalcError` (`EXPR_EMPTY`, `EXPR_SYNTAX`,
`DIV_ZERO`) per §3.1 and §3.3.

**CLI surface.** Invocation: `python3 -m calc "EXPR"` — exactly one positional argument, taken
verbatim (it may begin with `-`; no flag parsing exists, §6.1).

| Condition | stdout | stderr | exit |
| --- | --- | --- | --- |
| expression evaluates | canonical result + newline | empty | 0 |
| argument count ≠ 1 | empty | `usage: python3 -m calc "EXPR"` + newline | 2 |
| `CalcError` raised | empty | `error[<code>]: <message>` — plus ` (at position <pos>)` when a position is present — + newline | 3 |

**Canonical result formatting.** Denominator 1 → the base-10 integer, with a leading `-` when
negative. Otherwise `<numerator>/<denominator>` in lowest terms with a positive denominator and
the sign on the numerator (e.g. `17/2`, `-7/2`).

### 3.3 Algorithms & Key Flows

**Grammar** (binary operators left-associative; `*` `/` bind tighter than `+` `-`):

```
expr   ::= term (("+" | "-") term)*
term   ::= factor (("*" | "/") factor)*
factor ::= "-" factor | NUM | "(" expr ")"
```

**Tokenizing:** one forward scan. Space (U+0020) and tab (U+0009) separate lexemes and emit
nothing. A digit run `[0-9]+` is one `NUM` token (its base-10 value; position of the first digit).
Each of `+ - * / ( )` is one token. Any other character aborts the scan immediately with
`TOKEN_INVALID` at its index.

**Evaluating:** recursive descent, one function per production, threading a token index. Empty
input → `EXPR_EMPTY`. A token that fits no production → `EXPR_SYNTAX` at that token's position; a
production cut short by end of input → `EXPR_SYNTAX` with no position; a token remaining after a
complete `expr` is itself an offending token. Division checks the right operand first and raises
`DIV_ZERO` at the `/` token's position. All arithmetic is `fractions.Fraction`; unary `-` is exact
negation and nests (`--3` = 3).

**Worked examples:** `2+3*4` → `14` · `8-3-2` → `3` (left association) · `1/3` → `1/3` (exact) ·
`-(7/2)` → `-7/2` · `(1+2)*3-4/8` → `17/2`.

### 3.4 Verification Harness Design

The acceptance suite is **authored with this design, before any implementation exists**, and is the
sole conformance evidence for every component:

- One acceptance module per public surface (error type, tokenizer, evaluator, CLI), plus a scaffold
  self-check module and a mechanical style gate (§4.4). Every test names the requirement it covers.
- **Lenient vs strict mode.** By default, a component module that does not exist yet reports its
  acceptance tests as *skipped* (so the whole suite runs cleanly in the pre-implementation state).
  With the environment variable `SDD_REQUIRE_IMPL=1`, an absent component is a *failure* — this is
  the mode every per-component verification gate runs in, so nothing can pass by being absent. A
  present-but-broken module is always a failure in both modes.
- Evaluator tests build tokens by shape (§3.1), so they run without the tokenizer. CLI tests run
  the real interpreter as a subprocess from the repository root.
- The harness is **immutable with respect to implementation work**: a change set that implements a
  component must not modify the harness, and integration verifies mechanically that change sets
  touch only the paths their component owns. A harness change is a specification change.

## 4. Cross-Cutting Concerns

### 4.1 Security & Compliance

Input text is **never executed or evaluated as code** (§6.1). The tool performs no file, network,
or process I/O beyond its own stdio streams. There is no authentication surface.

### 4.2 Observability & Telemetry

None beyond the §3.2 stderr error line. No logging, metrics, or telemetry of any kind.

### 4.3 Scalability & Performance SLAs

None — correctness only (§1.3). No gate checks timing.

### 4.4 Code Health

Mechanical style floor over all project Python files, enforced by the harness's style gate: no tab
characters, no trailing whitespace, no line longer than 100 columns.

## 5. Execution & Rollout Plan

### 5.1 Migration Strategy

Greenfield; no data migration. Build-order constraints: the scaffold and acceptance harness exist
**before** all component work (§3.4); the shared error type precedes its consumers; tokenizer and
evaluator are mutually independent (§3.1) and may proceed in parallel; the CLI integrates last.

### 5.2 Release & Deployment Sequence

1. Baseline: scaffold + acceptance harness (ships with the specification).
2. Shared error type.
3. Tokenizer and evaluator, in parallel.
4. CLI entry point — the end-to-end slice; the full suite must pass in strict mode.

### 5.3 Failure Modes & Rollback Strategy

No deployment exists; rollback is version-control revert. A failing integration is reverted or
fixed in place — the mainline of record is never left failing. No build-time rollback artifacts
(flags, migrations) are required.

## 6. Alternatives Considered & Technical Debt

### 6.1 Rejected Alternatives

- **Floating-point arithmetic (binary or decimal):** rejected — rounding violates the exactness
  goal (§1.2); `0.1 + 0.2` must never be anything but exact.
- **Evaluating input with the host language's `eval`/AST machinery:** rejected — executes attacker
  -controlled text (§4.1) and cannot yield stable error codes and positions (§1.2).
- **Third-party parsing or CLI libraries:** rejected — violates the zero-dependency goal (§1.2).
- **Flag-style argument parsing (e.g. an argparse-like layer):** rejected — flag syntax conflicts
  with expressions that begin with `-`; the surface is exactly one verbatim positional argument
  (§3.2).

### 6.2 Accepted Technical Debt

- **Decimal literals** (`1.5`): deferred. A future revision extends the `NUM` lexeme and maps it to
  an exact `Fraction`; arrives as new requirements on the tokenizer under a new TDD revision.
- **Exponentiation operator:** deferred; would extend the grammar with a right-associative level.
