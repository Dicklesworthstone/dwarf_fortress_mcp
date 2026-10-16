# Whole-request automatic room layout

Source and actual Python component evidence: `room_layout.solve` chooses all
room origins together under bounded deterministic backtracking, then delegates
the complete connected plan to the existing RoomPlan compiler. It preserves
hard exclusions, required walls, material/subtype/distance constraints and every
area. Fixed origins and constrained origin regions are supported. Inconclusive
search budgets never claim absence or return a partial plan. Canonical import
repeats the complete original search.

All 16 tests pass using the real compiler, router and allocator: 256 obstacle
masks, 24 exhaustive two-area domains, backtracking, complete group constraints,
maximum eight-area/32-slot requests, work/node limits, all guarded interruption
points in a representative case, canonical import and process-seed equality.
Four unchanged dependency files were matched to their repository Git blobs.

No native game, Rust/MCP, full repository or CLI execution is claimed by this
increment. Geometry and focus remain planning assumptions, not observed terrain,
a fortress entrance, native pathfinding or permission to excavate. Broad bridge
beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open; WP-PLN-01 owns planning.
