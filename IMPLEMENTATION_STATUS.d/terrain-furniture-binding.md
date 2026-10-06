# Terrain/furnishing binding: executed Python integration

Sixteen methods in `test_terrain_furniture_handoff` passed with no skips on
Python 3.13.5. Tests use the real room compiler/allocator, private terrain journal,
whole-history replay, native map decoder, joined fragmented synthetic map peer,
and a real CLI subprocess. The maximum CLI case retains 32 slots and 646 excluded
item IDs. Source dependencies in this local subset were verified against their
exact Git blob hashes at base 79abc5777639d3c308ad38528ed1bc290c002844.

`bind_room_terrain.py` emits a separate closed handoff with the original terrain
journal identity and complete fresh raw map evidence. The origin reference is
not a proof: consumers must reopen and replay the exact original private journal.
The allocation input in these tests was generated locally, not acquired natively.
Placement/completion consumption is a separate increment; legacy readers refuse
this new format. No Rust/MCP, live DFHack/SDK, game effects or full-repository
qualification is claimed. Production admission remains unchanged, and broader
bridge beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
