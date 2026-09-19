# Observed terrain connectivity and single-point failures

Added `dfmcp_world::map_connectivity`: connected components, articulation tiles,
graph bridges, and exact within-component partition/impact counts for the existing
dry cardinal floor/complementary-stair route model. Iterative bounded low-link
DFS avoids recursion and returns no partial result on work/deadline refusal.
Hidden, unallocated and excluded cells never become connections. Results are
model-relative, not native unit paths, evacuation guarantees or mutation authority.

Eight Rust tests are registered, including exhaustive 3x3 vertex/edge deletion
oracles, differential existing-route checks, all vertical shape pairs, maximum
map size and long corridors. They are uncompiled/unexecuted: Rust/Cargo/rustfmt
are unavailable in this editing environment. The independent Python reference
passed 57,344 component/vertex/edge checks on 5,120 graphs; it does not execute
Rust or DFHack. No dependency, native protocol or admission change. MCP query
integration is a separate increment.
