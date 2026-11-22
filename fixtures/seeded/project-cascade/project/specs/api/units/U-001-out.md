---
unit: U-001
title: U-001 ships api/out.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [C-001]
parallel_group: —
owns: [api/out.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-001 — ships `api/out.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `api/out.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f api/out.txt && grep -qx 'api out' api/out.txt` | exit 0 |
| R1 | `grep -qx 'v1' core/data.txt` | exit 0 |
