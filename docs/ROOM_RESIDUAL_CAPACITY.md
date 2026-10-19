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
