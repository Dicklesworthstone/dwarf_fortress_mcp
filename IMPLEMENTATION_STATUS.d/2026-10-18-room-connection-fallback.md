## Reviewed complete corridor fallback and layout integration

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. Broad work remains open.

Published the previously pending minimum-corridor fallback through the existing
RoomPlan compiler. It restarts from complete original masks only when greedy
routing exceeds the tile allowance, shares the original 100,000-check budget,
preserves all protected cells, and validates the exact reconstructed union.
No component, room, furnishing or entrance may be dropped to obtain a fit.

Fresh review reproduced a saved-layout regression in the pending integration.
An original successful search rejected a first candidate and accepted a second;
enabling fallback accepted the first and invalidated the saved artifact. The
original no-layout report also failed replay. Explicit request-bound selection
now separates old greedy search from new fallback search. Existing artifacts
retain exact bytes, choices, traces and work counts. There is no global mode.

65 Python tests passed with warnings denied: 11 unchanged connector tests,
15 pending connector/fallback cases, 12 full RoomPlan/allocation cases, eight
solver-core cases, 14 new layout/replay cases and five real CLI cases. Tests use
full imports of real modules and real subprocess CLI execution. Two graph
oracles check all 6,561 small worlds, with 400 additional seeded larger worlds.
All six CLI exports run from requests and saved layouts. Resource fault injection
only lowers the actual solver component bound; no alternate solver is supplied.

The complete example preserves eight 7-by-7 bedrooms, two dining halls and 28
furnishings: 499 original tiles plus 13 optimal corridor tiles, 512 total in 24
parts. A separate three-component junction oracle confirms the added minimum.
The existing complete allocator consumes all 28 slots or returns whole shortage.

A differential comparison against hash-verified main 8813904e preserves all 162
legacy layout outcomes. Both the old successful and failed replay regression
fixtures have exact pre-change digest goldens. All 4,832 accepted and 1,729
refused generous-budget connector cases retain their original aggregate digest.
The independently added cross-level selectors remain unchanged and are tested
through compilation/regeneration without weakening same-level-only allocation.

Runtime and test blob hashes are checked against tested local bytes. The unchanged
CLI was verified as Git blob 16046453cbb72a167cbff1111b95af753e16019c; the original
layout compiler was 535259022d85d3a44b3c007d3e3b92f506d0b8ca, and the room compiler
was d3abbb23215e1be5c0994f9d9a25d29b37095b14. No AST extraction or import adapters
were used. Native/observed-map acquisition, Rust, DFHack, full-repository and live
qualification were not executed. Native methods, journals, mutation authority
and production admission are unchanged.
