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

## Existing allocation and review commands

The original-order mode is now available through both existing read workflows:

```sh
python3 scripts/allocate_route_furniture.py \
  --request-file furnishings-request.json --origin 0 0 5 --size 64 64 1 \
  --fixed-delivery-order --timeout-ms 10000

python3 scripts/review_route_furniture.py create \
  --request-file furnishings-request.json --origin 0 0 5 --size 64 64 1 \
  --fixed-delivery-order --timeout-ms 10000
```

The paths, fortress selectors and coordinates must match the intended capture.
Use the existing isolated read-only environment:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE=1
DFMCP_ROUTE_FURNITURE_ENDPOINT=127.0.0.1:5000
DFMCP_MAP_TOKEN=<operator-configured map read token>
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured inventory read token>
```

Other `DFMCP_` variables remain refused. The unchanged source owner brackets the
complete released operations inventory with equal paused map endpoints and closes
before solving. This is not an atomic cross-profile snapshot. The optimizer uses
the existing solver work reservation; review creation keeps its separate export
reservation. Current read authority and foreground time are checked before final
return. No new native methods, connections, credentials, retry or mutation path.

`--fixed-delivery-order` is a separate mode: do not combine it with
`--sequence-deliveries`, `--search-delivery-items`, `--minimize-delivery-cost`,
`--require-optimal-delivery` or `--delivery-search-nodes`. Incompatible flags are
rejected before input/native contact. There is no search-node cap for this
polynomial matching problem; a successful assignment is optimal for the fixed
order, while work/time exhaustion refuses instead of exporting an incumbent.
Existing modes keep their original behavior when the flag is absent.

Room-derived furnishing requests exported by the existing compiler can be used
without rewriting their dependency graph. This increment does not create a new
composite room handoff, attach original room geometry to a route batch, or extend
the native room lifecycle. Retain the parent room artifact separately. Exact
request preservation alone does not establish terrain completion or room custody.

A successful `create` response exports an ordinary immutable route review with
the exact selected items and unchanged request. The existing review codec and
`check` implementation are unchanged. `check` rejects the new allocation flag
before input/native contact and never invokes a solver or substitutes an item.
The review binds the allocation digest, not a replayable optimality proof; retain
the complete create response separately when that historical proof matters.
Rechecking individual routes is not a new proof of sequential-delivery feasibility.
The existing batch frontier checks remain required before any later placement.
Neither command writes a review file or starts a batch automatically.

Exit zero and `ok: true` mean a report exists, not necessarily a plan. Check the
status and nullable plan/review. `delivery_order_shortage` exports neither a
partial plan nor a review. Agent Turn briefing states fixed-order assignment
optimality and unchanged order without claiming optimality over other orders.

## Executed workflow evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_route_furniture_fixed_order test_route_furniture_fixed_order_runtime -v
```

All 29 distinct tests pass, zero skips: 15 core and 14 workflow tests. The workflow
suite loads the complete production runtime/CLI modules, not extracted function
bodies, and executes the real solver, map decoder, candidate projection, request,
plan and review codecs, file readers, argument parsing and publication checks.
It covers full 32-target review export, exact request preservation, no saved-item
substitution, pre-contact option refusal, source disagreement, failed acquisition,
shrinking work/export reservations, final authority loss and one-attempt stdout.
CLI-main subprocesses execute the same complete commands with injected transport.

Acquisition, operations-roster decoding, transport budget and endpoint parsing
are injected test boundaries. Legacy joint/cost-search modules are trap stubs in
the focused new-mode tests; their algorithms are not qualified by this run. No
native TCP/paging/release, real DFHack or standalone real-network import pipeline
is exercised. This is not a complete private room-parent or placement workflow.

Four deliberately weakened variants are rejected: dropping prior target blockers
(by independent forward verification), clipping each matching row to one item,
omitting final authority checks, and accepting allocation flags on saved-review
checks. The latter three fail regression assertions. Four existing modes (basic
and sequencing-only allocation and review creation) match their exact predecessor
packets byte-for-byte for one shared fixture. Five unchanged core/codec modules
match upstream Git blobs. Rust/MCP, SDK/plugin, live fortress and full-workspace
qualification remain unexecuted; no production admission is changed.
