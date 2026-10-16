# One-source room layout and excavation handoff

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` and
`df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open.

`room_layout_excavation` composes the existing automatic room solver, full
terrain survey and original-room excavation handoff. It accepts the existing
`dfmcp.room-layout-request/1`, preserving orientations, fixed origins, entrances,
site masks, exclusions and furniture constraints.

`prepare(request_bytes, guard)` validates the complete request and selects the
original domain padded one tile in all three dimensions. The caller acquires
that exact region through the existing map/1.5 reader, closes the reader, then
calls `solve(request_bytes, capture_bytes, manifest, endpoint, guard)`. The
result is a replayable compilation and an optional existing
`RoomExcavationHandoff`. No additional map read is needed.

Before search, walls with missing, hidden, liquid-bearing or designated
neighbors become wall-only cells. They may remain required room walls but may
not be selected for excavation. Already-open floors have no new mining duty.
This reuses the terrain reducer's conservative context checks; it does not
establish aquifer, pressure, collapse, pathfinding or native mining safety.

The compilation retains the original padded payload, source witness and SHA-256.
Its two subregions preserve native tile bytes and source metadata, rewriting
only the region and count. Every subregion explicitly references its parent.
Missing/hidden records are never synthesized as known tiles. A derived payload
is not an additional native acquisition or independent acquisition attestation.

The selected complete RoomPlan is stored once at `layout.room_plan`.
`terrain_survey` omits that duplicate field; its existing survey digest applies
to the full record with that exact RoomPlan restored. The optional handoff
contains the same original plan and only the remaining wall targets. Existing
floors and preserved room walls never enter that remaining mask.

`replay(compilation_bytes, guard)` regenerates projections, constraints, layout,
terrain and handoff, then compares every canonical byte. A recomputed outer hash
does not make altered derived outputs valid. No layout, node-limit exhaustion,
blocked terrain or empty remaining work produces a handoff. Empty work means
sampled terrain shapes, not completed rooms.

Limits: 16 KiB request, 1,024 domain cells, 4,096 padded cells, native axes at
most 128, 82,495 parent payload bytes, 512 KiB compilation, JSON depth 16 and
the existing at-most-20-million work allowance. Padding is never clipped or
split. One enclosing work/deadline guard covers every stage.

The legacy handoff is compatible with `dig_blueprint_client.py init
--room-handoff`; that consumer still requires its own native review,
confirmation, original-key custody and recovery. Exporting only the legacy
handoff retains selected room intent and derived survey bytes, not the parent
acquisition chain; retain the full compilation for that chain. No artifact
resolves uncertain effects, authorizes replacement keys or changes admission.

## Executed component checks

```sh
python3 -m unittest discover -s tests -p 'test_room_layout_excavation.py' -v
```

25 tests passed using the actual existing decoder, compiler, solver, terrain
reducer, handoff codec and allocator. They cover all four bedroom rotations,
a complete eight-area/32-furnishing project, alternate safe placement, source
and derived-output substitution, exact residual masks, every truncation of a
small parent, unknown presence, cancellation and process hash-seed replay.
Twelve unchanged dependency modules were verified against their Git blob IDs.

These are component tests with synthetic map payloads. Network acquisition,
DFHack/Rust builds, actual digging, full repository qualification and live
production admission were not established by this first increment.
