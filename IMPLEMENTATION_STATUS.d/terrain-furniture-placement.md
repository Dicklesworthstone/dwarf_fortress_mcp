# Terrain-backed placement and recovery: executed Python integration

Batch/4 now consumes TerrainFurnitureHandoff. It retains complete original room,
inventory and raw map evidence, and requires the original satisfied terrain
journal before initialization, review or advance. Whole-history replay and
subsequent custody checks use the same operation budget. The original effect
keys bind the full new manifest. Query/cancel/inspect/stop remain available when
the terrain file is missing and cannot authorize another placement.

Seventeen new methods passed in 27.139 seconds on Python 3.13.5, no skips. The
maximum case executed 32 exact placements with 646 excluded items retained after
the temporary handoff input was deleted. Real private journals, CLI subprocesses
and a joined fragmented furniture/1.19 fixture exercised placement, restart,
lost replies, lost terrain after preparation, post-effect publication refusal,
source changes and original-key recovery. Thirty canonical comparisons across
batch/1-/3 matched the exact prior source blob
553481212fe205df34bedfe273d1af972ca52ccb.

Source dependencies were verified against exact Git blob hashes. This remains a
local subset, not full-repository qualification. Terrain and inventory inputs in
the new placement tests were generated with real codecs and the private journal
writer, not acquired from a game. No Rust/MCP compilation, DFHack SDK/ABI/live
fortress or native room assignment was exercised. The batch/4 construction
monitor consumer is a separate increment. Production admission is unchanged;
broader bridge beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
