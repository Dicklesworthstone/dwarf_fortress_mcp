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
