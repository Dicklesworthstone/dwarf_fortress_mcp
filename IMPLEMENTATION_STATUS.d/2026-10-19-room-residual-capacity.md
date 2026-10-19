## Complete residual-capacity planning

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. Broad execution remains open.

The original final-only capacity check can reject the layout already selected
by search. Reproduced with a 32-furnishing dining hall: 59 scattered remaining
wall tiles require 59 parts. The opt-in bounded-original-dig/1 residual policy
checks compiler-accepted candidates before ranking and selects the feasible
position with 90 remaining tiles in 28 parts, preserving all 117 target tiles
and all 32 furnishings. Partition and native limits are unchanged.

The original terrain reducer's exact global/original-part choice and independent
8-wide partition are extracted into one shared function. Legacy guard counts
and canonical bytes are preserved. The search uses binary cost-one assumptions;
no native terrain evidence or mining permission is inferred. Node-limited
incumbents must be complete and capacity-valid; other resource failures return
no plan. Reopening repeats the exact original strategy and cost map.

Executed 15 warnings-denied Python tests using complete real compiler, allocator
and terrain modules. Tests cover independent enumeration of all 16 origins,
24 seeded cost maps, 512 small residual masks, actual raw-map reduction,
32-item allocation/whole shortage, original constraints, saved-artifact tampering,
empty work, node limits and every checkpoint in a small cancellation fixture.
All 224 independent legacy comparisons pass: 96 layout outcomes and 128 raw
terrain survey outputs with exact work counts.

Core source and tests were executed in a component checkout. Map-backed strategy
selection, full CLI/TCP workflow, actual native dig dispatch, Rust/DFHack builds,
full-repository and live-fortress qualification are not claimed by this increment.
