# C-002 — Farewell file

- Producer: U-002
- Consumers: —
- Stability: Frozen

## Surface

```text
# a/two.txt — the farewell file
alpha two
```

## Semantics

- THE SYSTEM SHALL write exactly one line and nothing after it.
- IF the file exists, THEN THE SYSTEM SHALL replace it whole.
