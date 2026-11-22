# project-pipeline

Two SDDs, `core` and `api`, three units each, chained U-001 → U-002 → U-003
inside each. core C-001 (package surface) is produced by core U-001, C-002
(parser surface) by core U-002; api pins both. api U-001 consumes and depends
on the C-001 pin; api U-002 depends on U-001 and the C-002 pin; api U-003 on
U-002. Priorities: core U-001 4; core U-002 and api U-001 3; api U-002 2;
core U-003 and api U-003 1. Critical path (ties by slug, then unit): core
U-001 → api U-001 → api U-002 → api U-003, four units.

Run with `STUB_REF=solution`, `STUB_MAP=stub-map-{sdd}.toml`. Expected: a
project run completes all six units; api U-001 is not claimed before core U-001
is built, api U-002 not before core U-002 is built.
