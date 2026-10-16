# Room template orientation

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5`.

A room-provisioning area may now specify `rotation`: an exact integer from
`0`, `90`, `180`, `270`. Angles are clockwise in map/screen coordinates
(+x right, +y down), around the area's requested `origin`. The origin remains
the original template pivot; it is not necessarily the rotated bounding-box
minimum. No mirror, vertical movement, arbitrary angle or automatic origin
adjustment is implied.

```json
{
  "schema": "dfmcp.room-provisioning-request/1",
  "world_folder": "your-save-folder",
  "site": 7,
  "areas": [{
    "name": "dining",
    "origin": [100, 100, 2],
    "rotation": 90,
    "template": {"kind": "dining_hall", "table_count": 4, "columns": 4}
  }]
}
```

Pass this request through the existing `scripts/plan_rooms.py` workflow. This
example produces a 3-by-13 floor at `[98, 100, 2]` rather than a 13-by-3 floor.
Bedroom clusters rotate their entire corridor network, room rectangles,
doorways, furnishing targets and entry point together. Item constraints and
furnishing dependencies are unchanged.

The same template compiler still owns geometry. Rotation occurs before
whole-project overlap, exclusion, floor/wall site-mask, circulation, corridor
connection and furniture-allocation checks. Bounds are checked on the final
rotated geometry; a valid shape near a map edge is not rejected just because
the unrotated template would extend off-map. No partial plan is emitted on
failure or cancellation.

Omitting `rotation` or supplying `0` preserves the original canonical plan
bytes and digests. Nonzero angles are retained in the sealed intent, and
artifact decoding regenerates the complete plan. Editing either the angle or
derived coordinates invalidates the artifact. Older implementations reject
nonzero-rotation intents instead of silently treating them as unrotated.

This is advisory geometry and existing artifact lowering only. It does not
create native room assignments, prove current terrain/pathfinding or
construction completion, grant mutation authority, admit a production tuple,
or add an MCP tool/native protocol generation.

Focused regression command:

```sh
python3 -m unittest discover -s tests -p 'test_room_orientation.py' -v
```

## Automatic orientation selection

The existing `scripts/layout_rooms.py` command accepts either a fixed
`rotation` or a nonempty `rotations` array on each layout area. The array is a
set of permitted quarter-turns, not a preference order. Duplicate angles,
noninteger values and supplying both fields are rejected. Omitting both keeps
the original unrotated search; a single zero choice is canonically identical
to the old request.

```json
{
  "schema": "dfmcp.room-layout-request/1",
  "world_folder": "your-save-folder",
  "site": 7,
  "region": {"origin": [100, 100, 2], "size": [3, 13, 1]},
  "areas": [{
    "name": "dining",
    "rotations": [0, 90, 180, 270],
    "template": {"kind": "dining_hall", "table_count": 4, "columns": 4}
  }]
}
```

Save this as `layout-request.json`, then run:

```sh
python3 scripts/layout_rooms.py --request-file layout-request.json --emit layout > layout.json
python3 scripts/layout_rooms.py --layout-file layout.json --emit room-plan > room-plan.json
```

This narrow site has no unrotated fit. The solver selects a 90-degree hall at
pivot `[102, 100, 2]`, then produces the existing complete room-plan artifact.
All other exports (`room-request`, `excavation`, `furniture-request`, `report`)
remain available. Replay regenerates both the search and the selected plan.

Each area is one variable in the existing whole-project backtracking search.
Candidates include every permitted orientation and translation that fits the
floor/wall constraints. Different areas can choose different orientations;
fixed origins, origin regions, exclusions and site masks stay in world
coordinates and are never rotated or relaxed. The selected angle is frozen in
the downstream room intent, while the layout artifact retains the allowed set.

Ordering is deterministic: entry distance to focus, origin y/x/z, then angle;
area selection still uses minimum remaining candidates with lexical area ties.
There are at most 4,096 candidates per area and 32,768 across eight areas.
The original aggregate work and node allowances cover all variants and final
compilation. Exhausting a node allowance remains inconclusive; exhausting work
or cancellation returns no partial plan. A no-layout result is only relative
to the declared finite domain, allowed angles and existing corridor compiler,
not a global impossibility or optimality claim.

Oriented searches report policy `dfmcp.mrv-oriented-room-layout/1`. Legacy
requests retain their previous policy, output bytes, trace and work counts.
The observed-site wrapper delegates to this same normalizer/solver; no extra
native operation or new mutation authority is introduced.

```sh
python3 -m unittest discover -s tests -p 'test_room_layout*.py' -v
```
