# C-004 — Wall clock (transcription)

- Producer: external — the host's clock, as POSIX `date -u` prints it; no SDD in this repository owns it
- Consumers: U-002
- Stability: Frozen by its producer; Surface and Semantics transcribe POSIX `date`

## Surface

```text
date -u +%Y-%m-%dT%H:%M:%SZ
```

## Semantics

- THE SYSTEM SHALL read the time in UTC.
