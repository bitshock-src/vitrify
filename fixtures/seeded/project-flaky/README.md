# project-flaky

`flaky` U-001 ships `flaky/check.sh`, whose Verification row counts its own
invocations in `$FLAKY_DIR/count` (a directory the test sets, outside any
checkout) and fails until it has run more than `$FLAKY_FAILS` times (default
1). `flaky` U-002 depends on U-001. `side` is an independent one-unit SDD.

Expected with `FLAKY_DIR` set and the default: the first gate of U-001 fails,
the retry passes; `side` is unaffected. `FLAKY_FAILS=0` never fails.
