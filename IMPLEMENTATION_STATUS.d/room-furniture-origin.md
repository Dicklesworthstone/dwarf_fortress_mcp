# Original room intent in furniture allocations

`RoomFurnitureHandoff` retains the complete canonical RoomPlan alongside the
existing exact inventory allocation. It regenerates the room geometry, requires
byte-identical original furniture constraints, and binds room-only changes in a
separate composite identity. Native dimension checks cover the entire original
floor/corridor and required bedroom walls, not only furniture target halos.

Fourteen semantic test methods passed locally using byte-verified copies of the
existing compiler, allocator and handoff modules from parent c4b14b76c2051b257e656d25d9e5fa68a00196b3.
Tests include 32 slots/646 exclusions, whole-intent substitution, source and item
constraints, noncanonical input, hostile bounds and interruption at every decode
checkpoint. This increment does not yet connect the type to native batch custody.
No network, MCP, Rust build, live fortress or full qualification was executed.
The capsule does not retain or certify the terrain-monitor completion history;
it proves internal intent/allocation consistency only. No authority is granted.
Beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
