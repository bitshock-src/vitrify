---
unit: U-002
title: U-002 ships a/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [a/two.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-002 — ships `a/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `a/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f a/two.txt` | exit 0 |
