---
sdd: dirty
title: a gate command leaves an ignored file behind
schema_version: 2.1
version: 1.0.0
status: Active
scope_roots:
  - dirty/
---

# dirty

## Ground Rules

One implementer per unit; write only owned paths.

## Technology Decisions

Plain files; POSIX sh.

### Commands

- Bootstrap: `true`
- Build:     `true`
- Test:      `true`
- Lint:      `true`

## System Layout & Ownership

unmanaged: []
