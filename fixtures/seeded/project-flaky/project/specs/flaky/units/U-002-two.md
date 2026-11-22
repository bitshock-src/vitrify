---
unit: U-002
title: U-002 ships flaky/two.txt
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns: [flaky/two.txt]
consumes: []
built: —
verified: —
trace: —
---

# U-002 — ships `flaky/two.txt`

## Requirements

- **U-002.R1** — THE SYSTEM SHALL ship `flaky/two.txt` with the frozen content.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `test -f flaky/two.txt && grep -qx 'flaky two' flaky/two.txt` | exit 0 |
