# Choose a complete connected room layout

The offline `room_layout.solve` search accepts the existing bedroom and dining
recipes without manually specified origins. It places the entire request inside
one explicit same-level region, preserving furnishing constraints, excluded item
IDs, protected target regions and optional per-area origin constraints. Every
complete candidate is compiled by the existing `RoomPlan`, including its exact
corridor router, wall rules, circulation checks and output bounds. No alternate
template or mutation compiler is introduced.

## Request

Use schema `dfmcp.room-layout-request/1`, a `world_folder`, `site`, a `region`
(`origin` and `size`), and 1..8 named `areas`. Areas use the existing `template`
and optional `item_constraints`. Unlike a provisioning request, their `origin`
is optional. An area may instead declare an `origin_region` restricting its
allowed origin positions. Specifying both refuses. Full floor and required wall
geometry must fit the outer region, not merely the selected origin.

`focus` optionally names a point inside the region. It is a geometric preference,
NOT an observed fortress entrance. Candidate entries nearer it are tried first.
`excluded_regions` retains the existing target-mask exclusion meaning: no
excavation enters one; a required unexcavated wall may lie inside it. Excluded
volumes are intersected with the bounded search grid, not expanded in full.
All chosen areas are connected; multi-level placement, rotation, stairs and
connection to an existing fortress are outside this generation.

## Deterministic whole-request search

The current compiler derives each template once. Translated floor/wall bitsets
permit bounded compatibility checks without cloning a full plan for each origin.
At every step, search selects the remaining area with the fewest compatible
origins; ties use ASCII area names. Candidate order is entry-to-focus Manhattan
distance, then y/x/z origin. Backtracking removes tentative choices when a later
area cannot fit or the connected compiler rejects a complete arrangement. It
never drops an area or weakens a constraint to manufacture a successful prefix.

The first full compiler-valid layout is returned. This is NOT a global packing,
travel-distance, excavation or corridor optimum. The existing corridor compiler
still owns routing within the complete placement's bounding capture.

The result retains the normalized original request, complete lowered `RoomPlan`,
candidate counts, search nodes, backtracks, compiler refusals and a deterministic
trace digest. Digests are comparison identities, not signatures or authority.
`Layout.decode` reruns the original search and compares all canonical bytes.
Changing only the chosen coordinates, constraints, counters or result labels is
not a valid import. A lowered plan is a frozen chosen geometry; it does not cause
automatic re-layout during subsequent allocation, excavation or monitoring.

`layout_found` contains one complete plan. `no_layout_in_domain` means every
translation combination in this finite model was exhausted or soundly pruned,
with no plan accepted by the existing bounded compiler. It is not global
infeasibility or a native safety claim. `search_limit_reached` is inconclusive
and contains no plan. Cancellation or aggregate work exhaustion raises without
returning any partial plan. Caller exceptions are never treated as rejected
geometry and swallowed by backtracking.

## Bounds and evidence

Limits remain 8 areas, 32 furnishings, 32 excavation parts, 512 target floor
cells and a same-level region of at most 1024 cells (128 per axis). Each area has
at most 1024 translated candidates, 8192 across a whole request. Node budget is
1..100000 (default 10000); aggregate search/compiler work is 1..20000000 (default
2000000). All template compilation, candidate enumeration, compatibility checks,
and final compilation share that work allowance and the caller's guard. Input
parsing has a separate fixed 16 KiB/10-depth bound and invokes the same guard.
Results fit 96 KiB. Search does not acquire native or file authority.

All 16 core tests execute the actual solver, compiler, corridor router and
furniture allocator. They cover 256 obstacle masks against an independent
cell-set oracle, 24 two-area domains against exhaustive origin pairs,
backtracking, exact full-wall and exclusion preservation, eight areas/32 slots,
constraint-retaining allocation/shortage, node/work bounds, every guard
interruption for a representative request, import mutations and independent
process hash seeds. The four unchanged imported modules match their Git blob
hashes. No producer doubles, native DFHack, Rust/MCP, live campaign or full
repository qualification is implied.

Owning beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` and the still-open
bridge integration work `df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/WP-10).
