## 2026-10-16 — Whole-room quarter-turn geometry

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5` (partial feature progress, not closure).

Implemented `room_orientation` through the existing `RoomPlan` compiler, not
as a disconnected geometry demo. Exact quarter-turns transform all template
outputs before the original whole-project validators and furniture allocator.
Zero/default preserves legacy bytes; nonzero angles are retained and checked
by artifact regeneration. No native protocol or MCP surface was widened.

Executed 16 focused Python regression tests, all passing. The five original
compiler/dependency modules materialized for these tests were checked against
their Git blob identities before modification. Rust/native/live-game and the
complete repository suite were not executed in this environment. General live
MCP mutation admission and native room assignments remain outside this change.
