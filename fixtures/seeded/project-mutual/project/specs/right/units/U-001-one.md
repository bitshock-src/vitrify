---
unit: U-001
title: U-001 ships right/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [right/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `right/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `right/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f right/one.txt && grep -qx 'right one' right/one.txt` | exit 0 |
