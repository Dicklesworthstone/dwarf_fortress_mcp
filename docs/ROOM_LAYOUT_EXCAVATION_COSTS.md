# Compare complete layouts by excavation cost

Bead: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`; whole-system work remains open.

The offline layout request can now select:

```json
"objective": "minimum-excavation-cost/1",
"excavation_costs": "111100001111000011110000"
```

`excavation_costs` is an exact binary string covering every cell of the original
same-level region, in x-fastest/y order. Zero means no charge; one means a charge
when the final target mask includes that cell. It is required only for this
objective. Missing, partial, nonbinary or unrelated cost maps are rejected.
The original region remains limited to 1,024 cells; no capacity is enlarged.

For an 8-by-3 region, the example prices the left four columns at one and the
right four at zero. A one-table dining hall fits either half. The search selects
the complete right-hand hall instead of the first, left-hand placement: cost
zero instead of twelve, with the same twelve target cells and two furnishings.

## Semantics and proof scope

The comparison order is charged target count, total target count, summed entry
distance to focus, then original-area lexical y/x/z/rotation tuples. The complete
floor mask is scored AFTER the existing compiler accepts it, including furniture
targets, access points and connector corridors. Required preserved walls are not
excavation targets. Costs do not permit a target in an excluded/protected cell.

Costs supplied to the offline solver are **planning assumptions, not observed
terrain**. A zero, an `F` site constraint, a self-consistent checksum, or a cheap
layout does not establish existing floor, mineability, safety, native access,
construction completion or mutation authority. Actual mining time and resource
costs are not modeled.

The original deterministic connection compiler is unchanged. It does not reroute
individual corridors according to this cost map. Proven optimality is among the
complete candidates produced by that compiler at the declared origins/rotations,
not over every possible corridor graph or fortress plan. The optional
`connection_strategy` still selects corridor fallback independently.

Branch bounds sum independent minimum area costs and relax extra corridor costs
to zero. They never remove hard constraints from accepted candidates. One shared
node/work/deadline allowance covers search, cost validation, scoring and output.
A node-limited complete incumbent has no optimality claim. Work exhaustion or
caller cancellation returns no incumbent or partial plan.

The existing `layout_rooms.py` solve/replay/exports remain the entry point.
The new request and cost map are retained and hashed; saved artifacts replay the
same complete comparison. Requests with no objective or the original footprint
objective retain their previous bytes, choices, traces and work counts.

## Executed validation

Fifteen warnings-denied Python tests run the actual complete compiler and
allocator. Independent origin/rotation enumeration checks 128 binary maps and
18 two-area cases. Other cases cover original access/corridor charges, all eight
areas and 32 furnishings, hard exclusions, fixed origins, cross-level selectors,
whole shortages, every checkpoint in a small cancellation case, exact work/node
limits, rehashed tampering and no input mutation. A separate differential run
preserves 108 legacy layout outcomes byte-for-byte.

These are component results with supplied planning costs, not map acquisition,
DFHack/native, Rust, full-repository or production qualification.
