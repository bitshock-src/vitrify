---
unit: U-002
title: U-002 ships core/note.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [core/note.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-002 — ships `core/note.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `core/note.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f core/note.txt && grep -qx 'core note' core/note.txt` | exit 0 |
