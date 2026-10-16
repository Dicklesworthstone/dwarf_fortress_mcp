## 2026-10-16 — Connect original room projects to an explicit access point

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5`; broader work remains open.

The existing RoomPlan compiler now admits one point plus a bounded same-level
routing domain when whole-project connection is explicitly requested. It uses
the original connection algorithm, retains the point in the original floor
goal, partitions only selected corridor cells, and keeps the downstream capture
and mask digest tied to the exact tight target hull. The root is witnessed even
when no corridor is needed. No area, furnishing or original constraint is lost.

Evidence: all 16 new Python component tests pass, including actual global item
allocation, connected eight-area projects, all four bedroom rotations, walls,
blocked roots, exclusions, site constraints, exact semantic digests, deterministic
process replay and interruption at every guard in the small fixture. All 318
legacy input outcomes were compared against the identity-checked original
compiler and connection modules: 314 accepted artifacts and four refusals are
unchanged. Corpus outcome SHA-256:
`60a05d1d8ec313ce63e76b40f00b878ab93cbdb5af51f5fa47c1c578ccd72869`.

The six existing Python modules used for the component run match their fetched
Git blob identities before editing. The local harness contains those components,
not a full repository checkout. Rust tooling and live DFHack were unavailable;
full verification/qualification was not executed. No existing-fortress-access,
native acquisition, mutation authority, compatibility or production-admission
claim is added. The selected point remains an offline assumption in this slice.

### Automatic search and original-map entrance evidence

The existing layout solver now retains a fixed access point, uses the complete
caller domain for corridor routing, and rechecks entrance-to-room connectivity
before accepting a full candidate. It backtracks when furniture obstructs the
point or a route fails; required walls cannot occupy the point. Router-specific
work exhaustion is distinct from infeasibility and propagates as WorkLimit.

The existing observed-layout wrapper verifies visible/dry/undesignated/
unoccupied floor directly from the one original map/1.5 payload, before caller
site-mask intersection. The resulting historical point evidence binds the same
map witness and tick as layout. It never proves existing-fortress membership,
native pathfinding or current access. Pure room-plan observation flags stay
false; offline replay does not claim new acquisition.

Final evidence: 39 scoped methods pass (16 core, 17 search/raw-evidence, 6 TCP/CLI).
The latter execute 18 joined synthetic-peer sessions using the unchanged full
MapClient and both existing CLIs. They exercise all seven offline survey exports,
stream closure before search, 10 unusable entrance states, lost replies, source
confusion, and authority revocation after capture. There is no native DFHack
producer or live-game result. The complete local run remains component-only.

A separate 112-request corpus compared both layout and observed-layout outputs
against their exact fetched original modules; all 224 artifact/refusal outcomes
are unchanged, including oriented requests. Outcome corpus SHA-256:
`d331ee5d74a4036457c9f84bf1d6314c1772749b30e38d9f8634c0ab93619fc8`.
