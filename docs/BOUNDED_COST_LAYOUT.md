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

## Raw-map workflow and original excavation handoff

The surveyed workflow selects the same exact strategy with the observed objective:

```json
"objective": "minimum-observed-excavation/1",
"connection_strategy": "bounded-cost-with-original-candidate/1",
"residual_partition": "bounded-original-dig/1"
```

Do not supply `excavation_costs` on this path. The original raw map supplies
wall/floor costs before caller site constraints are intersected. Preflight
retains the exact requested connector instead of reverting to the old algorithm.
Hidden, wet, occupied, designated and otherwise excluded cells remain unavailable;
required bedroom walls, entrances and original item constraints remain binding.

After configuring the existing isolated read-only map authority, and changing the
example's fortress selectors and region to match the intended fortress:

```sh
python3 scripts/survey_room_layout.py \
  --request-file examples/room_bounded_cost_observed.json \
  --require-optimal-layout --emit survey > survey.json
python3 scripts/survey_room_layout.py --survey-file survey.json \
  --require-optimal-layout --emit room-plan > room-plan.json
```

The existing padded-map composition also delegates to this exact strategy and
can retain the original-room excavation handoff. In the synthetic complete-room
regression, the selected map contains 511 existing floor targets and one wall
within the 512-target project. The actual padded compiler and handoff codec retain
all 28 furnishings and emit one residual blueprint part/native rectangle. This
uses one supplied parent capture and exact derived subregions, not independent
observations or an actual native dig. Empty residual work still cannot create
a replacement excavation blueprint.

All six offline-layout exports and all seven surveyed exports are tested from
original requests and saved artifacts. Saved replay stays offline; changing and
rehashing the strategy, costs or evidence cannot preserve inconsistent outputs.
The map stream closes before search. Exhausted labels/work, caller cancellation,
source faults or changed read authority never publish an earlier candidate or
repeat acquisition. A failed output attempt never emits a second response.

```sh
PYTHONPATH=scripts:tests python3 -W error -m unittest \
  test_room_bounded_layout test_room_bounded_observed_layout -v
```

Qualification is Python component/synthetic-source evidence: 31 new tests and
62 unchanged regressions pass in completed bounded invocations. Fifteen joined
synthetic peers exercise the actual map client and surveyed CLI subprocesses.
The padded compiler/handoff API was exercised, not the padded excavation CLI or
native digging. Real DFHack, Rust/native builds, room-backed effects, live-fortress
operation and full-repository qualification remain unexecuted. No native method,
receipt/journal format, mutation authority or production admission is changed.
