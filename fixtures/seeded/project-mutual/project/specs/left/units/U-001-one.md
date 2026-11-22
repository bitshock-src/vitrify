---
unit: U-001
title: U-001 ships left/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [left/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `left/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `left/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f left/one.txt && grep -qx 'left one' left/one.txt` | exit 0 |
