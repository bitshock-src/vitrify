---
unit: U-001
title: the unit
status: Verified
claimed_by: —
claim_expires: —
claim_seq: 1
depends_on: []
parallel_group: —
owns: [src/a.txt]
consumes: []
built: —
verified: —
trace:
  unit:   8d6b71249724
  global: 8599aed909f6
---

# U-001 — the unit

## Requirements

- **U-001.R1** — THE SYSTEM SHALL exist.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `exit 1` | always fails |
