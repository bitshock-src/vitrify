---
unit: U-002
title: U-002 ships b/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [b/two.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-002 — ships `b/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `b/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f b/two.txt` | exit 0 |
