# Plan complete rooms that fit the remaining excavation profile

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5` (not closed).

A complete room layout can still be unusable as a dig proposal: scattered
remaining walls may exceed the original 32 blueprint parts or 128 normal-mining
rectangles. Choosing the fewest wall targets does not imply these bounds fit.

The offline cost-layout request can now opt into:

```json
"objective": "minimum-excavation-cost/1",
"excavation_costs": "<one binary character per original domain tile>",
"residual_partition": "bounded-original-dig/1"
```

Under this explicit strategy, a cost-one target is assumed to be a remaining
wall and a cost-zero target is assumed already open. This is a planning contract,
not terrain evidence. A fresh terrain survey and the original native dig
review/confirmation are still required. Other objectives cannot select this
strategy without its complete binary cost map.

Every compiler-accepted leaf is checked before it can become the incumbent.
The check uses the final exact target union, including access points and
corridors. The existing terrain reducer and the search share the same smaller
of global/original-part width-first covers and the same independent 8-by-8
normal-mining partition. Only explicit capacity excess rejects a candidate;
invalid data, exhausted work and caller cancellation abort the operation.
No room, furnishing, original constraint or residual target is removed to fit.

The new result's `residual_partition` section reports checked and rejected
candidate counts plus the selected complete residual count, mask digest and
partition sizes. It does not export a partial blueprint. Zero residual is a
valid complete no-work plan, distinct from no layout; the existing handoff
workflow still cannot manufacture a nonempty excavation from it.

A node-limited result may retain only a complete capacity-valid incumbent. It
cannot call that result optimal. Work/deadline/cancellation refusal publishes
no incumbent. The same caller budget covers search, all partition attempts and
the final selected-mask recount. `--require-optimal-layout` on the existing
layout CLI applies to this constrained search without changing its limits.

Optimality and no-layout conclusions are relative to the selected deterministic
connection and partition policies, not every possible rectangle cover or global
corridor network. The partition itself is not asserted to be minimal.

The example is executable offline:

```sh
python3 scripts/layout_rooms.py --request-file examples/room_residual_capacity.json --require-optimal-layout --emit layout
```

Its 32-furnishing hall initially chooses 59 isolated wall targets, which cannot
fit 32 parts. With the strategy, search rejects seven unsuitable positions and
selects a complete 117-tile footprint with 90 remaining targets in 28 parts.
All 32 original furnishings remain. More wall targets are necessary to obtain
a representable proposal in this declared domain; no native budget is raised.

The strategy and cost map remain in the original normalized request. Saved
layout decoding repeats the full search and selected partition before accepting
any bytes. Omitting the strategy retains legacy output, trace and work counts.
This does not change RoomPlan geometry, native protocols, effect keys, journals,
mutation authority, or production admission.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_residual_capacity.py' -v
```

## Use actual observed walls and produce an original excavation handoff

Both existing map-backed commands accept the same residual strategy paired with
`minimum-observed-excavation/1`. They reject caller-authored `excavation_costs`.
The original decoded map, before caller site-mask intersection, supplies the
wall/floor distinction. A caller's floor-use permission cannot convert a wall
into an already-open tile or erase its residual partition cost.

```json
"objective": "minimum-observed-excavation/1",
"residual_partition": "bounded-original-dig/1"
```

`survey_room_layout.py` independently rebuilds the selected residual from raw
wall coordinates and checks its count, mask digest and both partitions against
the search result. Its evidence retains the original map witness and tick.
This unpadded observation does not prove remaining-wall halo eligibility.

`survey_room_excavation.py` obtains one complete padded capture and projects the
planning and final terrain views from those same bytes. Its existing halo
constraints remain mandatory. The final original terrain survey rechecks the
selected residual count, digest and partition sizes before the existing
`RoomExcavationHandoff` can be exported. No second native read, automatic dig,
replacement key, journal creation or furniture placement occurs.

After setting the example's fortress and map region for the intended source,
use the existing isolated map-read environment and run:

```sh
python3 scripts/survey_room_excavation.py --request-file examples/room_residual_capacity_observed.json --require-optimal-layout --emit compilation > compilation.json
python3 scripts/survey_room_excavation.py --compilation-file compilation.json --require-optimal-layout --emit excavation-handoff > room-excavation-handoff.json
```

The second command replays saved evidence offline. It is not a fresh observation.
The handoff is input to the existing explicitly reviewed room-backed dig batch;
it grants neither placement authority nor permission to repeat uncertain work.
A complete zero-residual layout has no excavation handoff and remains distinct
from no feasible layout. The command's `--require-optimal-layout` flag works for
all nine exports and refuses an unproven incumbent without exporting an artifact.
It cannot invent an objective, change the selected strategy or renew a budget.

The synthetic 32-furnishing case previously ends in a blocked terrain survey.
With the constraint it exports an exact 90-target, 28-part original handoff.
Search assumptions, the raw-map recount, the final padded terrain survey and
handoff all agree on its mask identity. The same workflow retains all eight
areas and 32 furnishings in a separate fixed-origin case. Native mineability,
structural support, aquifers, current safety and completed rooms remain unproven.

Validation uses 105 warnings-denied Python tests: 43 new capacity/raw-source/CLI
cases and 62 unchanged compiler/connector cases. The new CLI suite performs
19 joined synthetic TCP sessions with the actual map client and all nine
request/saved exports. Source loss, changed authorization, exact node/work
limits, final deadlines and short output do not repeat acquisition or publish a
stale handoff. All 404 independent legacy result/work-count comparisons remain
identical to the fetched baseline. These are component results, not real DFHack,
Rust/native, actual dig dispatch or full-repository qualification.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_residual*.py' -v
```
