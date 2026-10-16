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
