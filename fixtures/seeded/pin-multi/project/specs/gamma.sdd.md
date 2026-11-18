---
sdd: gamma
title: a single-file consumer that pins
schema_version: 2.0
version: 1.0.0
status: Active
scope_roots:
  - g/
---

# gamma

## Ground Rules

One implementer per unit; write only owned paths.

## Technology Decisions

Plain files; POSIX sh.

### Commands

- Bootstrap: `true`
- Build:     `true`
- Test:      `true`
- Lint:      `true`

## System Layout & Ownership

unmanaged: []

## Interface Contracts

### C-001 — Greeting file (pinned)

- Producer: external — alpha, C-001
- Consumers: U-001
- Stability: Frozen by its producer
- Pin: alpha C-001 @ ef9dc34b4daf


## Work Units

### U-001 — ships `g/one.txt`

```yaml
unit: U-001
title: U-001 ships g/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [g/one.txt]
consumes: [C-001]
built: —
verified: —
trace: —
```

#### Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `g/one.txt` with the frozen content.

#### Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f g/one.txt` | exit 0 |
