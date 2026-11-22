# project-schema20

`base` (schema 2.1) produces C-001; `old` (schema 2.0) pins it and its U-001
consumes the pin.

Expected: a plain `vitrify run` refuses with exit 4, naming `old`;
`vitrify run --sdd specs/old` still works and completes.
