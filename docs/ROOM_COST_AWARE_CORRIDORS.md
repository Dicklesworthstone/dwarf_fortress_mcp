# Cost-aware room corridors

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` and
`df-dfhack-bridge-plane-c-pic.3/.4/.5`.

`room_cost_connections` compares the existing complete connector with a
cost-first corridor candidate. Binary per-cell planning costs do not identify
observed terrain. Protected cells, original floors, entry points, same-level
bounds, the remaining tile allowance and remaining rectangle allowance stay
hard constraints. Zero-cost cells are not made passable by their cost.

The baseline runs first under the original connector, including its bounded
minimum-tile fallback. A second deterministic greedy pass minimizes added cost,
then added tiles, then cardinal steps for each connection. It is NOT a global
minimum-cost network solver. Compare complete feasible candidates by summed
added-cell cost, added tiles and rectangle count; prefer the baseline on exact
ties. A baseline exceeding the remaining rectangle limit cannot be selected.

A cheaper detour that exceeds either hard capacity is discarded as a whole.
The baseline may then be retained. Work exhaustion, cancellation or any other
unexpected failure instead returns no result, even after a valid baseline.
One original 100,000-check ceiling covers cost validation, both routing attempts,
comparison, reconstruction verification and witness publication. The baseline
has no separate renewed allowance. No background search or native call exists.

The result reports both costs, which candidate was selected, why a cheaper
candidate was rejected and the cost-map digest. The exact selected partition
is re-expanded and checked against the full original masks. The result never
claims observed floors, native pathfinding, global cost optimality or permission
to mutate the game.

Core regression command:

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_cost_connections.py' -v
```

These 18 tests use actual original connector modules. An independent simple-path
oracle checks all 2,187 cost/barrier assignments for a two-terminal 3-by-3 map.
Tests also cover original component preservation, remaining tile/part limits,
every guarded checkpoint, exact shared work exhaustion and unchanged inputs.
This increment establishes a Python component, not live game qualification.
