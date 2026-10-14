## Recheck the original remaining furniture delivery order

Add a bounded fixed-order suffix checker using the existing captured route
model. Preserve original items/dependencies, conservatively remove authenticated
prefix targets and simulated earlier targets, and emit only complete route
witnesses. No allocation, reordering or native effect is performed.

Fourteen component tests pass, including 600 independent BFS cases; four
weakened implementations fail regressions. Placement integration follows.
