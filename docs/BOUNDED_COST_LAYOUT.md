# Tile-budgeted corridors in complete room layout search

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5` (still open).

The existing bounded-cost connector is now selectable by the ordinary automatic
layout workflow, not only by direct room provisioning. Set:

```json
"objective": "minimum-excavation-cost/1",
"connection_strategy": "bounded-cost-with-original-candidate/1"
```

Supply `excavation_costs` with one binary character per original region cell.
These offline costs are assumptions, not terrain evidence or digging authority.
The complete request still requires every original room, furnishing, exclusion,
allowed rotation, fixed origin and entrance. The selected connector strategy is
retained in the downstream RoomPlan, not silently replaced by cost-first routing.

```sh
python3 scripts/layout_rooms.py \
  --request-file examples/room_bounded_cost_layout.json \
  --require-optimal-layout --emit layout > layout.json
python3 scripts/layout_rooms.py --layout-file layout.json \
  --require-optimal-layout --emit room-plan > room-plan.json
```

The example has eight 7-by-7 bedrooms and two dining halls, with 499 required
floor/door/local-corridor tiles and all 28 furnishings. The older cost-first
strategy discards an overlong inexpensive route and retains a five-cost route.
The bounded strategy retains cost/tile alternatives during path search and
finds a one-cost corridor using exactly 13 new tiles, for 512 complete targets.
No tile, rectangle, item or work limit is increased. A separate layered-graph
oracle verifies this two-component corridor's cost and tile count.

`residual_partition: "bounded-original-dig/1"` remains available with either
cost strategy. In the example it proves the assumed one-wall residual fits one
blueprint part and one native rectangle. It does not prove native eligibility.

The result policy is `dfmcp.mrv-bounded-cost-routed-room-layout/1`. Original
requests retain their old algorithm, canonical bytes, trace and work counts.
Saved layouts regenerate the complete original strategy; editing and rehashing
strategy/cost fields cannot preserve a conflicting retained layout.

One shared layout budget covers normalization, all complete candidates, routing
and final ranking. The connector also retains its existing 100,000-check and
32,768-label bounds. Exhausting either aborts without returning an earlier
baseline. A node-limited whole incumbent remains explicitly unproven; strict
optimality export refuses it. No partial room subset is returned.

Optimality is only within the declared origins/rotations and selected deterministic
connection compiler (and residual policy when requested). Joining multiple
components greedily is not a globally optimal corridor network, nor a measured
native mining-time or dwarf-pathfinding claim.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_bounded_layout.py' -v
```
