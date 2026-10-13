# Route-aware furniture delivery sequencing

**Evidence: Python component checked; synthetic captures only.** This does not
qualify Rust, the workspace, a native build, DFHack gameplay, a bridge tuple,
production admission, placement eligibility, or future pathfinding.

Beads: `df-franken-storage-mvcc-54h.4` and `df-dfhack-bridge-plane-c-pic.3/.4`.

## Functional boundary

`scripts/route_furniture_sequence.py` closes a gap between whole-plan allocation
and fixed-order furniture execution. An assignment may have a route for every
item in the initial map, yet placing a near-door furnishing first may obstruct
all routes to a later target in the existing conservative route model.

`allocate_sequenced_routes(request, candidates, terrain_bytes, guard, ...)`
returns **`(effective_request, allocation)`**. Keep this pair together: the
allocation's request digest, executable plan, and retained review must bind the
effective request, not a stale copy of the input request.

The planner uses the unchanged map/1.5 decoder, dry cardinal floor/stair graph,
exact route allocator, request validator, and fixed-order action DAG. It does
not read a native process, bind another RPC, place furniture, reserve items,
reallocate a selected item, or relax material, subtype, exclusion, distance,
vertical, target, world, site, or original dependency constraints.

The planning model removes each earlier placement's target from the captured
graph. This is deliberately conservative; it is not a claim about the exact
walkability of a particular native furnishing. A source item covered by an
earlier target is unavailable in this model too. Every delivery must remain
within the original maximum route length after earlier targets are removed.

## Ordering and outcomes

An already feasible canonical order is preserved without changing request or
plan bytes. Otherwise the compiler adds a dependency chain that forces a full
feasible order, while retaining every original `after` edge. The original
request and plan identities remain in `delivery_sequence` for inspection.

The initial allocator's item assignment and minimum-initial-route objective are
unchanged. Sequencing is not a joint item/order optimizer and does not claim to
minimize the sum of post-placement delivery distances.

If no order exists for those selected items and original constraints, the result
has `status: delivery_order_blocked`, a scoped blockage, and **no executable
plan or plan digest**. It does not claim a global inventory shortage: another
item assignment, a larger observed region, or actual native behavior may differ.
Existing shortage and unestablished-target outcomes are retained unchanged.
Budget exhaustion, cancellation, malformed captures, and output overflow refuse
without publishing a partial plan or partial sequence certificate.

## Algorithm and witnesses

The policy is `preserve-canonical-else-reverse-feasible-sinks/1`. For a changed
order, repeatedly choose the lexically greatest remaining dependency sink with
a delivery route after *all other remaining targets* have been removed. Move
that sink to the end and repeat. This has at most `N*(N+1)/2` route probes, not
factorial permutation search. Existing bounds remain 32 slots and 16,384 tiles.

This reverse choice is complete for the fixed-assignment monotone model: any
valid order can move a feasible sink to the end. Other deliveries only lose
blockers, while the chosen sink has already passed its worst remaining prefix.
If no sink can pass, no dependency-respecting ordering of those selections can
pass that model. The final compiled order is independently replayed forwards.

Each forward witness commits to the raw map digest, path policy, removed-target
prefix, source, target, route limit, and every tile in the selected shortest
route. The allocation decision digest covers the original request, effective
request identity, full order, witnesses, outcome, and work accounting. These
are deterministic content commitments, not signatures or native acquisition
attestations. `future_routes_preserved`, `future_routes_guaranteed`, and native
pathfinding claims remain false. Fresh original-selection review and normal
receipt/placement boundaries are still required.

One cumulative work allowance covers initial allocation, graph decoding,
sequencing, and forward replay. No allowance is replenished between stages.

## Component verification

```sh
python -m unittest discover -s scripts/tests -p 'route_furniture_sequence_tests.py' -v
```

The suite exercises blocked corridors, original dependency conflicts, longer
post-placement detours and exact route limits, covered source items, cross-level
stairs, unavailable targets, unchanged safe plans, deterministic permutations,
32-step prefix execution, complete digest coverage, cancellation, and exact
work-budget boundaries. A seeded 500-instance oracle enumerates every order of
up to five slots with an independent coordinate BFS and compares feasibility
and each returned delivery length. The fixture bytes use the real native value
codec layout; all maps, items, and progress receipts in these tests are synthetic.
