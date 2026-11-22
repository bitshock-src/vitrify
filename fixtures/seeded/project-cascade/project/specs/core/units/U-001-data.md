---
unit: U-001
title: U-001 ships core/data.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [core/data.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `core/data.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `core/data.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f core/data.txt && grep -q '^v' core/data.txt` | exit 0 |
