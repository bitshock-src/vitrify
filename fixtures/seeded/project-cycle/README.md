# project-cycle

Four SDDs. alpha U-001 depends on the pin of beta C-001 (built by beta U-001),
and beta U-001 on the pin of alpha C-001: a unit-level cycle through pins.
gamma and delta form a second, independent one. alpha U-002 depends on alpha
U-001, so it only waits behind a cycle.

Expected: a project run is refused before anything is claimed, every cycle
reported (alpha U-001 and beta U-001; gamma U-001 and delta U-001); alpha
U-002 is the unit that only waits behind one.
