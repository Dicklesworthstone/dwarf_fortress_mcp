# Live and historical terrain connectivity queries

Wired `map_connectivity` into shared spatial query dispatch/schema discovery,
spatial/1.8 live historical reads, and archive-only current/exact-record reads.
Report named landmark membership, connected areas, articulation-tile partition
sizes and cut-edge side sizes; rank bottlenecks by separated tile-pair impact.
Preserve explicit unknown/excluded cells, full-anchor query fencing, whole-row
pagination, complete output budgets and existing active-watch/archive boundaries.

The source adds six query regressions and one multi-stage actual-handler scenario
to the eight world-layer tests. All 15 Rust tests are uncompiled/unexecuted because
Rust/Cargo/rustfmt are unavailable. The independent Python graph reference passed
57,344 checks over 5,120 graphs; the standalone schema passed 71 cases (19 accepted,
52 rejected). Neither reference executes Rust, MCP or DFHack. See
`docs/TERRAIN_CONNECTIVITY.md` for exact model, scope, commands and limitations.
No new dependency, native method, mutation authority, archive encoding or admission.
