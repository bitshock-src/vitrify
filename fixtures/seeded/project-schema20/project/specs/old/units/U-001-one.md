---
unit: U-001
title: U-001 ships old/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [old/one.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-001 — ships `old/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `old/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f old/one.txt && grep -qx 'old one' old/one.txt` | exit 0 |
| R1 | `grep -qx 'base one' base/one.txt` | exit 0 |
