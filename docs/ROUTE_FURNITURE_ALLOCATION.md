# Route-aware whole-plan furniture allocation

Beads: `df-franken-storage-mvcc-54h.4` and `df-dfhack-bridge-plane-c-pic.3/.4`.
This adds an obstacle-aware alternative to the existing Manhattan-distance
furniture allocator. It does not change the legacy allocator or placement path.

`route_furniture_allocation.allocate_routes(request, candidates, terrain_raw)`
validates the complete unchanged map/1.5 capture, preserves all original slot
material, subtype, same-z, exclusion and maximum-Manhattan constraints, then
minimizes total shortest candidate-route steps. Equal totals use the existing
exact lexical slot-name/item-ID tie break. A separate `max_route_steps` limit
narrows route eligibility; it does not reinterpret `Slot.max_distance`.

The movement model matches `observed-dry-cardinal-floor-stairs/1`: visible dry
floor/stair cells with no observed building/unit occupancy and nonzero native
walkability; vertical transitions require complementary stair shapes. Ramps,
diagonals, doors, hidden cells and routes outside the captured region are not
modeled. Item/target endpoints must share a z level, but a modeled route may use
stairs to another level and return. This is NOT native unit pathfinding, danger
assessment, throughput or a forecast of routes after earlier furnishings.

For N targets, a full target-rooted BFS provides shortest distances before
keeping the N best eligible items for that target. That reduction preserves a
global optimum: the other N-1 targets cannot consume every one of those N better
items. The existing maximum-matching and exact integer Hungarian solver choose
the whole assignment. A deficient Hall set has fewer than N neighboring items,
so no member's eligible list was trimmed: the witness covers the full supplied
candidate graph within this bounded route model.

Unobserved/unsupported targets return `target_unestablished`, null maximum
assignment counts and no shortage/plan. Established targets may yield a checked
model shortage; no partial executable subset is emitted. Unknown terrain never
becomes complete-world absence. Successful proposals use the existing exact-item
`dfmcp.furniture-plan/1` format. Material constraints and route evidence are not
automatically enforced by that legacy plan format at future placement time.

A second bounded BFS rechecks each selected route and creates a streaming digest
of every flattened path index. The path policy is target-rooted BFS with stable
flattened-index ties, not necessarily the same tie as a BFS rooted at the item.
The decision digest binds the normalized request, all supplied candidates (even
unselected ones), exact raw map digest, policies, route limit, work budget and
complete result. Source/order changes cannot silently reuse earlier evidence.
Pure callers supply candidate facts; these APIs do not attest their acquisition.

Bounds: 32 slots, 65,536 candidates, 16,384 terrain tiles, 128 cells per region
axis, 20,000,000 shared work units and 49,152 result bytes. Space is
O(tiles + candidates + N*N); no slot-by-inventory or slot-by-terrain matrix is
retained. The same cooperative allowance covers decoding, routing, full candidate
scans, matching, optimization and route witnesses. Refusal returns no partial
result. Final serialization and hashing are bounded but not internally preempted.

## Executed development validation — 2026-10-13

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 -m unittest test_route_furniture_allocation -v
```

All 16 test functions passed. They execute the actual Python implementation,
not a mirror: all 512 three-by-three obstacle masks (unestablished targets remain
unknown), 120 random inventories against independent Floyd-Warshall/unpruned
permutation and cardinality oracles, material starvation, complete Hall witnesses,
legacy constraints, stair detours, unknown targets, source/order invalidation,
exact work boundaries, every cooperative callback refusal, every small-fixture
truncation, maximum candidate/slot counts and an 8,255-tile snake in a maximum-size
map. Both reused upstream Python modules were verified against their pinned Git
blob hashes. No Rust, native DFHack, full repository or production qualification
is implied, and the broader beads remain open.
