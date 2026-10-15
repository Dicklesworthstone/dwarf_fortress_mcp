# Connect the complete intended room project

Owning beads: `df-dfhack-bridge-plane-c-pic.3`, `.4` and `.5` (WP-05/WP-10).

Room provisioning previously checked circulation separately inside each area.
Set `connect_areas: true` in a `dfmcp.room-provisioning-request/1` to join every
original area's walkable floor into one same-level project. This is an offline
geometry proposal, not native pathfinding, terrain safety or excavation authority.

## Use the existing compiler

The complete example is `schemas/examples/connected_rooms_v1.json`. Replace its
fortress identity and coordinates with the intended project. Compile through the
existing entry point:

```sh
python3 scripts/plan_rooms.py compile \
  --request-file schemas/examples/connected_rooms_v1.json > connected-room-plan.json

python3 scripts/plan_rooms.py compile \
  --request-file schemas/examples/connected_rooms_v1.json \
  --emit excavation > connected-excavation.json
```

The example retains two bedrooms and three dining pairs, all twelve furniture
slots and all original per-area circulation witnesses. It adds five corridor
floor tiles in one additional rectangle: the complete plan has 64 floor tiles
and eight excavation parts. The new `project_connections` summary records the
root area, all area names, link endpoints, added cells, partition count, routing
work and a reproducible connection digest.

`connect_areas` must be a JSON Boolean. Omission or `false` preserves the old
normalized intent, plan bytes and digests; no new summary appears. Setting it to
`true` is an explicit request for additional excavation targets, even though this
compiler never dispatches them. Old compiler versions refuse the new field rather
than silently dropping the connection requirement.

## Preserve rooms and avoid unrequested excavation

The canonical first area's entry roots the project. Each next link connects its
current complete floor component to another intended component, minimizing new
excavation and then cardinal path length, with deterministic coordinate ties.
This greedy procedure does **not** claim a globally minimal corridor tree.

Every furniture target is blocked for circulation. Every required bedroom wall
is protected except its original doorway. All declared target-mask exclusions
are respected. Only the original bounded capture is searched; there is no
outside-region detour, hidden expansion, inferred stair or diagonal movement.
Multi-level requests with this option refuse. Legacy independent multi-level
recipes retain their previous behavior.

New corridor cells are packed into disjoint rectangles by exact horizontal runs.
The output never fills the route's bounding box. If walls or exclusions disconnect
an area, or the complete plan cannot fit the existing 512-tile/32-part limits, the
compiler refuses the **whole** request. It never drops a room or publishes a
partially connected substitute. Eight areas and 32 furnishings remain supported
when their complete geometry fits the original bounds.

## One retained intention through existing consumers

The new cells enter the existing `excavation_blueprint`, `target_tiles` and
`excavation_mask_digest`; they are not a separate disposable annotation. The
normalized original intent records the opt-in, and the complete `RoomPlan` digest
binds its result. Decoding regenerates the corridors and checks every canonical
byte. Dropping a corridor or forging the summary cannot pass import validation.

Furniture selection, constraints and dependency order are unchanged. The existing
allocator still allocates the entire furniture request together, or returns a
complete shortage with no executable subset. `plan_rooms.py allocate` uses the
same compiler and import path, so it retains the connected room intention without
another command or native protocol. The room handoff retains the whole RoomPlan;
the terrain survey/goal source derives its floor mask from the complete excavation
blueprint, including these corridors. No placement or monitor protocol was changed.

Native excavation still requires the existing fresh map survey, reviewed dig
plan and explicitly authorized execution. Subsequent terrain/furnishing evidence
remains necessary. Connected intended geometry does not prove current terrain,
connection to the existing fortress, room assignments, safe movement, or completion.
No source, compatibility or production-admission claim is added.

## Bounded execution and evidence

Routing uses at most 1,024 capture cells and 100,000 routing work units. The
128 KiB internal decision witness binds the original floor, protected cells,
entries, selected paths and exact partition. `work_units` in the summary counts
the router only; compilation and exclusion preparation also call the enclosing
guard. Excluded volumes are tested against the bounded capture, never expanded.
Every routing checkpoint retains the caller's existing deadline/authority guard.

Executed component tests:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_connections test_connected_room_provisioning -v
```

All 27 methods passed. They execute the actual Python router, room compiler and
furniture allocator. Coverage includes all 128 3x3 barrier masks versus exhaustive
simple paths; all 512 3x3 masks versus exact partition coverage; every cancellation
checkpoint in the router and mixed-room compiler; northern bedroom-wall detours;
whole-project exclusions and bounds; maximum area/slot selections; regeneration,
shortages, the checked-in example/contract, and separate-process hash-seed equality.

All 314 legacy template requests were compared with a witness generated from the
unmodified Git-blob-verified compiler: 310 successful full artifacts retain their
exact bytes and four retain refusal. Explicit `false` also preserves those bytes.
The pre-change compiler blob is `1a486bd993bc19c6d6f19d9999eac5d1682ac1b9`.

This evidence is not native DFHack, CLI/TCP pipeline, Rust/MCP or full-repository
qualification. Those broader suites were not executed in this environment.
The machine contract is `architecture/room_connections_v1.json`.
