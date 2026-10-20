## Automatic layout integration for bounded-cost corridors

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. These broad beads remain open.

The existing bounded-cost connector was not selectable by automatic layout:
normalization rejected its strategy. Layout now admits the explicit strategy
only with a full binary cost objective, lowers the exact strategy into RoomPlan,
and assigns a distinct replay policy. Existing ranking, residual partition,
whole-project constraints, budgets and strict CLI exports remain owners.

33 warnings-denied Python tests pass: 15 new layout/compiler/allocator/CLI cases
and 18 unchanged cost-connector cases. A 499-tile original project with eight
7-by-7 bedrooms and two dining halls selects a 13-tile, one-cost corridor rather
than the original five-cost corridor. All 28 furnishings remain and allocate.
A separate layered state graph independently confirms (cost=1, added=13).
An eight-area/32-item case, cancellation, label exhaustion after a baseline,
exact work bounds, all six request/replay CLI exports, tampering and process
hash seeds are exercised. Eighty-four baseline/final legacy layout results
remain byte-identical by exact byte count and SHA-256, including trace and work.

Baseline layout, observed wrapper and connector were matched to fetched Git
blobs c66ef787a5848b26f5710c7cac2b66db14d13a5f,
534922abaae01f8637e22a286d5c88cae958c297 and
e8a44cb00bb34039e28e6636a8531b236492cb4d. The bounded-path source matches
98b103edc39ed572c9d689e3ba225e7343f87431 and is unchanged.

This increment tests offline Python components with synthetic geometry. Observed
map integration, native transport, room-backed execution, Rust/DFHack builds,
full-repository and live qualification are not claimed by this increment.
No native method, journal, admission or mutation permission changes.

### Observed workflow and whole original-room handoff

The observed normalizer now admits both existing cost strategies and restores
the EXACT original selection after preflight. Costs still derive solely from
original decoded map evidence; caller-authored cost maps refuse before contact.
The selected complete target union is independently recounted, and the original
padded compiler repeats terrain/capacity validation before handoff creation.
Neither native reader, CLI implementation, connector nor handoff codec changes.

The synthetic full-project result has 511 observed floor targets and one wall,
retaining eight bedrooms, both dining halls, all 28 furnishings and 512 total
targets. Padded compilation, the one-wall/one-part residual and the complete
RoomExcavationHandoff all replay through their actual existing codecs. A separate
maximum-eight-area/32-furnishing layout test passes. Explicit entrance and
cross-level furniture selectors remain unchanged through map-backed lowering.

93 warnings-denied tests pass on the final upload source in completed bounded
invocations: 15 new layout tests, nine supplied-map/padded-handoff tests, seven
actual TCP/CLI tests, and 62 unchanged solver/compiler regressions. Fifteen joined
synthetic peers exercise the actual MapClient and real CLI subprocesses, all
seven request/saved exports, source failures, closed-before-search lifetime,
label exhaustion after a valid baseline, authorization replacement after final
serialization and single-attempt output. No replacement runtime solver, client,
codec or extracted function body is used. Fault hooks wrap actual boundaries.
An aggregate rerun hit the execution time limit and is not counted as completed;
the complete final evidence is retained in three successful bounded runs.

Independent baseline/final interpreters preserve 244 legacy output byte counts
and SHA-256 values: 84 offline layouts, 64 observed surveys, 64 reports, 16 padded
compilations and 16 handoff outcomes. These cover trace/work counts and exact
old strategy behavior. Hash checks compare uploaded runtime/test files with
executed bytes. The unchanged padded compiler is
07966157544d15f705599bc1d0caeba4235a8fb1; the surveyed CLI is
15a1b7658d32bdc1126c4efcec9612426e02d537. No existing tests are replaced.

This adds supplied-map, padded compiler/handoff API and surveyed TCP/CLI evidence.
Padded excavation CLI, native mutation dispatch, durable room-batch execution,
real DFHack/Rust builds, physical safety, live fortress and full-repository
qualification are not claimed. Native methods, journals, original-key recovery,
mutation authority and production-admission restrictions remain unchanged.
