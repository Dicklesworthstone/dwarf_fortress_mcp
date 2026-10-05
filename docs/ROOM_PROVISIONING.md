# Room-count intentions to excavation and furnishings

`room_provisioning.RoomPlan` compiles a complete bounded room request into the
existing `dfmcp.excavation-blueprint/1` and `dfmcp.furniture-request/1` artifacts.
It replaces manual per-tile/per-item authoring, not native observation, review,
placement custody, or subsequent completion evidence. It is a recipe proposal,
**not a prepared mutation plan or proof of completed rooms**.

## Closed request

```json
{
  "schema": "dfmcp.room-provisioning-request/1",
  "world_folder": "region1",
  "site": 2,
  "areas": [
    {
      "name": "sleep",
      "origin": [10, 10, 2],
      "template": {"kind": "bedroom_cluster", "rooms_count": 2, "room_size": [3, 3]},
      "item_constraints": {"bed": {"material": [419, -1], "max_distance": 100}}
    },
    {
      "name": "eat",
      "origin": [10, 20, 2],
      "template": {"kind": "dining_hall", "table_count": 3, "columns": 3}
    }
  ],
  "excluded_items": [],
  "excluded_regions": []
}
```

The IDs and coordinates are illustrative. Each named area has an explicit origin.
Bedrooms use the existing geometry policy's four-room rows, separating walls,
south doorways, row corridors and west connecting spine. Each room requests one
bed, chair and table. Interiors are 3..7 tiles in each horizontal dimension.
Dining areas request chair/table pairs in rows of 1..4 pairs with clear aisles.
These templates do not create native bedroom zones, dining assignments or doors.

Per-area `item_constraints` accepts only furniture kinds used by that template.
Each kind can constrain exact `material: [type,index]`, `subtype` and
`max_distance`. Unspecified material/subtype means any; distance defaults to
65,532. Every slot retains these constraints in the existing allocation request.
The entire multi-area request enters one global allocator, so a generic area
cannot consume the sole compatible item required by a constrained area.

`excluded_regions` contains at most 32 exact `{"origin":[x,y,z],"size":[w,h,l]}`
cuboids. None may intersect proposed excavation targets, including corridors and
doorways. This is a **target-mask exclusion**, not a host lease, native shared-block
scheduling fence, or guarantee that existing terrain will remain unchanged.
No whole excluded volume is expanded. Overlaps between proposed excavation parts,
or excavation of another bedroom's separating wall, refuse the complete recipe.

## Deterministic artifacts and geometry evidence

```python
from room_provisioning import RoomPlan
plan = RoomPlan.from_request(request_bytes)
result = plan.json()
# Existing consumers accept these independent artifacts:
blueprint = result['excavation_blueprint']
furniture_request = result['furniture_request']
```

Area order is canonical by name; slots reuse the existing request's normalization.
Each unit retains its intended footprint, doorway and slot names. A cardinal
connectivity traversal treats every planned furniture tile as blocked, checks all
remaining intended floor from that area's entry, and selects a deterministic
adjacent approach to every furnishing. Its digest binds the intended floor,
entry and approaches. This proves **geometry only**: it does not inspect terrain,
prove native pathfinding, or connect the layout to an existing fortress.

The complete plan digest binds the normalized room intent and all emitted data.
`RoomPlan.decode` regenerates the recipe from the original intent and compares
every canonical byte. Replacing targets, constraints, output digests, geometry
claims or derived counts without matching the original intent is refused.
Returned JSON objects do not alias retained plan bytes.

## Limits and downstream execution

Whole-request limits are 8 areas, 32 furniture slots, 32 excavation parts,
512 excavation tiles and one bounding capture of at most 1,024 cells (each axis
at most 128). An area allows up to 10 bedrooms or 16 dining pairs, but global
limits still apply. Larger dimensions/counts cannot silently drop rooms or use a
bounding-box excavation. Input and normalized intent are at most 16 KiB; the
complete plan is at most 48 KiB. All target coordinates leave the existing native
halo inside 0..32767. Actual map dimensions remain unobserved.

Pass the excavation artifact to the existing `dig_blueprint_client.py` workflow;
its normal-mining compiler and per-step native review remain authoritative for
execution. Pass the furniture request to `allocate_furniture.py --with-handoff`,
then retain its handoff through the existing batch/2 placement and completion
workflow. Excavation, placement and completion are separate reviewed stages.
No step in this compiler automatically runs any later stage or assumes a stage
finished. Furniture completion alone does not establish room zoning, terrain
safety, wall preservation, access, or completion of the combined room project.

## Focused evidence

The 16 Python core tests execute the actual existing request, allocator and
furniture-plan codecs. They cover all 250 bedroom count/size combinations with
explicit whole-request refusals, all 64 dining count/column combinations,
independent cell-set geometry, cardinal access, mixed-area global scarcity,
complete shortages, closed fields, exclusion and wall conflicts, bounds,
canonical import mutation rejection and interruption at every guarded boundary
of the two-bedroom fixture. They do not execute Rust, native DFHack or gameplay.
The native dig compiler and live inventory CLI are separate integration checks.

The owning bridge beads `.3`, `.4` and `.5` remain open. The machine contract is
`architecture/room_provisioning_v1.json`. No production runner, protocol, journal
format, dependency pin or compatibility registry is changed.
