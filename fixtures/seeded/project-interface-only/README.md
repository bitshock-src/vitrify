# project-interface-only

Like project-pipeline, but api U-001 consumes the C-001 pin without listing
it in `depends_on`: an interface-only read, no order. api U-002 still depends
on U-001 and the C-002 pin.

Expected: api U-001 is claimable at once and builds alongside core U-001; the
run completes all six units.
