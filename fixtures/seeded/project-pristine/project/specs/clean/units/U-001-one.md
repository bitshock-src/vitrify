---
unit: U-001
title: U-001 ships clean/one.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [clean/one.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `clean/one.txt`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `clean/one.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f clean/one.txt && grep -qx 'clean one' clean/one.txt` | exit 0 |
| R1 | `test ! -e dirty/gate.log && test ! -e clean/gate.log` | exit 0 |
