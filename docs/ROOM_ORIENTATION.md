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
