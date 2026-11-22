---
unit: U-001
title: U-001 ships delta/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [C-002]
parallel_group: —
owns: [delta/one.txt]
consumes: [C-002]
built: —
verified: —
trace: —
---

# U-001 — ships `delta/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `delta/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f delta/one.txt && grep -qx 'delta one' delta/one.txt` | exit 0 |
