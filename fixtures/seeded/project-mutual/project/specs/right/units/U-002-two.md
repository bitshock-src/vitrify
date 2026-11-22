---
unit: U-002
title: U-002 ships right/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, C-002]
parallel_group: —
owns: [right/two.txt]
consumes: [C-002]
built: —
verified: —
trace: —
---

# U-002 — ships `right/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `right/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f right/two.txt && grep -qx 'right two' right/two.txt` | exit 0 |
| R1 | `grep -qx 'left one' left/one.txt` | exit 0 |
