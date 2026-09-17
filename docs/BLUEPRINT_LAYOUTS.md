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

Intent and obligation terminal predicates remain `False`, because this path has
no registered excavation-completion witness. Geometry success, plan source and
an immediate acknowledgement must not discharge excavation work. No native
writer, bridge protocol, top-level MCP tool, or mutation authority is added.

## Validation

The Rust geometry suite checks exact perimeter-minus-crossing sets for 1,666
small moat configurations, connected/disjoint bedroom layouts for 1,176 cases,
24 workshop configurations, preserved walls, deterministic output and malformed,
oversized and coordinate-boundary inputs. Existing compiler tests retain magma,
missing-coverage, span, zero-dimension and false-completion checks where present.
Rust/Cargo/rustfmt are unavailable in the editing environment: these tests have
not been compiled or executed there. Independent Python geometry checks are
reference evidence only, not Rust, MCP, native DFHack or live-game qualification.

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
