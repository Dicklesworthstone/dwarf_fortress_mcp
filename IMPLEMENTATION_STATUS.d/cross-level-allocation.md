# Cross-level furniture allocator

The Python route allocator can select other-floor furniture with explicit
max_vertical_distance from 0 through 127 and a complete observed stair route.
Default zero preserves prior same-floor request and allocation bytes. Material,
subtype, exclusions, 3D lower-bound and route limits remain independent checks.

Thirteen allocator tests execute actual production Python with synthetic map
bytes and inventory values: 81 stair pairs, 180 global-assignment oracle cases,
32-slot/127-level bounds, unknown terrain and legacy canonical comparisons pass.
This commit alone does not update retained reviews or placement frontiers.
No native transport/game, Rust/MCP, private parent or full-workspace evidence.
Beads df-dfhack-bridge-plane-c-pic.3/.4 and df-franken-storage-mvcc-54h.4.
