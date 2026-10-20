# Cost-aware corridors within the original tile allowance

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5`. These broad beads remain open.

The previous cost-first connector finds an unconstrained cheap route and then
checks its size. When that route is too long, it discards the whole alternative.
A slightly more expensive route that fits can be missed even when it improves
on the original connector. The new opt-in strategy enforces the remaining tile
allowance during path search, without increasing any project limit.

For direct room provisioning, add the strategy to the complete existing
`corridor_costs` object:

```json
"corridor_costs": {
  "region": {"origin": [100, 100, 2], "size": [16, 12, 1]},
  "cells": "<one 0 or 1 per original domain cell>",
  "strategy": "bounded-cost-with-original-candidate/1"
}
```

The cells here are planning costs, not observed terrain facts. Direct room
compilation retains the complete cost region, cells and strategy in its intent.
As before, `connect_areas` must be true. Omitted strategy selects the original
`cost-first-with-original-candidate/1` behavior; explicit original strategy
normalizes to the same legacy bytes. No saved artifact is silently migrated.

## Search and hard limits

At each connection vertex the search retains nondominated combinations of
charged cost and new tile count. A cheaper but longer prefix cannot eliminate
an alternative needed to fit the hard limit. Path length breaks ties after
charged cost and new tiles, with stable coordinate/insertion ordering.
Immutable parent records survive frontier pruning and are independently checked
for exact path length, added tiles and charged cost during reconstruction.

The current connected floor component is a multi-source zero-cost root; already
required floors are not charged as new corridor tiles. Each completed link
subtracts its exact added tiles from the same original project allowance. All
original floor components and named entries must remain connected. Required
walls, furniture targets and excluded cells never become routing vertices.

The original connector still establishes the baseline first. A baseline failure
is not masked. A complete bounded-cost alternative competes with that baseline
by charged cost, tile count, part count and original-on-tie. If the alternative
cannot finish or exceeds the original part allowance, the entire alternative
is discarded. Only a complete baseline within the part limit can be retained.
There is no partial corridor output and no budget renewal.

At most 32,768 labels, including retired records needed for reconstruction, are
retained by a link search. All baseline work, label processing, reconstruction,
partition checks and output validation share the original 100,000-check ceiling
and caller guard. Label exhaustion, work/deadline exhaustion or cancellation
raises without returning the baseline or another cached candidate.

The next-link path is cost-minimal within its remaining tile allowance and
current graph. Greedily joining multiple components does NOT establish a globally
minimum network. Part count is checked on complete candidates, not optimized by
the path search. The unchanged layout optimum has only its original finite
candidate/compiler scope. Nothing here observes mining time, proves native
pathfinding, changes game state, or grants mutation or production authority.

## Regression evidence

A seven-tile connection in a 7-by-5 synthetic graph previously selected six
charged tiles after discarding an overlong cheap path. The bounded search uses
one charged tile with the same seven-tile allowance. An independent simple-path
oracle verifies its cost. Exhaustive checks cover all 2,187 small cost/barrier
worlds at each of eight tile allowances (17,496 comparisons), plus 200 seeded
larger worlds. Cancellation, label/work limits, exact protected masks, multiple
source tiles, retained components, hash-seed determinism and old defaults are
covered. These are Python component results, not native or live qualification.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_bounded_cost_paths.py' -v
```
