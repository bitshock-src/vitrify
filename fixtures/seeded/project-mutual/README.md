# project-mutual

Two SDDs, `left` and `right`, each producing C-001 (U-001) and pinning the
other's. Each U-002 depends on its U-001 and on the pin of the other SDD's
C-001. The pins point both ways, the unit graph is acyclic.

Expected: the run completes all four units; both U-001 build first.
