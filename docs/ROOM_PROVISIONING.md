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

## Executable compiler and inventory handoff

The new `scripts/plan_rooms.py` command exports canonical artifacts without
creating any journal or contacting a bridge in `compile` mode:

```sh
python3 scripts/plan_rooms.py compile --request-file rooms.json > room-plan.json
python3 scripts/plan_rooms.py compile --request-file rooms.json --emit excavation > excavation.json
python3 scripts/plan_rooms.py compile --request-file rooms.json --emit furniture-request > furniture-request.json
```

Compile succeeds with exit 0 and writes one exact standalone artifact, without
a trailing newline, so `room-plan.json` can be imported byte-for-byte. Other
commands do not accept compile-only export flags. Each input is a bounded regular
file opened without following the final symlink; descriptor and path metadata
must stay stable throughout the read. This is operator input, not private journal
custody or all-parent no-follow traversal. Oversize, special, replaced, duplicate
JSON, malformed or conflicting requests fail before native contact.

To select the actual complete item set, use the existing isolated inventory
configuration from `FURNITURE_ALLOCATION.md`: exact
`DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION=1`,
`DFMCP_OPERATIONS_PAGED_TOKEN`, and optional numeric-loopback
`DFMCP_FURNITURE_ALLOCATION_ENDPOINT`. No other DFMCP variables are accepted,
including furniture placement credentials and production admission settings.

```sh
python3 scripts/plan_rooms.py allocate --plan-file room-plan.json > room-allocation.json
# Alternatively, compile and allocate the original request in this same call:
python3 scripts/plan_rooms.py allocate --request-file rooms.json > room-allocation.json
```

An imported plan is regenerated from its complete original intent before any
socket is opened. The existing inventory client binds only operations/1.4
Handshake and ReadObservation. It acquires and verifies all pages of one capture,
including its digest and release acknowledgement. The existing projection decodes
that complete inventory once for global allocation and handoff derivation. It
does not call a placement profile, reread the game or reserve items.

The response uses profile `room-provisioning/1` and the existing bounded allocation
Agent Turn. An allocated result retains its exact `plan` and `handoff`, plus
`room_provisioning`, a reproducible recipe summary with original intent and plan
identity. That summary omits the duplicate furniture request: it is not the full
standalone `--plan-file` artifact. Retain the original `room-plan.json` for later
recipe import. The full derived request remains inside the handoff.

Both `ok` and `result.status` matter. A valid shortage returns exit 0, status
`shortage`, no handoff, no executable furniture plan and no partial assignment;
all room intent remains visible. A refused input/read/source/authority/budget
returns exit 2 with result null, no cached inventory or partial plan, and no
credential, native text or caller path. The unallocated excavation proposal in
a recipe is never permission to mine, including when furnishings are short.

The complete result must fit 65,536 bytes. Compilation, input reads, native calls,
projection, optimization and serialization share one shrinking 1..60,000 ms wall
allowance (default 10,000), 272 calls, 20 MiB native bytes, 1 GiB input bytes and
20 million work checks. Native pages and room areas cannot renew that allowance.
Current operator authority is checked throughout room revalidation and CPU
projection, and again after the entire final result is serialized. Failures never
trigger an automatic reconnect. These are cooperative checks, not hard realtime.

Export only an allocated result's `handoff` with the existing canonical codec,
then use the separate furniture batch environment and its normal fresh
review/prepare/one-shot-commit workflow. The handoff retains every derived item
slot and material/subtype/distance constraint; it does not carry native room
zoning or grant a terrain effect. Native placement, journal, completion and Rust
interfaces are unchanged. The room manifest is separate operator data, not a
new cross-stage project journal or combined room-completion certificate.

### Executed integration scope

The actual compiler and allocator subprocesses passed 16 additional tests against
the unchanged existing inventory TCP client, strict roster codec, global
allocator and handoff codec. All 16 core tests were rerun too. A 32-slot recipe
(eight bedrooms and four dining pairs) used one 2,032-item, two-page, 95,622-byte
capture and returned a complete 23,028-byte response. The maximum-width 32-slot
source/name/material fixture returned 34,109 bytes. These are fixture sizes, not
universal performance claims.

The tests cover full shortages, recipe/handoff substitution, missing/corrupt
release, wrong page offsets/digests/profiles, lost reads, foreign fortresses,
malformed full rosters, bounded files and caller configuration, shared budget
exhaustion, final serialization revocation, future selected-item constraint
checks, and the unchanged default inventory command. Four independently weakened
implementations were rejected by regression assertions: final authority omitted,
recipe revalidation omitted, request-digest binding omitted, and required bedroom
wall checks omitted. Exact source/log hashes are recorded in
`docs/evidence/room-provisioning-python.json`.

The TCP peer and captured inventory are explicit fixtures, not native DFHack.
This session did not execute the existing dig compiler/designation client, batch
placement process, completion monitor, Rust/MCP, real SDK or live-game campaigns.
It verifies the room-to-inventory-handoff boundary, not those downstream stages
or production qualification. All existing source dependencies used in these
Python tests were verified against their Git blob hashes.
