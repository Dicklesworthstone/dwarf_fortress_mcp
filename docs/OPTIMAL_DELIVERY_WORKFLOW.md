# Cost-optimized allocation and retained-review workflow

Beads: `df-franken-storage-mvcc-54h.4`, `df-dfhack-bridge-plane-c-pic.3/.4`.
The optimizer in `OPTIMAL_DELIVERY_ALLOCATION.md` is now reachable through both
existing read-only commands. It remains opt-in and only selects NEW intent.
Neither placement, recovery, original-review checking nor native protocols change.

## Allocation command

Use the existing isolated route-furniture read environment:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE=1
DFMCP_ROUTE_FURNITURE_ENDPOINT=127.0.0.1:5000
DFMCP_MAP_TOKEN=<operator-configured map read token>
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured inventory read token>
```

Other `DFMCP_` variables are refused by the unchanged authority loader. Tokens
are not written to outputs. The following paths and coordinates are illustrative;
the request must refer to the captured fortress and region.

```sh
python3 scripts/allocate_route_furniture.py \
  --request-file furnishings-request.json --origin 0 0 5 --size 64 64 1 \
  --sequence-deliveries --minimize-delivery-cost --require-optimal-delivery \
  --delivery-search-nodes 4096 --timeout-ms 10000
```

`--minimize-delivery-cost` requires `--sequence-deliveries`: it selects both items
and their dependency-respecting order to minimize summed delivery-route steps
AFTER earlier target removal. The default remains unchanged initial-route
allocation; sequencing alone retains its existing fixed-item behavior.

`--delivery-search-nodes` admits 1..4096 search nodes, default 4096. This bound is
not a new work or wall allowance. Projection, baseline solve, bounds, search,
forward replay and publication remain in the existing shrinking foreground budget.
The unchanged client acquires its complete map/inventory bracket and closes before
CPU analysis. No new socket, native method, credential, reservation or effect path
is added by optimization.

Without `--require-optimal-delivery`, a node-limited search may return its complete
verified feasible incumbent, explicitly marked unproven. With that flag, it emits
no plan until numeric optimality is proved. Exhausting work or cancellation does
not export an incumbent. Agent Turn briefing reports the numeric proof flag and
absolute cost gap, without asserting native hauling-time optimality or safety.

## Freeze the selected plan as a new original review

```sh
python3 scripts/review_route_furniture.py create \
  --request-file furnishings-request.json --origin 0 0 5 --size 64 64 1 \
  --sequence-deliveries --minimize-delivery-cost --require-optimal-delivery \
  --delivery-search-nodes 4096 --timeout-ms 10000
```

The existing create workflow passes the effective request, exact selected items,
source, native types and complete allocation together through the unchanged review
codec. It reserves artifact-export work before acquisition and does not lend it
to the optimizer. Strict incomplete results produce null `review`, `review_digest`
and `result.plan`. A successful review preserves all original requested targets
and selectors, strengthening dependencies only to encode the chosen order.

`--search-delivery-items` remains the existing first-feasible joint search. It is
mutually exclusive with cost optimization; it has not been replaced or rewritten.
Basic, sequencing-only and first-feasible review modes retain their prior behavior.

A retained review freezes the selected items and order. It binds the allocation
result digest but does not embed a replayable optimization search. Preserve the
complete create response separately when the numerical proof/gap matters.
`check` rejects optimization, reordering and search flags BEFORE input/native
contact. It revalidates the exact original selections; a missing item stays
unestablished even when a better replacement exists. It does not re-optimize,
renew constraints, change the review, or claim current optimality.

Use the unchanged artifact-retention and `check` commands documented in
`ROUTE_FURNITURE_REVIEW.md`. The CLI emits artifacts but does not write review
files or automatically start the existing route batch.

## Result interpretation

Exit zero and `ok: true` establish a report, not necessarily an exported plan.
Inspect `result.status`, the nullable plan/review, and `delivery_optimization`:

- `allocated`: a complete forward-verified proposal; inspect
  `cost_optimality_proven` rather than assuming all allocated results are optimal.
- `delivery_optimization_incomplete`: no exported plan; a node cap prevented
  proof or finding any complete candidate under the selected mode.
- `delivery_assignment_blocked`: complete search established no assignment/order
  in this finite supplied target-removal model, not global game infeasibility.

Static shortage and unknown-target results retain their existing status meanings.
Cost fields separate the initial assignment from complete delivery, expose a lower
bound and absolute gap, and distinguish matching-bound attainment, complete search,
and incomplete search. Complete infeasibility has its own proof-basis label.
All claims remain conditional on the supplied captured model and item policy.

## Executed evidence and limits

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_route_furniture_optimal test_route_furniture_optimal_runtime \
  test_route_furniture_optimal_review -v
```

All 39 actual Python test functions pass, zero skips: 16 core, 12 allocation
runtime/CLI and 11 retained-review cases. The core compares exact numeric costs
against independent exhaustive oracles on all 512 small obstacle masks and 200
random domains. It also covers 65,536 items, 32 targets, stair policies, exact work
limits and every callback interruption. Three weakened core implementations are
rejected (first-improvement stopping, omitted blockers and ignored strict mode).

Runtime tests load the complete production modules and execute the real solver,
map decoder, candidate projection, review codec, regular-file readers and CLI
main functions, including subprocesses and distinct Python hash seeds. Native
acquisition, operations decoding and the transport budget/endpoint boundary are
injected. These tests therefore do NOT qualify native TCP/paging/release or the
complete real-network import/execution path. Cases include pre-contact flag
refusal, source changes, final authority loss, no budget renewal, short stdout,
strict incomplete export, saved-review no-substitution and the full 32-target
review within the existing packet/codec limits.

For one shared fixture, five legacy modes (two allocation and three review modes)
produce byte-identical packets against exact predecessor source. Original imported
modules were Git-blob verified; the existing first-feasible joint planner and
review codec remain byte-identical. No real DFHack, SDK/plugin, Rust/MCP,
full-workspace, live-fortress or production-admission result is claimed.
