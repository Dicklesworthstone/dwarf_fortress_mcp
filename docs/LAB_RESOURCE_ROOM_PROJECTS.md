# Gather finite inputs for a complete furnished room

The laboratory can retain a complete `provisioned_room` or `connected_room`
project while gathering its missing materials. Add `acquire_resources` to the
room request. If the observed supplies cannot fund the project, the first
review contains finite WOOD harvesting and any explicitly permitted harvesting
labor. After that work is quiet, `continue_goal` compiles the next stage from
the observed yield. Each stage requires its own review and commit.

This closes the gap between resource acquisition and item-backed room building:
the caller does not need to discard the intended room, calculate a separate
wood target, or reconstruct the room request after harvesting. The original
geometry, furnishing slots, setup permissions and any entrance/access policy
remain bound to the project. This is laboratory functionality; current native
and live admission boundaries are unchanged.

## Request

Pass this object as the JSON string in `fortress.plan`'s `blueprint` argument in
the `starter_fortress_resources` scenario:

```json
{
  "template": "provisioned_room",
  "planner": "provisioned_room_resources_v1",
  "layout": {
    "template": "bedroom_cluster",
    "origin": [2, 6, 10],
    "rooms": 2,
    "room_size": [3, 3]
  },
  "acquire_resources": {"assign_labor": false},
  "prerequisites": {
    "assign_labor": true,
    "workforce": "complete_project_v1",
    "workshops": [
      {"building": "workshop:Carpenters", "location": [0, 0, 10]}
    ]
  }
}
```

New requests may omit `planner`; intake fixes the matching generation. The
presence of the closed `acquire_resources` object enables this workflow. Its
`assign_labor` defaults to false and controls harvesting labor only. The
separate `prerequisites.assign_labor` controls production setup; with
`workforce: "complete_project_v1"`, it also covers the room's mining and
construction staffing. An omitted permission cannot authorize a labor enable.
Duplicate keys, unknown fields, null policies and mismatched planner generations
are refused. The request schema is
[`lab_resource_room.schema.json`](../schemas/lab_resource_room.schema.json).

`dining_hall` is also supported with its existing origin, width and height
layout. Eligible loose items and complete installed items in a continuation
reduce manufacturing demand. Available supplies may fund the room immediately;
opting in never forces an unnecessary harvest. A room requiring two new beds
and a missing Carpenter workshop needs three modeled logs: one for the
workshop, two for its output. An existing suitable workshop removes that setup
cost. Every amount is derived from the actual current source.

## Connected rooms and stairs

Put the same resource-aware request in the `room` field of `connected_room` and
provide the original `access.entry` and bounded `access.region`. The matching
single-level compiler is `connected_room_resources_v1`; omitting the outer
planner selects it when the inner room opts in. For paired stairs, explicitly
select `connected_room_stairs_resources_v1`. The inner room still uses
`provisioned_room_resources_v1`.

The routing and hazard rules are those in
[connected room access](LAB_CONNECTED_ROOM_ACCESS.md): current qualified
occupancy, complete observed routing terrain, ordinary safe mining and, for
stairs, paired adjacent stair cells. Resource acquisition does not grant
permission to enlarge the routing volume, replace the entrance, change the
stair model or weaken its proof. A room with an impossible connector, unsafe
geometry, conflicting workshop site or unavailable required staffing is refused
before its finite sources are depleted.

## Review, commit, observe and continue

The initial response uses schema `dfmcp.resource-room/1` and kind
`resource_room`. Its complete analysis is under `room`, including
`room.resource_workflow.original_request`, the current `stage`, material
requirements and downstream preflight. The response retains the exact current
plan steps and mandatory proof obligations. Its forecast is explicitly scoped
to the current reviewed stage; a harvest forecast is not a forecast of room
completion.

| Stage | Executable work | Exact goal of this plan |
| --- | --- | --- |
| `acquire_resources` | Finite harvesting and permitted harvesting labor | Observed stock for this acquisition stage, with its ordinary harvest proof |
| `room` | Funded setup, production, room/access excavation and exact item placement | The complete reviewed room pursuit, including access when requested |

After reviewing the first seal, call `fortress.commit`, wait for its work to
finish, and observe the retained goal. When eligible, its `continuation_request`
provides the next public call. The equivalent blueprint is:

```json
{
  "template": "continue_goal",
  "plan_digest": "<retained original room or continuation digest>"
}
```

The continuation response has kind `resource_room_continuation`. The server
reconstructs the full original request and recompiles against current observed
inputs. Review and explicitly commit the new seal. If a canceled harvest left
only part of the required wood, continuation gathers the remaining deficit;
if the inputs now suffice, it proposes the room stage. No future harvest yield
is inserted into stock, and no future furniture identity is invented to stand
in for an acquisition stage's final room.

The entire deferred room program is bounded and preflighted before harvesting:
geometry, actual proposed workshop footprints, access, staffing and the shared
64-action limit. Preflight does not execute or reserve that future program.
Later world changes can invalidate it, and every later stage needs fresh
observed evidence and a new review.

## Complete project truth and exact historical proof

Goal observations add `semantic_room_goal.truth`. This reads the complete
original terrain and all required installed furniture with qualified,
reciprocal item/building custody. Connected projects additionally evaluate the
original entrance, region and movement model. Complete source evidence is
required; unknown terrain or unqualified custody cannot establish completion.

The existing `predicate_truth`, action results and first-achieved record still
describe the exact sealed stage. The separate `goal_scope` and
`historical_achievement_scope` label that meaning. A successful harvest can have
True exact stage truth while `semantic_room_goal.truth` is False. Conversely,
after its wood has been consumed to finish the room, the old harvest stock
predicate can be False while the complete project is True. Neither event
rewrites an original action or its historical achievement.

An unfinished or unqualified room project remains protected from goal-capacity
eviction after harvesting. A project with qualified complete room truth and
quiet work can be considered under the ordinary verified-history and custody
rules even when its old WOOD target is no longer held. Candidate comparison
also requires the same complete canonical room request: two different rooms
are not interchangeable merely because their harvest stages need equal WOOD.

## Authority, cancellation and recovery

Current Observe, Query and Plan authority and the complete source budget are
required before proposals and repeated before commit. The complete response
must fit its publication budget before replacing an existing pending plan.
Refused requests preserve the previously reviewed plan. A changed source
anchor requires an explicit new resource-room review; it does not silently
publish or commit replacement work.

Continuation requires verified retained source and physical quiescence across
every pursuit sharing the original root. The check runs again before commit,
so two sessions cannot use separately reviewed continuations to duplicate
active work. Cancellation retains actually observed yield and valid completed
installations. Residual planning reuses them without refunding spent inputs,
minting replacements for retained items or claiming old canceled action proof.

The additive private source generations are `dfmcp.room-continuation/2` and
`dfmcp.connected-room-continuation/2`, with distinct digest domains and matching
resource residual compilers. They retain the complete original request and
flat parent/root lineage. Verified durable recovery reconstructs the exact
source at its original snapshot; it restores no grants or implicit dispatch
authority. Checkpoint restore continues to abandon prior pursuits.

Original room/connected compilers and continuation `/1` retain their canonical
source and semantics. No top-level tool, native RPC or live mutation capability
is added. Current execution evidence and remaining qualification limits are
recorded in [implementation status](../IMPLEMENTATION_STATUS.md).
