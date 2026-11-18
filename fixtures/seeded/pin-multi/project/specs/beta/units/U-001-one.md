---
unit: U-001
title: U-001 ships b/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [b/one.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-001 — ships `b/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `b/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f b/one.txt` | exit 0 |
