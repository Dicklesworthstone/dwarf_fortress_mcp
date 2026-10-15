# Whole-project room connectivity (2026-10-15)

Owning beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10).
The corridor engine is now wired into RoomPlan compilation and canonical import.
An explicit connect_areas=true retains every original area, connects its intended
walkable floor within the fixed same-level capture, and places every added cell
in the existing excavation blueprint. Walls, furniture and excluded cells remain
protected. Unreachable or over-budget projects refuse without a partial plan.
The complete plan/terrain consumers retain the new mask through their existing
RoomPlan path; those downstream native/CLI workflows were source-reviewed only.

Executed: 27 Python unittest methods, zero failures or skips. These run the actual
router, compiler and furniture allocator, including 128 barrier-mask path cases,
512 exact-partition masks, all guard interruption boundaries in representative
fixtures, separate-process hash-seed equality and maximum area/slot cases.
A pre-change blob-verified baseline covers 314 legacy requests: 310 successful
artifacts remain byte-identical and four retain refusal. Explicit false preserves
legacy artifact bytes too. The checked-in mixed example retains 12 furnishings,
adds five corridor cells and yields 64 total floor cells in eight parts.

No native game, SDK, CLI/TCP pipeline, Rust/MCP or full-repository qualification
was executed. This is intended geometry, not observed terrain, native pathfinding,
room assignments, construction completion or mutation authority. No native method,
production runner, dependency or compatibility entry changed. Broad beads remain
open. Contract: architecture/room_connections_v1.json.
