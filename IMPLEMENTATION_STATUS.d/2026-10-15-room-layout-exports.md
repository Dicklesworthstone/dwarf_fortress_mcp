# Executable automatic room layout exports

`layout_rooms.py` now accepts complete room intentions without manual area
origins and exports the existing RoomPlan, provisioning request, excavation
blueprint and furniture request. Saved layout imports regenerate the original
bounded search before any export. The optional report carries an offline Agent
Turn; no native source, credential, effect or automatic next stage is acquired.
Whole-request bounds and one foreground file/search/output deadline remain.

All 31 methods pass across four scoped runs: 16 actual solver/compiler/router/
allocator tests, 13 real-file/subprocess CLI tests and two schema/runtime tests.
Exports round-trip through unchanged existing codecs, retain every material and
distance constraint, and support whole-group allocation or explicit shortage.
Eight-area/32-furnishing requests fit every export. Tests cover finite versus
inconclusive search, canonical regeneration, final-input replacement, symlinks,
FIFOs, output deadline/short writes, redacted errors and no network/child-process
effects. The structural schema rejects 21 malformed variants. All 256 tiny-grid
obstacle masks include feasible and infeasible cases; 24 two-area dimensions
are independently enumerated. Four unchanged dependency blobs match GitHub.

The aggregate test command exceeded the tool's per-call execution window during
initial attempts; all methods subsequently completed in the bounded scoped runs.
No native inventory CLI/TCP, designation, room-monitoring, Rust/MCP, SDK/live or
full-repository qualification was performed. The four real compiler dependencies
are present in this working checkout; the full repository could not be cloned.
The selected layout remains intended geometry, not observed safety or existing
fort access. Broad bridge beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
