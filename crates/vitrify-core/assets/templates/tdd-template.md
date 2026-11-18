# Technical Design Document (TDD): [Project / Feature Name]

> Template for a TDD instance. Structure only — the TDD→SDD handoff rules live in the TDD schema (`vitrify schema show --tdd`). Section numbers §1–§6 are permanent anchors: SDD text cites them, so never renumber across revisions.

## Document Metadata

> ⇄ SDD: pinned by the [SDD front matter](sdd.md) (`tdd`, `tdd_version`); re-pins recorded in the [Changelog](sdd.md#changelog)

| Document Metadata | Details |
| --- | --- |
| **Author(s)** | [Name / Team] |
| **Revision** | [rev] |
| **Status** | [Draft \| In Review \| Approved \| Deprecated] |
| **Created / Updated** | [YYYY-MM-DD] / [YYYY-MM-DD] |
| **Reviewers** | [Name(s)] |
| **Tracking** | [link] |

## 1. Context & Scope

> ⇄ SDD: consumed by the [Overview](sdd.md#overview); measurable goals become pass conditions in [Verification Gates](sdd.md#verification-gates)

### 1.1 Problem Statement

[What is broken or missing, and why it matters.]

### 1.2 Goals (In Scope)

- [Goal — measurable where possible.]

### 1.3 Non-Goals (Out of Scope)

- [Explicit exclusion.]

## 2. High-Level Architecture

> ⇄ SDD: consumed by [System Layout & Ownership](sdd.md#system-layout--ownership) and the contract skeleton of [Interface Contracts](sdd.md#interface-contracts)

### 2.1 System Diagram

```
[component diagram — every component and every cross-component edge named]
```

### 2.2 System Components

- **[Component]:** [responsibility, hosting, key frameworks.]

## 3. Detailed Technical Design

> ⇄ SDD: consumed by the contract bodies of [Interface Contracts](sdd.md#interface-contracts) and the Implementation blocks of [Work Units](sdd.md#work-units)

### 3.1 Data Model & Schema

[Entities, relationships, exact schemas.]

### 3.2 API Specifications & Data Contracts

[Endpoints, payloads, error codes — exact.]

### 3.3 Algorithms & Key Flows

[Design-level algorithm descriptions and edge cases.]

## 4. Cross-Cutting Concerns

> ⇄ SDD: consumed by [Ground Rules](sdd.md#ground-rules), [Technology Decisions](sdd.md#technology-decisions), and the thresholds of [Verification Gates](sdd.md#verification-gates)

### 4.1 Security & Compliance

[Authn/authz model, data protection standards.]

### 4.2 Observability & Telemetry

[Logging shape, metrics, alerting.]

### 4.3 Scalability & Performance SLAs

[Load figures and latency targets — as numbers.]

## 5. Execution & Rollout Plan

> ⇄ SDD: consumed by the [Execution Order](sdd.md#execution-order) (phases and dependency edges)

### 5.1 Migration Strategy

[Ordering constraints: what must exist before what.]

### 5.2 Release & Deployment Sequence

[Phased rollout sequence.]

### 5.3 Failure Modes & Rollback Strategy

[Triggers and rollback plan; build-time artifacts it requires.]

## 6. Alternatives Considered & Technical Debt

> ⇄ SDD: closed doors for [Decisions & Open Questions](sdd.md#decisions--open-questions) and unit prohibitions in [Work Units](sdd.md#work-units); deferred items arrive later via the [Changelog](sdd.md#changelog)

### 6.1 Rejected Alternatives

- **[Alternative]:** [description]. *Rejected:* [reason].

### 6.2 Accepted Technical Debt

- **[Item]:** [what is deferred and until when.]
