# Golden corpus — pin-multi

The regression goldens for pins: a contract that stands for another SDD's
contract through a `Pin:` line rather than a body of its own. Pinned
outputs over the pristine `fixtures/seeded/pin-multi/project/`: `alpha`, a
bundle that produces two contracts; `beta`, a bundle that pins both — one
pin current, one stale; and `gamma`, a single-file SDD that pins the first.

Paths are normalized to the literal token `{ROOT}` where a verb prints
absolute ones.

| File | What it pins (cwd = a pristine copy of the project) |
| --- | --- |
| `pin-list.json` | `vitrify pin list --all --json` (exit 2: one stale pin) |
| `readset-beta-U-001.json` | `vitrify readset --unit U-001 --sdd specs/beta --json` — the pinned contract's own entry, then its authority's body |
| `readset-gamma-U-001.json` | `vitrify readset --unit U-001 --sdd specs/gamma.sdd.md --json` — the same, from a single-file SDD |
| `trace-beta-U-001.json` | `vitrify trace record --unit U-001 --stamp built --sdd specs/beta` — `C-001` is the hash of beta's own contract file, pin line included |

The finding set `vitrify lint --sdd specs/beta` must report is
`fixtures/seeded/pin-multi/expected.json`.

The fixture and this corpus move in lockstep: the pin hashes in the
fixture are computed over `alpha`'s contract bodies, and the trace hash
over `beta`'s contract file.
