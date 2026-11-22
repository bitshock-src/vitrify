---
unit: U-001
title: U-001 ships flaky/one.txt, flaky/check.sh
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns: [flaky/one.txt, flaky/check.sh]
consumes: []
built: —
verified: —
trace: —
---

# U-001 — ships `flaky/one.txt, flaky/check.sh`

## Requirements

- **U-001.R1** — THE SYSTEM SHALL ship `flaky/one.txt, flaky/check.sh` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f flaky/one.txt && grep -qx 'flaky one' flaky/one.txt` | exit 0 |
| R1 | `sh flaky/check.sh` | exit 0 |
