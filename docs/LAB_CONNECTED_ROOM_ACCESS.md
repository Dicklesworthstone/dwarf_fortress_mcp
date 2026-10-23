# Connect and furnish a laboratory room

The `connected_room` blueprint combines a complete item-backed room with a
connector from an explicitly selected entrance. It reuses eligible furniture,
manufactures missing items from finite WOOD when needed, excavates the room and
connector, and installs exact items. Its final goal also requires current access
to every furnishing from that original entrance. Use the existing
`fortress.plan`, `fortress.commit`, wait and prove workflow; no tool is added.

This is the bounded reference simulator's structural movement model. The
completion predicate is `Predicate::LaboratoryRoomAccess`, with fixed policy
`dfmcp.laboratory-room-access/1`. It retains the entrance, domain and furnishing
targets and is evaluated against the current snapshot whenever an original
goal, forecast or archived proof is checked. Implementation and development
checks do not establish full runtime qualification or native compatibility;
the exact evidence posture remains in
[IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md).

## Plan a connected room

Pass this object as the JSON string in `fortress.plan`'s `blueprint` argument:

```json
{
  "template": "connected_room",
  "planner": "connected_room_v1",
  "room": {
    "template": "provisioned_room",
    "planner": "provisioned_room_v1",
    "layout": {
      "template": "bedroom_cluster",
      "origin": [2, 6, 10],
      "rooms": 2,
      "room_size": [3, 3]
    },
    "prerequisites": {
      "assign_labor": true,
      "workshops": [
        {
          "building": "workshop:Carpenters",
          "location": [1, 0, 10]
        }
      ]
    }
  },
  "access": {
    "entry": [0, 2, 10],
    "region": {
      "min": [0, 0, 10],
      "max": [12, 12, 10]
    }
  }
}
```

These coordinates match the starter-fortress example. For another snapshot,
select an observed, unoccupied FLOOR entrance and a domain containing that
entrance plus all original rooms, doorways, corridors and furnishing targets.
Region endpoints are inclusive. The entry cannot be a furnishing target or a
protected bedroom wall tile. The complete routing region must be observed;
planning does not assume missing terrain is safe to excavate.

The nested `room` accepts the same `bedroom_cluster` and `dining_hall` layouts
and optional prerequisite grammar as [item-backed furniture](LAB_ITEM_FURNITURE.md).
The setup above permits labor assignment and an explicitly chosen carpenter
site when required. Existing suitable capacity is reused, and unused setup
choices remain part of the sealed request. A dining hall supplies its table and
chair through the same exact-item workflow. Doors are not added automatically.

New intake may omit the two `planner` markers; canonical retained source fixes
them to the versions shown. Unknown fields, replacement planner generations,
and archived source missing its original version markers are refused. Review
the returned inventory and allocation analysis together with the `access`
analysis: `route`, `room_endpoint`, `new_corridor_tiles`, `corridor_parts`, the
selection policy and work estimate. Planning and forecasting leave the source
fortress unchanged.

## Deterministic connector routing

The compiler first fixes the room endpoint. A bedroom cluster uses its template's
designated access point. A dining hall uses the first unoccupied, non-furnishing
perimeter tile in `(z, y, x)` order. It verifies that the intended room floor
geometry connects this endpoint to an approach beside every furnishing.

Within that endpoint contract, routing policy
`laboratory-room-mining-cardinal/1` chooses:

1. The fewest new ordinary wall tiles outside the room's already planned floor
   geometry.
2. The shortest cardinal path among those choices.
3. The lexicographically first complete path in `(z, y, x)` order when both
   costs are equal.

Observed floor can therefore win over a shorter route that requires mining.
The compiler avoids current qualified building footprints, furnishing target
tiles and workshops actually planned by the same production setup. Ordinary
wall candidates must pass the existing hazard checks; other terrain cannot be
used as a connector. An incomplete hazard halo, magma proximity or unsupported
excavation span refuses the affected route. The optimization does not search
for a cheaper alternative room entrance or leave the supplied domain.

For an outside entry, the route joins the room at its fixed endpoint. An entry
already inside the intended floor geometry must be in the same connected
internal circulation. A narrow layout whose furniture blocks every required
approach is refused even if its excavation and item quantities are otherwise
valid.

Bedroom wall rings constrain routing: the connector does not mine through the
template's separating wall tiles outside its explicit doorway and corridor
openings. **This is not a terminal wall-preservation promise.** The compiler
does not establish that every ring tile is currently a wall, and the final goal
does not require those tiles to remain walls after unrelated changes.

Only selected connector wall tiles become mining actions. Contiguous runs are
partitioned into disjoint exact rectangles; the compiler does not excavate a
route's entire bounding box. Existing route floor needs no new mining action.
Each new furniture placement waits for verified production, all remaining room
excavation and every connector excavation. Work-scaled excavation deadlines and
the ordinary dependency deadlines remain in force.

## Complete terminal goal

The reviewed pursuit must establish all of the following:

- Every original room, doorway and internal corridor terrain region.
- Every connector region that this pursuit selected for new excavation.
- Every exact installed furnishing, including original generation, material,
  production provenance and exclusive reciprocal item/building custody.
- Current entrance-to-furnishing access within the original routing domain.

Production stock quotas are intermediate conditions. Installing furniture spends
loose inventory, so those quotas are not retained as final room requirements.
Neither a completed work order nor a compile-time route proves the full goal.

## Meaning of access

The region contains at most 1,024 tiles on one level. The entrance and all
one to sixteen distinct furnishing targets must be inside it. Movement uses
cardinal neighbors whose observed canonical terrain is FLOOR. Buildings and
all furnishing target tiles block traversal, including planned furnishings
that have not yet been installed. Each target must have at least one adjacent
floor tile reachable from the original entrance.

This model treats every building footprint as an obstacle, including doors.
It describes structural floor access from the selected point. It does not
establish native DF pathfinding, actual hauling, unit access permissions,
connection of an arbitrary selected point to the whole fortress, bedroom
ownership, or a sleeping/needs benefit.

The predicate is true when every target has a currently established approach.
It is false when even allowing unknown terrain cannot provide every approach
inside this exact region. It is unknown when missing terrain could contain a
necessary route, or the complete qualified building census is unavailable.
A route outside the supplied region does not satisfy this bounded claim.

## Current occupancy and source authority

The complete building domain is inspected, including canonical aliases of the
building kind. Its bound is 65,536 entities. Qualified laboratory lifecycle and
full footprint facts establish occupied tiles. A malformed or unqualified
building record cannot silently become free space. Valid cancelled generic
buildings and cancelled/released furniture no longer obstruct the route;
marked furniture history must still establish the registered original
placement identity before its old footprint can be ignored.

An aliased building remains a routing obstacle. If its footprint overlaps the
original room geometry or an actual proposed workshop, connected compilation
refuses: the older excavation and installation machinery cannot establish
typed custody for that representation. Unrelated aliases do not prevent a
safe route around them.

The reference model's trusted laboratory evidence owner is required to use
this predicate as a proof. An arbitrary hash-valid snapshot, caller-provided
source label, replay fact or live scoped grant cannot acquire laboratory
authority. Raw queries remain structural inspection rather than admission.
Terrain absence stays unknown, even within the complete laboratory entity
universe.

Occupancy is rasterized with bounded rectangle differences, then two cardinal
searches distinguish established and potential routes. Both predicate-tree
shape and query work budgets include these extra census/search costs. All
shape checks run before allocation and recursive evaluation; duplicates,
out-of-region targets, multiple levels and oversized work refuse explicitly.

## Durable proof and concurrency

Canonical and plan-archive predicate tags append the new variant while
preserving all earlier tags and bytes. Target order is normalized by level,
row and column; duplicates remain invalid after normalization. Archived
reconstruction retains the exact entrance, domain and furnishing targets.

Read witnesses include the complete building domain as well as the terrain
region. Inserting a previously unseen building, moving a footprint, changing
a lifecycle or removing a building invalidates the old occupancy witness.
Existing witness certificates that do not use this predicate retain their
original representation. Compilers that also select inventory, retain
installation custody or choose routes must recheck those additional inputs at
a changed anchor.

A later obstruction can change current goal truth without rewriting an old
receipt or the first-achieved historical record. Current access may use any
qualifying route inside the original domain; the selected route is not an
instruction to treat those coordinates as permanently traversable. The other
terrain and exact-furniture clauses still have to hold.

## Continue an interrupted room or repair access

Use the existing retained-goal request:

```json
{
  "template": "continue_goal",
  "plan_digest": "<retained committed connected-room plan digest>"
}
```

The digest can identify the original connected plan or an earlier connected
continuation. The new pursuit preserves the complete original room request,
all supplied setup choices, **the original entry and the exact original access
region**. A caller cannot replace the entry, enlarge the domain, inject lineage
or drop an inconvenient furnishing through this request.

The residual compiler retains valid completed installations under their exact
original identities, then allocates or manufactures only missing furnishings.
It excludes installed tiles from new excavation and recomputes any necessary
connector in the original domain. If the room is already furnished but a wall
now cuts access, a continuation can contain only mining actions. It neither
rebuilds furniture nor spends new WOOD solely to repair that connector. An
entry that has ceased to be unoccupied observed floor is refused rather than
silently replaced.

Current Observe and Plan authority, a retained nonabandoned source, current
False goal evidence and proven quiet work across every same-root pursuit are
required. These conditions are rechecked at commit. Unknown goal evidence
requires reconciliation; an active sibling or a currently satisfied quiet
same-root pursuit prevents duplicate continuation. The proposal requires an
explicit commit of its own new seal.

Successful continuation proves the complete room and access request for that
reviewed pursuit. It preserves earlier cancelled action outcomes, original
exact goals and first-achieved records. A historical original goal can remain
False while its child pursuit is True. See
[room goal continuation](ROOM_GOAL_CONTINUATION.md) for the common authority,
custody and historical-proof rules.

## Replay and durable recovery

The full connected request, including unused setup and access choices, is bound
into the intent summary and seal. Any changed anchor requires a newly reviewed
source replay for both original connected plans and continuations. This also
applies to excavation-only access repairs: ordinary action witnesses cannot
stand in for current route, inventory, occupancy and retained-generation checks.
A stale commit dispatches nothing; when replay succeeds, review and explicitly
commit the replacement seal.

Original source uses the distinct durable `connected_room` tag. Continuations
use `connected_room_continuation`, the private
`dfmcp.connected-room-continuation/1` envelope and fixed
`connected_room_residual_v1` compiler. The envelope keeps parent/root digests and
the complete original `connected_room_v1` request without nesting earlier
envelopes. It is retained server source, not an accepted public blueprint.

Exact archived reconstruction uses the original source, sealed snapshot and
intent identity. It does not consult the current mutable goal book, grant
authority or dispatch effects. Durable compaction and verified reopen preserve
the original source and each pursuit's separate history under the recovered
observation frontier. Checkpoint restore still abandons prior pursuits; restore
cannot turn an old connected request into implicitly authorized current work.

## Explicit limits and compatibility

| Domain | Current bound |
| --- | --- |
| Connected request and its canonical source | 65,536 bytes |
| Private continuation envelope | 16,384 bytes, including the retained request |
| Routing region | At most 1,024 cells on one level |
| Furnishing targets | 1 to 16 distinct tiles, all inside the region |
| Combined setup, production, excavation and placement | At most 64 actions |
| Source census | At most 65,536 entities and 4,096 terrain chunks |
| Each source terrain chunk | Canonical 16-by-16 shape, at most 256 positive runs totaling 256 tiles, and at most 256 valid overlay offsets |
| Compiler structural work | At most 12,000,000 charged work units |

The work allowance includes source validation, spatial indexing, occupancy,
route search and route reconstruction. It is a conservative structural bound,
not elapsed time. Predicate-tree and query limits separately charge current
access evaluation. Session authority, response, action and general planner
budgets may refuse a request before these maxima; the bounds are not a promise
that every combination is accepted.

Ordinary `provisioned_room_v1` and `provisioned_room_residual_v1` requests retain
their previous output, source identity and semantics. They do not silently gain
an access requirement. The legacy abstract `bedroom_cluster` and `dining_hall`
blueprints also retain their interpretation. This new wrapper does not add
native pathfinding, hauling execution, construction staffing, room ownership
or a sleeping/needs benefit.
