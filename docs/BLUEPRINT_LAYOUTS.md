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
