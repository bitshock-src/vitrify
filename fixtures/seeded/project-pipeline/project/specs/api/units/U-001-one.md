---
unit: U-001
title: U-001 ships api/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [C-001]
parallel_group: —
owns: [api/one.txt]
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-001 — ships `api/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `api/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f api/one.txt && grep -qx 'api one' api/one.txt` | exit 0 |
| R1 | `grep -qx 'core package' core/pkg.txt` | exit 0 |
