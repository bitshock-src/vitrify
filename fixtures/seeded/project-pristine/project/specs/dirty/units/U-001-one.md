---
unit: U-001
title: U-001 ships dirty/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [dirty/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `dirty/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `dirty/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f dirty/one.txt && grep -qx 'dirty one' dirty/one.txt` | exit 0 |
| R1 | `echo leftover > dirty/gate.log` | exit 0 |
