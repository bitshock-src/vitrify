---
unit: U-003
title: U-003 ships core/cli.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-002]
parallel_group: —
owns: [core/cli.txt]
consumes: [C-002]
built: —
verified: —
trace: —
---

# U-003 — ships `core/cli.txt`

## Requirements

- **U-003.R1** — THE SYSTEM SHALL ship `core/cli.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f core/cli.txt && grep -qx 'core cli' core/cli.txt` | exit 0 |
| R1 | `grep -qx 'core parser' core/parser.txt` | exit 0 |
