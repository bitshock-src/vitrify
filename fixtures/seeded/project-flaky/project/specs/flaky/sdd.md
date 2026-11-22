---
sdd: flaky
title: a unit whose check fails until its counter is high enough
schema_version: 2.1
version: 1.0.0
status: Active
scope_roots:
  - flaky/
---

# flaky

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
