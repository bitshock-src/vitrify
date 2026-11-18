# Decisions & Open Questions — calc

## Decision Log

- **D-001** (2026-10-28, SDD 1.0.0) — TDD §3.2 is silent on leading zeros in integer literals.
  Decided: a digit run with leading zeros is one `NUM` token carrying its base-10 value
  (`007` → 7). Affects U-003; restated in C-002.
- **D-002** (2026-10-28, SDD 1.0.0) — The U-001 baseline (scaffold plus acceptance suite) is
  authored together with this SDD, before any repository history exists; its built/verified stamps
  and its journal events therefore record the placeholder commit `0000000`. Every later event
  records a real commit. Affects U-001.

## Open Questions

None.
