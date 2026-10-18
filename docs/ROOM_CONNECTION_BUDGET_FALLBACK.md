# Complete room corridors under tight tile budgets

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/WP-10; broad work remains open).

The room connector previously used a sequence of cheapest links. That can spend
more tiles than a shared junction. For floors `(1,1,1)`, `(2,2,1)`, `(3,1,1)` in
a 3-by-3 domain, it selects `(1,2,1)` before `(2,1,1)`. The two-tile result is
connected, but a one-tile allowance rejects it even though `(2,1,1)` alone joins
all three original floors.

The existing `room_connections.connect` now falls back only when its greedy
links would exceed the admitted new-tile allowance. It discards its provisional
corridors and solves from the complete original floor and protected masks. It
never keeps an expensive greedy prefix, drops a room, clears a protected wall,
changes an entrance or extends the routing domain.

## Algorithm and limits

The fallback uses vertex-weighted subset dynamic programming. Already intended
floor costs zero; an admitted new corridor tile costs one. Every cardinally
connected original floor component is a terminal, including components without
named entries. The DP joins terminal subsets at a shared vertex, subtracting
that vertex's duplicate cost, then propagates each subset with multi-source
Dijkstra. Values above the admitted tile allowance are never retained.

The recurrence is `join[S,v] = min(dp[A,v] + dp[S-A,v] - weight[v])`, followed by
shortest-path closure paying each destination vertex. An optimal connecting
tree decomposes at a vertex into smaller terminal subtrees; strict subset order
makes those entries available before a merge. Nonnegative costs permit pruning
above the total tile allowance. Reconstruction retains the exact union and
checks its cardinality against the optimal cost. A separate cardinal flood and
exact rectangle expansion validate the returned whole mask.

At most nine original floor components are admitted to the DP: the eight-area
model plus an external access component. The original 1,024-cell capture,
512-total-walkable-cell structural bound, and 100,000 cooperative work checks
remain unchanged. The DP tables are byte-bounded to 8 MiB before allocation;
on the tested four-byte `array('i')` implementation the structural maximum is
less than 4 MiB. Heap and graph work is separately charged to the same owner.
Worst-case time is exponential in component count; the work limit may stop a
large instance well before its structural maximum. No worst-case completion or
hard real-time guarantee is made.

The fallback does not renew the work allowance already consumed by greedy
routing. More than nine components or exhausted work raise the existing
`room_connections.WorkLimit`, which the layout solver propagates as inconclusive
rather than an exhausted no-layout domain. Cancellation also returns no partial
artifact. An exhaustive DP failure means no connecting mask fits this fixed
region and tile allowance, not that no fortress layout exists elsewhere.

## Compatibility and evidence

Every successful greedy route retains its original geometry, work count,
summary, policy and digest. The fallback result has its own policy and digest
domain, `dfmcp.room-project-minimum-connections/1`, and reports
`minimum_excavation_in_region_proven: true` with explicit finite-mask scope.
The tie order is component coordinate, ascending subset mask, ascending split
mask, strict cost improvement and `(cost, tile-coordinate)` heap order.

The existing RoomPlan compiler already consumes the connector's exact parts and
summary, checks its complete floor/room/furniture contract and regenerates saved
plans. No new mutation or MCP operation is added. Older implementations reject
new fallback artifacts when they cannot regenerate the retained original intent.
A previously successful RoomPlan artifact does not silently acquire a new route.
Saved layout searches additionally require the strategy separation described below.

Minimum tile count does not imply minimum rectangle count, current terrain,
native pathfinding, construction completion, mutation authority or production
admission. The original outer 32-part limit still applies. A minimum-tile mask
that fails that separate limit does not prove that no other partition-friendly
layout exists.

## Regression command

```sh
PYTHONPATH=scripts python3 -W error -m unittest discover -s tests \
  -p 'test_room_connection*.py' -v
```

The tests use the actual pure connector and solver, not native sources. They
check all 6,561 three-state 3-by-3 worlds with a fixed root against a separate
exhaustive subset oracle; protected cells, all original components, explicit
entries, every cancellation checkpoint, exact work exhaustion, maximum capture,
process hash seeds and exact rectangle masks. A baseline digest preserves all
4,832 successful and 1,729 refused generous-budget outcomes, including prior
work counts. The original eleven connector tests run unchanged.

## Complete 512-tile room project

`examples/room_connection_budget_fallback.json` is a complete ordinary room
request, not a synthetic call to the graph solver. It contains eight 7-by-7
bedrooms and two one-table dining halls, one rotated 90 degrees. The original
floors, doors and local corridors occupy 499 tiles; the original room compiler
allows only 13 additional corridor tiles before reaching its unchanged 512-tile
limit. The old greedy router refuses the project. The fallback compiles it into
512 exact tiles in 24 disjoint blueprint parts, retaining all eight bedrooms,
both dining halls and all 28 original furniture slots.

The new corridor is a shared junction with 13 cells, independently confirmed by
three-component 0/1 shortest-path and branching-vertex enumeration. The whole
capture is 39 by 24 cells (936, within the existing 1,024 bound). The routing
operation consumes 58,369 of its original 100,000 checks on this input. That is
a deterministic work count for this fixture, not a wall-time performance claim.

The existing room compilation command accepts the example unchanged:

```sh
python3 scripts/plan_rooms.py compile \
  --request-file examples/room_connection_budget_fallback.json --emit plan
```

Ordinary `RoomPlan.compile`/`decode` and actual `furniture_allocation.allocate`
were executed in the component tests; the above CLI/import graph was not run.
Twelve whole-room tests cover exact masks, the independent minimum oracle,
required bedroom walls, all room and furniture approaches, complete 28-slot
allocation, whole-plan shortage, artifact tampering, original exclusions,
request-order independence, cancellation and legacy complete-plan digests.
They import the actual repository modules without source extraction or validator
doubles. The example names synthetic fortress selectors; it is not a live game
observation, placement permit or production-admission artifact.

The full-room tests also preserve the independently added cross-level furniture
selectors from main `8813904edc93c5e48c17c2bdf0e1e183c23efe3c`. A corridor
proof does not permit the same-level allocator to discard a nonzero vertical
selector. The concurrent compiler change is retained, not bundled or overwritten
by this corridor patch.

## Replay-safe automatic layout search

Direct room compilation enables the fallback automatically. Automatic layout
search is different: accepting an earlier candidate can change a previously
successful selected layout, its search trace and its work count. Re-running
old saved searches with new feasibility rules broke canonical decoding during
review. Even a previously saved no-layout report became unreplayable.

The layout request therefore opts into the new search behavior explicitly:

```json
"connection_strategy": "greedy-then-minimum-tiles/1"
```

This top-level field belongs to `dfmcp.room-layout-request/1`, not the room
provisioning request. It is retained in the normalized original request and
sealed under policy `dfmcp.mrv-minimum-fallback-room-layout/1`. Omitting it keeps
the exact original greedy layout search, including failed candidate work and
old successful choices. Unknown values are refused. The choice is passed
explicitly per call; no global mode, recovery-time substitution or budget reset
exists. A selected RoomPlan still regenerates through the ordinary compiler.

`examples/room_connection_fallback_layout.json` is the executable layout version
of the 512-tile example, using the original fixed room positions. Run:

```sh
python3 scripts/layout_rooms.py --request-file examples/room_connection_fallback_layout.json --emit layout > layout.json
python3 scripts/layout_rooms.py --layout-file layout.json --emit room-plan > room-plan.json
```

Review validation runs 65 warnings-denied tests: 11 existing connector cases,
15 pending connector/fallback cases, 12 complete room/allocation cases, eight
independent solver-core cases, 14 new layout/replay cases and five real CLI
cases. Both request input and saved-layout input exercise all six exports.
A separate differential run preserves all 162 legacy layout outcomes. The
original 6,561-world connector digest also remains unchanged. The direct solver
is independently checked over 6,561 small worlds and 400 seeded 4-by-4 worlds.

Review also closed numeric alias acceptance at the internal solver root: Boolean
and floating-point values may compare equal to integers in a set, but do not
satisfy the exact coordinate contract. The public connector already rejected
these inputs; both boundaries now do so.

Observed-map wrapper delegation was reviewed but not executed here. The tests
above do not qualify native acquisition, DFHack, Rust or live fortress effects.
