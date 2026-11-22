---
unit: U-002
title: U-002 ships left/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, C-002]
parallel_group: —
owns: [left/two.txt]
consumes: [C-002]
built: —
verified: —
trace: —
---

# U-002 — ships `left/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `left/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f left/two.txt && grep -qx 'left two' left/two.txt` | exit 0 |
| R1 | `grep -qx 'right one' right/one.txt` | exit 0 |
