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

Follow-up: `room_layout` now expands each area's explicitly allowed quarter-turns
into the same MRV search variable, bounded by the original shared work/node
allowances. It lowers the complete selected project through `RoomPlan`, retains
the original allowed set, and binds nonzero selected angles into the trace and
sealed plan. Old requests preserve byte-identical outputs and work counts.

Validation: 50 Python tests pass (16 template-orientation, 18 new global-layout
orientation, 16 existing layout cases). Coverage includes a two-hall site that
requires different angles simultaneously; complete 16-slot allocation; 64 new
exhaustive obstacle masks; 256 existing obstacle masks; bounded cancellation;
legacy goldens; process hash-seed replay; ten actual CLI solve/replay artifact
exports; and CLI refusal to export an inconclusive partial result. The unchanged
CLI and original layout tests were verified against their fetched Git blob
identities before execution. The existing layout negative-angle test was updated
from now-admitted 90 to still-forbidden 45; no test was removed or ignored.

This does not close the broad spatial-planner/native-mutation beads. The observed
map wrapper's delegation was reviewed, but live map acquisition, Rust/native
builds, full-repository qualification and production admission were not tested.
