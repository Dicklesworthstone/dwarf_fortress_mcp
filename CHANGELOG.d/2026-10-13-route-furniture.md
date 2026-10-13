# Obstacle-aware furniture assignment

Add `route_furniture_allocation.allocate_routes`: preserve the complete original
request while selecting distinct items by minimum total observed route length,
not geometric proximity alone. Reuse the existing exact matching solver, return
complete Hall witnesses on model shortages, retain unknown target semantics and
hash complete selected routes. Work, input and output remain bounded.

Sixteen actual Python tests pass, including 512 obstacle masks, 120 independent
unpruned allocation comparisons, 65,536 candidates and maximum terrain. No native
or production qualification. See `docs/ROUTE_FURNITURE_ALLOCATION.md`.
