# Furnish rooms from another level

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5`,
`df-franken-storage-mvcc-54h.4`.

Room recipes now accept `max_vertical_distance` in each furniture-kind entry of
`item_constraints`. It is an exact integer in 0..127, measured between the item
and target levels. The default is zero: same-level endpoints. An explicit zero
normalizes to the original canonical bytes, including for rotated areas.

```json
{
  "schema": "dfmcp.room-provisioning-request/1",
  "world_folder": "your-save-folder",
  "site": 7,
  "areas": [{
    "name": "dining",
    "origin": [100, 100, 2],
    "template": {"kind": "dining_hall", "table_count": 4, "columns": 4},
    "item_constraints": {
      "chair": {"max_vertical_distance": 2, "max_distance": 100},
      "table": {"max_vertical_distance": 2, "max_distance": 100}
    }
  }]
}
```

Compile with the existing `plan_rooms.py compile` command. Its
`--emit furniture-request` export retains the limits for every original slot:

```sh
python3 scripts/plan_rooms.py compile --request-file rooms.json --emit plan > room-plan.json
python3 scripts/plan_rooms.py compile --request-file rooms.json --emit furniture-request > furniture-request.json
```

Use the existing observed-route furniture allocation/review workflow with that
complete request and a map region spanning both endpoints and the staircase.
The ordinary Manhattan-only allocator, including `plan_rooms.py allocate`,
refuses any nonzero vertical limit; it cannot silently drop the constraint.

A vertical limit is not a path or permission to create stairs. The existing
route allocator still requires observed dry traversable cells and compatible
up/down stair pairs. Material, subtype, excluded-item, total 3-D Manhattan and
independent route-length limits all remain binding. Unknown, occupied, missing
or incorrectly oriented stairs cannot establish a route. A shortage produces no
partial executable plan. Room geometry, entrance corridors, native assignments,
source admission and mutation authority are unchanged.

Validation: 12 focused tests use the complete real compiler and route allocator
with synthetic map/1.5 bytes, including all 32 furnishings, every template and
rotation, broken/hidden/occupied stairs, exact limits and unchanged same-level
behavior. A differential run preserved 208 pre-change room artifacts byte for
byte. This is Python component evidence, not live/native/CLI qualification.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_cross_level_routes.py' -v
```
