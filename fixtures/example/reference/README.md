# reference/ — the author's reference implementation

These four files are **not** part of the SDD's governed tree (`scope_roots` is `calc/` and
`tests/` only). They exist for exactly one purpose: the tier-2 end-to-end battery
(`crates/vitrify-cli/tests/e2e.rs`) feeds them to the deterministic stub worker as a canned,
known-correct implementer, so the full generation protocol can be exercised mechanically,
without a model, on any machine.

A real generation pass must produce its own `calc/errors.py`, `calc/tokenizer.py`,
`calc/evaluator.py`, and `calc/__main__.py` from each unit's reading set. Nothing in the SDD
references this directory, and no implementer's reading set includes it.
