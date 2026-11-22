---
unit: U-001
title: U-001 ships core/pkg.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [core/pkg.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `core/pkg.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `core/pkg.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f core/pkg.txt && grep -qx 'core package' core/pkg.txt` | exit 0 |
