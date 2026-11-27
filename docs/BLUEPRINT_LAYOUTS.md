# Bounded connected blueprint layouts

`BlueprintPlanner::layout` and `BlueprintLayout::generate` implement the pure
`dfmcp.blueprint-layout/1` geometry model for WP-PLN-01. They return disjoint,
deterministically ordered excavation cuboids, their exact summed tile count,
and optional access/crossing geometry. They neither inspect nor mutate a fortress.

Bedrooms use up to four rooms per row. Each room retains separating walls and has
one south doorway. Row corridors join a west spine; `access_point` identifies its
first endpoint. Workshops use the same access arrangement in one row of 5x5 bays.
The endpoint is not proof of connection to an existing fort, and a doorway is an
excavation opening, not furniture. Dining halls and stockpile vaults remain single
rectangles. Stockpile category is a bounded label, not a configured filter.

A defensive moat is a one-tile perimeter on a single z-level, not an excavation of
its enclosed interior. Its input must span at least 3x3 tiles. `drawbridge_span`
leaves a centered unexcavated crossing on the north/min-y edge while preserving
both corners; an odd remaining tile goes east. The returned `reserved_crossing`
is geometry, **not a resource reservation or a constructed drawbridge**. Origin is
used by room templates; moats use the explicit absolute `perimeter_cuboid`.

Limits apply before terrain scanning: 1..24 rooms/bays, 64 disjoint parts and
16,384 excavated tiles. Category labels are at most 128 UTF-8 bytes without NUL.
Coordinates use checked arithmetic. The intent compiler additionally preflights
at most 131,072 summed halo tile visits, including repeated halo work. It checks
all parts, including access corridors, before returning any intent.

The legacy spatial index's magma and span checks are deliberately limited: they
do not establish aquifer, water pressure, load-bearing support, unit access,
hidden geology, or overall safety. Its `Safe` enum variant means only that those
listed checks found no hazard. No live compatibility or admission is added.

The excavation compiler seals `RegionTerrain` predicates for every excavated
part. Geometry success, a retained plan source and an immediate acknowledgement
do not discharge excavation work: a later observation must establish the target
terrain. These are laboratory predicates; this path adds no native writer,
bridge protocol, top-level MCP tool, or mutation authority.

## Furnished laboratory plans

Ordinary laboratory `fortress.plan` blueprint requests now seal the
`furnished_blueprint_v2` compiler identity in their retained source. Bedrooms
require one completed bed in each room; dining halls require a completed table
and chair. Their original goal requires all excavation plus the exact generated
building identities, kinds, positions, footprints and completed construction
stages. Finishing excavation alone cannot mark a furnished goal achieved.
An ordinary `workshop_hub` can also construct production facilities. Supply one
registered workshop per bay in west-to-east order. This example creates a still,
kitchen and carpenter's workshop in the laboratory.

```json
{
  "template": "workshop_hub",
  "origin": [2, 6, 10],
  "bays": 3,
  "workshops": ["workshop:Still", "workshop:Kitchen", "workshop:Carpenters"]
}
```

Each build follows excavation of its own bay and occupies the middle 3x3 tiles
of a 5x5 room. The original goal requires all excavations and every specified
workshop's exact identity, kind, location, footprint and completed stage.
Repeated workshop kinds create distinct facilities. The list accepts 1..20
entries within the 64-action limit. The ordinary planning checks for guarded
construction, active material models and shared labor apply. Production orders
remain separately reviewed actions.

Requests with a workshop list retain the `equipped_workshop_hub_v1` compiler
marker. Omitting the list retains the excavation-only workshop template. A
stockpile vault continues to request excavation for the named category.

New ordinary requests leave synthetic step deadlines to the planner. On a
`shared-workforce/1` scenario, its conservative deadline floor includes queued
work and all planned service, while dependent furniture deadlines also account
for their excavation prerequisites. This removes the fixed 1,000-tick mining
deadline that could expire before a large project received enough worker time.
Explicit caller obligations elsewhere keep their original deadline policy.

Furniture footprints must be distinct and inside the excavated room. A
one-tile-wide dining hall uses its north/south axis; a 1x1 hall is refused before
preparation because it cannot fit both furnishings. This compiler still uses
abstract laboratory construction. It does not select or consume furniture
items, prove native access, or assign room ownership; the separate provisioned
and connected-room compilers own their stronger contracts.

Already retained, unversioned ordinary blueprint sources reconstruct the prior
compiler exactly, including excavation-only goals and fixed excavation
deadlines. Recovery never silently strengthens their original goal or changes
their reviewed plan. Public intake stamps new requests with the current marker,
accepts that same marker idempotently, and refuses legacy, unknown, null or
duplicate markers. Candidate comparison uses the same intake path as planning.

## Validation

The Rust geometry suite checks exact perimeter-minus-crossing sets for 1,666
small moat configurations, connected/disjoint bedroom layouts for 1,176 cases,
24 workshop configurations, preserved walls, deterministic output and malformed,
oversized and coordinate-boundary inputs. Existing compiler tests retain magma,
missing-coverage, span and zero-dimension checks. Furnished-plan regressions cover
excavation without furniture, incorrect or missing furnishings, narrow halls,
queued work with a shared worker, current intake and exact legacy reconstruction.
The MCP regression exercises ordinary plan, commit and wait through the public
facade. Test execution and qualification evidence remain recorded in the owning
Beads and implementation status; laboratory tests are not native DFHack or
live-game qualification.

## Coherent spatial query

The existing read-only spatial `fortress.query` handler accepts
`blueprint_layout`. It uses the same captured terrain and source anchor as the
other spatial queries; it does not acquire another native frame. This is a query
preview, not `fortress.plan` or `fortress.commit`.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "blueprint_layout",
    "origin": [10, 10, 5],
    "template": {
      "kind": "bedroom_cluster",
      "rooms_count": 20,
      "room_size": [3, 3]
    },
    "limit": 8
  }
}
```

The five template variants and their closed inputs are published in
`schemas/mcp_spatial_blueprint_v1.json`. Input coordinates are 0..32767. Derived
corridors outside the map are reported explicitly, not clamped or declared safe.
Moat templates use absolute `min`, `max` and `drawbridge_span` fields; `origin`
is still required by the common request but does not translate the moat.

Each row reports its disjoint cuboid, role, dig mode, tile count and native
coverage. The full summary distinguishes visible, hidden, unallocated,
outside-capture and outside-map positions, both over the footprint and the unique
one-tile three-dimensional halo. Only visible cells contribute shape, liquid,
occupancy and existing-designation counts. A reserved moat crossing has separate
coverage and is never counted as excavation. `all_positions_visible` describes
coverage only, not hazard absence or suitability.

`max_work` bounds conservative cell visits, including map validation, per-part and
total footprint reads, repeated halo insertions, halo reads and crossing reads.
The hard ceiling is 262,144. Pages are whole-row and byte/token bounded; their
continuations bind session, anchor, source, origin, policy and analysis. Page
width may change without reallocating geometry. Changed captures or requests
require restarting the query.

Every result keeps `plan_created`, `commit_compatible`, `reservation_created`,
`safety_proven`, `excavation_eligibility_proven` and `completion_proven` false.
Aquifers, water pressure, structural support, protected areas and native unit
access remain unknown. No archive whitelist or new bridge write is introduced.

Additional Rust regressions cover presence accounting, missing vertical halo,
out-of-map corridors, separate crossing coverage, work bounds, schema discovery,
authority and stale anchors, input smuggling and complete-envelope pagination
through the live spatial handler. These Rust tests are not executed in this
environment. The JSON Schema was independently validated in Python: five valid
templates accepted and fifteen malformed/adversarial requests rejected.
