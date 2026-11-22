---
unit: U-002
title: U-002 ships api/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, C-002]
parallel_group: —
owns: [api/two.txt]
consumes: [C-002]
built: —
verified: —
trace: —
---

# U-002 — ships `api/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `api/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f api/two.txt && grep -qx 'api two' api/two.txt` | exit 0 |
| R1 | `grep -qx 'core parser' core/parser.txt` | exit 0 |
