## 2026-10-16 — Single-source layout and original-room dig handoff

Beads dfmcp-wp-pln01-spatial-blueprint-layout-pwd and
df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.

Source now composes actual existing automatic placement, terrain surveying and
RoomExcavationHandoff from one padded raw map/1.5 capture. Original bytes and
parent/child projection witnesses remain in the bounded replayable compilation.
A conservative halo-derived site constraint lets the solver avoid unsuitable
excavation while preserving required walls. Existing floor cells are not dug.
No partial project or handoff is returned for failure or inconclusive search.

Executed 25 Python component tests with twelve unchanged dependency files
verified against their fetched Git blob identities. Complete room compiler,
orientation/access routing, solver, terrain reducer, handoff and allocator code
ran; no stand-in producer functions replaced those implementations. The map
payloads are synthetic. Actual network/native/Rust/MCP/dig execution and full
repository qualification are not claimed for this increment. The production
registry, runner map, effect journals and native protocol remain unchanged.
