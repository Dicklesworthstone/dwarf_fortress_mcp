# Sparse normal-mining blueprint batches

`scripts/dig_blueprint.py` compiles the **floor-only** subset of the existing
`dfmcp.excavation-blueprint/1` format to disjoint dig/1.16 rectangles. It preserves
unselected gaps, walls, room separation and levels; it never mines the enclosing
bounding box. Its geometry digest uses a separate execution domain. The original
blueprint semantic-mask digest remains compatible with the read-only monitor.

Run `python3 scripts/dig_blueprint.py blueprint.json` to inspect the complete
partition. Parts use `{"region":{"origin":[x,y,z],"size":[w,h,levels]},"shape":"floor"}`.
The top-level object is `{"schema":"dfmcp.excavation-blueprint/1","parts":[...]}`.
A shape goal is not native mutation permission: each resulting rectangle still
requires a fresh paused, eligible natural-wall observation and exact confirmation.
Already-excavated floor cells are NOT silently skipped or treated as native work.

The deterministic partition visits z/y/x order, takes up to eight consecutive
x-coordinates and then extends through up to eight complete selected rows. It
is an exact cover, not a claim of minimum rectangle count. Reordering or splitting
input parts without changing the selected mask preserves the compiled identity.
Overlapping parts, channels, ramps, stairs and wall-preservation goals are refused,
not silently interpreted as normal mining. Coordinates must leave the complete
one-tile three-dimensional native halo inside the supported coordinate range.

The existing monitor's limits remain: 32 parts, 512 targets, a bounding capture
of at most 1,024 cells with per-axis extent at most 128. The compiled batch must
also fit the existing non-evicting directory registry's 128 native intents.
All extent checks precede target expansion; no partial oversized plan is returned.

Validation: `python3 scripts/test_dig_blueprint.py`. Nine executed groups cover
all 4,095 nonempty 4x3 masks, 384 large rectangle sizes, multiple levels and room
gaps, semantic identity, unsupported modes, overlap, malformed JSON, native halo
edges, retention capacity and the actual compiler subprocess. These tests execute
Python; they do not establish Rust, DFHack SDK, live-game or production admission.

This compiler alone performs no native I/O. Beads
`df-dfhack-bridge-plane-c-pic.4` and `df-action-coordinator-exec-ero.4` remain open.
