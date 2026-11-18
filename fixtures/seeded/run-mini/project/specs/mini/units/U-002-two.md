---
unit: U-002
title: U-002 ships src/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: a
owns: [src/two.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-002 — ships `src/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `src/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f src/two.txt` | exit 0 |
