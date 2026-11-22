---
unit: U-002
title: U-002 ships api/more.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [api/more.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-002 — ships `api/more.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `api/more.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f api/more.txt && grep -qx 'api more' api/more.txt` | exit 0 |
