---
unit: U-001
title: U-001 ships side/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [side/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `side/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `side/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f side/one.txt && grep -qx 'side one' side/one.txt` | exit 0 |
