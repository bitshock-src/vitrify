---
unit: U-002
title: U-002 ships core/parser.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [core/parser.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-002 — ships `core/parser.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `core/parser.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f core/parser.txt && grep -qx 'core parser' core/parser.txt` | exit 0 |
| R1 | `grep -qx 'core package' core/pkg.txt` | exit 0 |
