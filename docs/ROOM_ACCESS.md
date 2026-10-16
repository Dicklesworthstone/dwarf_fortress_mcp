# Entrance-anchored room projects

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5` (partial progress).

A connected set of new rooms is not necessarily connected to the existing
fortress. Room provisioning can now retain one caller-selected access point and
route every area to it inside an explicitly bounded same-level domain:

```json
{
  "schema": "dfmcp.room-provisioning-request/1",
  "world_folder": "region1",
  "site": 2,
  "connect_areas": true,
  "access": {
    "point": [10, 10, 2],
    "region": {"origin": [10, 10, 2], "size": [10, 6, 1]}
  },
  "areas": [{
    "name": "hall",
    "origin": [16, 10, 2],
    "template": {"kind": "dining_hall", "table_count": 1}
  }]
}
```

Use the existing `scripts/plan_rooms.py compile` command with this request.
The entrance and selected corridor tiles become part of the original full
excavation-floor goal, including the entrance when it lies outside the room
footprints. Furniture targets remain unchanged. All eight area slots remain
available: the access point does not consume a room slot or an inventory item.

The routing region is permission to search, **not permission to excavate its
bounding box**. It may be larger than the original room hull, allowing an access
corridor to detour around an exclusion. Only the exact selected mask is emitted;
the downstream capture and mask digest use its tight hull. The complete request
still has the original 512-target, 32-part, 32-furnishing and 1,024-capture-cell
limits. The entrance itself counts toward those limits. The point must lie in
the routing domain; all original room floors must fit too. `connect_areas` must
be explicitly true. No other level or outside-domain detour is invented.

The same original connection algorithm runs from the explicit point, minimizing
new target cells per greedy link, then cardinal length and coordinate ties. It
is not a globally optimal corridor-tree algorithm. Every furniture cell,
required bedroom wall, excluded target and floor-forbidden site cell remains
blocked. This applies at the access point too. It cannot be put through a bed,
a wall that must be retained, an excluded tile or an unsupported site cell.

The complete intent retains the point and routing domain; the connection
witness also binds the root even when no added corridor is needed. Import
regenerates all output, so changing the root, domain, corridor mask or claims
is rejected. Existing requests without `access` retain their previous bytes.

## Observation and execution remain separate

Offline selection does not prove the point is an existing fortress entrance,
that its tile is currently floor, or that dwarves can reach it. The result says
`all_areas_connected_to_point: true` only for intended geometry; both
`point_floor_observed` and `point_is_existing_fortress_entrance_proven` remain
false. `existing_fort_access_proven` and mutation authority remain false too.

Survey the complete room plan with the existing `scripts/survey_rooms.py`
workflow before mining. Its original-room excavation handoff derives only
remaining walls: an already-dug entrance or corridor floor must not become a
new native designation. The full original floor goal and the remaining-wall
effect mask are distinct. Normal fresh native review, original-key custody,
construction monitoring and room assignment requirements are unchanged.

No native method, production runner, capability, dependency or journal format
is added. There is no automatic game mutation or background work.

## Component checks

```sh
python3 -m unittest discover -s tests -p 'test_room_access.py' -v
```

Sixteen tests execute the actual room compiler, connection algorithm and item
allocator. They include obstacle detours outside the old hull, eight rooms plus
access, all four bedroom rotations, inaccessible roots, immutable artifacts,
exact mask digests, budgets, process hash-seed replay and cancellation at every
guarded point in the small access fixture. Separately, 318 legacy compiler
inputs were compared against the exact fetched pre-change source: all accepted
artifact bytes and refusal reasons remained unchanged. These are Python
component results, not Rust/native/live or full-repository qualification.

## Search and survey the entrance together

Automatic layout accepts `access_point` at the top level. Its existing `region`
is also the explicit corridor-search domain. The point stays fixed when the
solver moves or rotates rooms; it is not a focus preference that may be ignored.

```json
{
  "schema": "dfmcp.room-layout-request/1",
  "world_folder": "region1",
  "site": 2,
  "region": {"origin": [10, 10, 2], "size": [10, 6, 1]},
  "access_point": [10, 10, 2],
  "areas": [{
    "name": "hall",
    "origin": [16, 10, 2],
    "template": {"kind": "dining_hall", "table_count": 1}
  }]
}
```

The existing `layout_rooms.py` solve/replay/export commands accept this request.
The existing `survey_room_layout.py --request-file` command uses the same request
with its isolated read-only environment (`DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT=1`,
`DFMCP_MAP_TOKEN`, optionally `DFMCP_ROOM_LAYOUT_ENDPOINT`). It acquires exactly
one map/1.5 capture and closes the stream before search; no new native API or
credential source is introduced. Tokens must not be placed in request files.

The solver now checks complete entrance-to-room routes at each full candidate.
A candidate that puts furniture on the point, strands a room behind a barrier,
or exceeds the total corridor allowance is rejected and search continues.
Required walls cannot occupy the point. All original origin, rotation, site and
exclusion constraints remain fixed. A corridor work-limit failure is explicitly
inconclusive, never misreported as an exhausted infeasible domain. Node/work
limits, cancellation and final output bounds remain shared across the search.
Access searches identify policy `dfmcp.mrv-entrance-room-layout/1`.

For observed layout, the point must be **visible, dry, undesignated, unoccupied
floor in the original decoded capture**. Missing, hidden, liquid-bearing,
occupied, designated, wall-shaped or other-shape points refuse. This check runs
before intersecting the caller's site mask: a caller's `F` permission on a native
wall cannot become observed-floor evidence. A valid point that is separated
from the planned rooms can still produce `no_layout_in_domain`, without a plan.

The survey's `access_evidence` retains the exact point, map witness and game tick
and reports `point_floor_observed: true` with `same_capture_as_layout: true`.
It does not attest that the tile is connected to the rest of the fortress or is
currently reachable. `point_is_existing_fortress_entrance_proven`,
`native_pathfinding_proven` and `current_access_proven` stay false. The embedded
pure room-plan artifact also keeps its observation flags false. Saved survey
replay rederives the point fact from raw bytes and reports no acquisition in the
current call; it cannot relabel old evidence as a fresh observation.

Seventeen added search/evidence tests and six actual CLI/TCP tests pass alongside
the sixteen compiler tests (39 methods total). Eighteen joined synthetic-peer
sessions exercised the unchanged client/CLI, including ten unusable entrance
states, source confusion, dropped responses and post-capture authority
revocation. All seven saved-survey exports were replayed offline. In a separate
112-request comparison, all 224 legacy layout/observed-layout outcomes remained
byte-identical to the original source. This is component and synthetic-protocol
evidence only; no real DFHack plugin, live fortress or full workspace was run.

```sh
python3 -m unittest discover -s tests -p 'test_room_access*.py' -v
```
