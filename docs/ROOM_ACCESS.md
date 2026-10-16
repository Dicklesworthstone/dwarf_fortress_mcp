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
