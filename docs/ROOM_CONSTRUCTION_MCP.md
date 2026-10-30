# Preserve a complete room intent through native furnishing

The furniture development server accepts a bounded room request containing
required floor cells, required wall cells and the complete existing furniture
request. It allocates all furnishings from one fresh operations/1.4 capture and
retains both the original geometry and every allocation constraint before any
placement. This supplies the original objective needed for subsequent joint
terrain-and-furnishing monitoring.

This is an explicitly unadmitted development extension of the existing eleven
MCP tools. It introduces no native method or mutation protocol. Configure the
server and private placement/batch files as described in
[the furniture MCP guide](BUILD_PLACEMENT_MCP.md).

## Open with the complete declared intent

Pass this JSON string as `room_request` to `fortress.open_session`:

```json
{
  "schema": "dfmcp.room-construction-request/1",
  "terrain": {
    "region": {"origin": [14, 14, 2], "size": [5, 4, 1]},
    "floors": [[15, 15, 2], [16, 15, 2], [17, 15, 2], [16, 16, 2]],
    "walls": [[14, 15, 2], [18, 15, 2], [16, 14, 2]]
  },
  "furniture": {
    "schema": "dfmcp.furniture-request/1",
    "world_folder": "region1",
    "site": 2,
    "slots": [
      {"name": "bed", "kind": "bed", "target": [15, 15, 2]},
      {"name": "chair", "kind": "chair", "target": [16, 15, 2],
       "material": [419, -1], "max_distance": 100, "after": ["bed"]},
      {"name": "table", "kind": "table", "target": [17, 15, 2],
       "after": ["chair"]}
    ],
    "excluded_items": [123]
  }
}
```

Coordinates, native material values and fortress identity above are illustrative.
The outer request has exactly `schema`, `terrain` and `furniture`; each nested
terrain object is closed. Duplicate keys, floating-point coordinates, unknown
fields, repeated coordinates and overlapping floor/wall requirements are
refused. The complete input is limited to 128 KiB and nesting depth 16. The
nested furniture object passes the existing strict decoder with its original
bytes, so duplicate fields or noninteger IDs cannot disappear during projection.

`selection`, `furniture_plan`, `furniture_request` and `room_request` are mutually
exclusive. Omit all four when reopening existing custody. Clock-configured and
monitor-only recovery profiles refuse new intent inputs.

The region has exactly one z-level, at most 4,096 cells and side lengths no
greater than 128. Required floors must be nonempty. All selected coordinates lie
inside the region, floor and wall sets are disjoint, and every requested
furniture target must be a required floor. Cells in neither set make no terrain
condition claim. The complete region must fit the operator's scope and the native
map dimensions returned by placement bootstrap.

The full furniture request retains its 1..32 slots, exact optional material and
subtype constraints, distance ceilings, dependencies and exclusions. Existing
allocation feasibility and current-authority checks apply to the whole request.
An allocation shortage returns the complete declared room and shortage evidence,
opens no session and creates no partial batch.

## Retain the same identity through placement and recovery

A successful intake stores a new `DFMFBD03` batch definition. Its unchanged
allocation handoff retains the full furniture request once; compact canonical
terrain masks extend that same definition. Decoding reconstructs the room intent
from the exact retained request. A geometry-only change produces a different
batch identity and different original placement keys, even if every selected
item and furniture target is unchanged.

Existing `DFMFBD01` exact-item and `DFMFBD02` allocation-backed batches keep their
original bytes, digest domains and keys. They cannot acquire a room objective
retrospectively by supplying geometry on reopen. A changed request cannot replace
an existing room-backed parent. The original definition survives stop, offline
inspection and reopening under the existing private-file custody rules.

Continue with the same explicit prepare, review, commit and reconcile workflow
used for furniture batches. Every placement still requires current native
evidence and all existing construction policy checks. Room intake neither
prepares a placement nor advances game time.

`result.batch.room` reports the full retained furniture request, its digest, the
room intent digest, the exact region and both complete terrain masks. The mask
encoding is `x-fastest-row-major-lsb0-hex`: flatten the region with x changing
fastest; cell `i` uses bit `i % 8` of byte `i / 8`. Each mask has exactly
`ceil(region volume / 8)` bytes and unused high bits are zero. The projection
contains every selected cell without expanding thousands of coordinate rows.
The complete projected result is reserved before new custody is created.

Canonical terrain is bounded by 1,058 bytes, the standalone full room intent by
17,458 bytes and the combined batch definition by 43,098 bytes. Maximum requests
therefore remain inside the unchanged 64 KiB parent store and existing complete
construction-origin bound.

## Evidence scope

Intake reads furniture inventory and preserves declared geometry. It does not
observe terrain or establish room completion. A native `Placed` receipt proves
the recorded placement outcome; ordinary furniture completion concerns the
original furnishings and does not prove all retained room terrain requirements.
The explicit room projection keeps those distinctions visible.

This narrow Rust request generation is separate from the Python `RoomPlan`
format. It does not import or discard that format's routing fields, compile a
layout, designate terrain, create a native room assignment, prove pathfinding or
certify structural safety. The owning bridge beads remain in progress pending
their broader functionality and qualification work.

Focused checks exercise canonical masks, complete 32-slot requests, the 4,096-cell
bound, constraint retention, original keys, exact legacy compatibility, shortage
behavior and real private-parent reopening. See
[implementation status](../IMPLEMENTATION_STATUS.md) for the exact executed
component evidence and the remaining runtime/native/live qualification limits.
