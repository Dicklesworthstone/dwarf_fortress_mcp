# Minimize complete furniture-delivery route cost

Beads: `df-franken-storage-mvcc-54h.4` and
`df-dfhack-bridge-plane-c-pic.3/.4`. Broader integration/qualification remains open.

`scripts/route_furniture_optimal.py` adds an explicitly selected numeric cost
optimizer. The existing first-feasible joint planner, sequencer and retained-review
contracts are unchanged. This optimizer is for NEW intent, never an executing
batch or an original review being recovered or revalidated.

Initial shortest-route matching is not the complete-delivery objective: earlier
placements can force detours or strand later items. The new objective sums the
shortest candidate route AT EACH delivery, after earlier target tiles are removed
from the existing captured cardinal floor/stair graph. Both item assignment and
dependency-respecting order may change before review. All original item selectors,
exclusions, Manhattan/vertical/route limits and requested targets remain binding.

## Bounded optimization and proof

The existing complete sequenced allocation supplies an incumbent when feasible.
Reverse scheduling selects a dependency sink and an unused deliverable item,
blocking the other remaining targets. Branch-and-bound explores cheaper complete
assignments rather than stopping at the first alternative or first improvement.
All complete candidates are checked by the existing independent forward replay.

At each sink, the R cheapest currently deliverable unused items suffice for R
remaining slots: at most R-1 can be used by other slots, so a discarded item can
be replaced without higher cost or changing another target/route. This reduction
occurs AFTER routing with blockers, not by the initial unblocked distances. The
entire supplied inventory is scanned; a regression has 65,536 candidates with
the only usable alternative at its end.

Lower bounds use exact distinct-item matching with mandatory predecessor targets
already blocked. This remains a relaxation of every possible total order, but
avoids enumerating independent permutations just to rediscover an unavoidable
predecessor-induced detour. Matching failure prunes an impossible branch. A bound
at least the incumbent cost prunes nonimprovements. Attaining a sound global lower
bound proves the numeric optimum without enumerating all orders. Equal-cost ties
keep the first deterministic incumbent; no globally lexical optimum is claimed.

`delivery_optimization` exposes best complete cost, lower bound, absolute gap,
node/prune/improvement counters, deterministic trace and `cost_optimality_proven`.
`search_complete` means enough search/pruning to prove the result, not that every
permutation was visited. A node cap can return a FULL feasible incumbent with an
explicit nonzero/unknown optimality gap. With `require_optimal=True`, an unproven
incumbent is not exported: status is `delivery_optimization_incomplete`, and plan,
assignment rows and route certificates are absent. No incumbent plus complete
search yields `delivery_assignment_blocked` only in the supplied captured model.
Unknown targets and static inventory shortages retain their existing meanings.

```python
from route_furniture_optimal import allocate_optimal_routes

effective_request, result = allocate_optimal_routes(
    request, candidates, raw_map_capture, guard,
    max_route_steps=1000, max_search_nodes=4096,
    maximum_work=20_000_000, require_optimal=True,
)
```

Keep the effective request with the allocation when creating a new review. The
base allocation schema/selector policy stays compatible; `selection_policy` and
`objective` identify the additional optimization. Candidate initial-route costs
remain separately visible from complete-delivery cost. Numeric route optimality
is not native hauling-time minimization, game safety or effect authorization.

One work allowance spans baseline allocation, matching bounds, search, full route
replay and output. Work exhaustion/caller cancellation raises without publishing
an incumbent or renewing the budget. Node bounds are 1..4096; the existing 32-slot,
65,536-item, 16,384-tile and 49,152-byte result bounds remain. Worst-case search is
exponential but work/node bounded. BFS storage is released before recursion;
there is no slots-by-inventory or recursive-depth-by-map allocation.

## Executed component checks

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error \
  -m unittest test_route_furniture_optimal -v
```

Sixteen actual Python tests pass with no skips. They exercise the real solver,
existing allocator, sequencer, map decoder and plan codecs. All 512 small obstacle
masks and 200 deterministic random domains agree with independent exhaustive
assignment/forward-order numeric-cost oracles. Other cases cover all 32 slots,
65,536 candidates, explicit stairs, exact work boundaries, every callback abort,
node-limit incumbents, strict export, changed sources and original selectors.

One fixture improves complete route cost from nine to seven; another improves
12 to 11 initially and continues to the proved optimum eight. These are synthetic
model cases, not measured dwarf hauling. Three deliberately weakened source
variants are rejected: stop at first improvement, ignore removed targets, and
ignore strict optimality export. Four reused source modules matched original Git
blob IDs before tests. No native acquisition, real DFHack, Rust/MCP, SDK/plugin,
live-fortress, full-workspace or production qualification is claimed.
