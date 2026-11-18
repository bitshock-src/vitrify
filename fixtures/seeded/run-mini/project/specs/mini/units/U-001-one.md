---
unit: U-001
title: U-001 ships src/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: a
owns: [src/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `src/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `src/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f src/one.txt` | exit 0 |
