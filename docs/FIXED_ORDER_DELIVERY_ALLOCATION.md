# Exact delivery allocation without changing original dependencies

Beads: `df-franken-storage-mvcc-54h.4`, `df-dfhack-bridge-plane-c-pic.3/.4`.

`route_furniture_fixed_order.allocate_fixed_order_routes` selects NEW items for
the original canonical topological delivery order. It returns the same normalized
Request and unchanged dependencies, not a strengthened request. This matters for
room-derived requests whose parent contract requires exact furnishing intent.
It is not a way to substitute items in a retained review or executing batch.

Initial nearest-item matching can assign a later delivery an item on a tile that
an earlier placement removes. Even when all individual routes exist now, that
assignment is not executable in the target-removal model. This solver prices each
slot after ALL earlier canonical targets are removed, including blocked item
origins, and then chooses a complete distinct-item assignment across those rows.

The order fixes every blocker set independently of item selection. Consequently
one Hungarian matching exactly minimizes total complete-delivery steps, then the
item-ID vector in lexical SLOT order, even when execution order is different.
It does not enumerate permutations or return an unproven node-limited incumbent.
Other delivery orders may be cheaper or feasible when this one is impossible;
`alternative_orders_ruled_out` is always false. This is not native hauling-time
optimality, worker access, future route preservation or permission to place items.

For N slots, retain each row's N cheapest compatible items AFTER routing with its
blockers. In an optimal matching the other N-1 rows cannot consume all N choices;
replacing a discarded choice with an unused retained item cannot increase cost
or the lexical item vector. A deficient Hall set has fewer than N neighbors, so
its rows were not truncated. Shortage reports therefore cover all supplied
candidates for that fixed order, not just an arbitrarily clipped search set.

The unchanged initial allocator validates all input and supplies baseline
counts/digests. One shrinking work allowance spans that solve, blocked routing,
matching, initial-route commitments for the selected items, and separate complete
forward delivery replay. The replay verifies every row cost before publication.
No native acquisition, journal, dependency, wire profile or authority is added.

Result distinctions:

- `allocated`: complete optimal assignment for the original order. Initial route
  lengths remain separate from `fixed_order_delivery.complete_delivery_cost`.
- `delivery_order_shortage`: scoped Hall-deficiency witness, null plan, no
  assignment rows or partial route certificate. It is not global infeasibility.
- Original static `shortage` and `target_unestablished` retain their meanings.

`compatible_counts` describes initial routing; the counts inside
`fixed_order_delivery` describe the original-order, blocked-target graph.
`delivery_sequence` uses the existing route witness model. Saved review codecs
can retain the chosen items without changing the request. A digest identifies
bytes, not native acquisition or permission. Fresh batch checks remain required.

```python
from route_furniture_fixed_order import allocate_fixed_order_routes
original_request, result = allocate_fixed_order_routes(
    request, candidates, raw_map_capture, guard,
    max_route_steps=1000, maximum_work=20_000_000,
)
```

Bounds remain 32 slots, 65,536 candidate items, 16,384 tiles, 49,152 result bytes
and 20,000,000 structural work units. Space is O(items + tiles + N*N), with only
one per-target BFS retained. Matching and routing are polynomial; maximum-size
combinations can still exhaust work or foreground time and refuse completely.
No relaxed bound, silent subset or fallback to initial matching is permitted.

## Executed component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error \
  -m unittest test_route_furniture_fixed_order -v
```

15 tests pass without skips. Independent exhaustive assignment and coordinate-BFS
oracles cover all 512 three-by-three obstacle masks and 250 seeded domains with
selectors, exclusions, dependencies and route limits. Other cases cover 32 slots,
65,536 items with the only usable later item beyond the initial candidate
reduction, complementary stairs, zero-step delivery, exact work limits, every
callback interruption, full replay refusal and process hash seeds. The four
imported implementation modules match their frozen upstream Git blob hashes.
These execute Python implementations, not DFHack, native TCP, Rust/MCP or a full
workspace. No production or live-fortress qualification is established.
