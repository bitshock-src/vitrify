# Golden corpus — calc example

The regression goldens for `schema_version: 2.0`: committed outputs of every
mechanical verb over the pristine calc example (`fixtures/example/`). Where
vitrify and the corpus disagree, the disagreement is a bug — unless the change
is to the document format or the trace recipe, which raises `schema_version`
and regenerates the corpus in the same reviewed change.

Paths are normalized to the literal token `{ROOT}` (the scratch root the
corpus was generated under) and the ledger derived-date to `{DATE}`, so the
files are location- and day-independent.

| File | What it pins (cwd = a pristine copy of the example) |
| --- | --- |
| `locate.json` | `vitrify locate --units --json` |
| `readset-U-00N.json` | `vitrify readset --unit U-00N --json` |
| `replay.json` | `vitrify journal replay --json` |
| `audit.json` | `vitrify audit --skip-rotten --json` |
| `progress.txt` | `vitrify ledger progress` |
| `ledger.md` | `vitrify ledger regen` → `specs/calc/ledger.md`, date-normalized |
| `trace-U-00N.json` | `vitrify trace record --unit U-00N --stamp built` (stdout JSON; hash map under `recorded`) |

The corpus and the example move in lockstep: the trace hashes are computed
over `fixtures/example/specs/calc/**` bytes, so neither side changes without
the other.
