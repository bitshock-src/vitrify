# Golden corpus — pin-states

Pinned outputs of `vitrify pin list` over the pristine
`fixtures/seeded/pin-states/project/`: three SDDs in one repository —
`alpha`, a bundle that produces two contracts; `beta`, a bundle with four
external contracts — a current pin, a stale pin, a pin whose authority is
missing, and a transcription; and `gamma`, a single-file SDD whose pin is a
region of its own file.

| File | What it pins (cwd = a pristine copy of the project) |
| --- | --- |
| `pin-list.json` | `vitrify pin list --all --json` (exit 2: one stale pin, one missing authority) |
| `pin-list.txt` | `vitrify pin list --all` |

Paths in both are relative to the repository root, so the files hold no
`{ROOT}` token. The finding set `vitrify lint --sdd specs/beta` must report
is `fixtures/seeded/pin-states/expected.json`.
