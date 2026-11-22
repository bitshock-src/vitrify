---
unit: U-003
title: U-003 ships api/three.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-002]
parallel_group: —
owns: [api/three.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-003 — ships `api/three.txt`

## Requirements

- **U-003.R1** — THE SYSTEM SHALL ship `api/three.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f api/three.txt && grep -qx 'api three' api/three.txt` | exit 0 |
