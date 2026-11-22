# project-pristine

The repository ignores `*.log`. `dirty` U-001's second Verification row
writes `dirty/gate.log` into the checkout it runs in; `clean` U-001's second
row asserts `dirty/gate.log` does not exist.

Expected: both units verify, because every gate runs in a checkout wiped of
untracked and ignored files; `clean` fails if it ever sees the file.
