# Obstacle-aware furniture assignment and native read workflow

Add `route_furniture_allocation.allocate_routes`: preserve the complete original
request while selecting distinct items by minimum total observed route length,
not geometric proximity alone. Reuse the existing exact matching solver, return
complete Hall witnesses on model shortages, retain unknown target semantics and
hash complete selected routes. Work, input and output remain bounded.

Add `allocate_route_furniture.py`: a single query-only connection brackets one
complete released operations/1.4 inventory with equal paused map/1.5 endpoints.
Close before solving; require exact fortress, region, tick and software; reuse
existing paging/release and full-roster decoding. Shared protobuf decoding gains
an opt-in bounded reply size for maps; old callers retain their page ceiling.
Keep material/z/distance constraints and reject unavailable or related inventory.
Output a complete proposal or explicit shortage/unknown report, never a partial
plan. No placement credential, effect, retry, journal or background work is added.

All 35 actual Python tests pass: 16 core functions and 19 integration functions,
including independent allocation oracles, maximum inputs, real loopback wire
faults and CLI subprocesses. No real DFHack, Rust/MCP or production qualification.
See `docs/ROUTE_FURNITURE_ALLOCATION.md` for invocation and precise evidence limits.
