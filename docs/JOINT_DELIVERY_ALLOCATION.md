# Joint furniture item selection and delivery order

Beads: `df-franken-storage-mvcc-54h.4` and
`df-dfhack-bridge-plane-c-pic.3/.4` (broader work remains open).

`route_furniture_joint.allocate_joint_routes` closes a planning gap between
minimum-distance allocation and delivery sequencing. The cheapest assignment
can be impossible to deliver under the original dependencies, even though a
slightly farther compatible item makes the complete request feasible. Searching
only permutations of the cheapest selected items cannot solve that case.

The new opt-in planner first executes the existing allocator and sequencer.
Already deliverable, shortage and unestablished-target results remain unchanged,
including canonical output bytes and work counts. Only `delivery_order_blocked`
starts a joint search over every supplied eligible item and dependency-respecting
order. It never runs during recovery or revalidation of retained original intent.

## Search and result meaning

Search constructs the schedule backwards. Its last remaining step must be a
sink in the original dependency graph and its selected item must reach its
target with every other remaining target blocked. Every possible sink and every
unused compatible item is enumerated in descending slot-name / ascending item-ID
order. Backtracking undoes tentative choices, so a generic slot cannot permanently
consume the only compatible item for a constrained slot. Later suffix targets
remain open while an earlier delivery is evaluated. This covers every assignment
and order in the declared monotone target-removal model, subject to the bounds.

The full supplied candidate list is used. The cheapest-N reduction valid for
static minimum-cost matching is NOT valid for delivery feasibility: all N cheap
items can be stranded on the wrong side of an earlier target. A regression uses
65,536 items with the sole deliverable alternative at the end of the inventory.

A complete solution is independently replayed forwards using the existing
stable-index route checker. Original material, subtype, excluded IDs, Manhattan,
vertical and total route limits remain. Dependencies are only strengthened when
needed to encode the selected order; no target is omitted. Initial shortest-route
witnesses and placement-order route witnesses are both regenerated.

Fallback is first-feasible, NOT minimum-cost. Its `selection_policy` and
`delivery_sequence.policy` are
`preserve-deliverable-else-joint-reverse-sink-item-search/1`, and its objective
explicitly says first-deliverable. The report retains the initial assignment and
its decision digest, changed selections, search counters and trace digest. The
base allocation schema/selector policy stays unchanged for existing consumers.

Results distinguish `allocated`, `delivery_assignment_blocked` (finite supplied
model exhausted), and `delivery_assignment_search_limit` (inconclusive). Failed
joint searches contain no plan, assignment rows or partial route certificate.
An initial static matching count is not a delivery-completion count. Exhaustion
is not global absence, native pathfinding infeasibility or construction safety.

## Bounds and API

```python
from route_furniture_joint import allocate_joint_routes

effective_request, result = allocate_joint_routes(
    original_request, candidates, raw_map_capture, guard,
    max_route_steps=1000, maximum_work=20_000_000, max_search_nodes=10_000,
)
```

Keep the effective request and result together, as with the existing sequencer.
This chooses NEW intent before review; it cannot replace items in a retained
review or original batch. Its candidates and raw map require independently
qualified acquisition. No native call, reservation, mutation or permission occurs.

One work allowance spans the legacy solve, full-inventory scan, joint search,
forward replay and publication. Work/callback failures raise without a result.
The node cap is 1..100,000, default 10,000, including root and successful leaf.
The existing 32-slot, 65,536-item, 16,384-tile and 49,152-output-byte limits remain.
Search is exponential in the worst case but cannot exceed those work/node caps.
No exponential memo table or slots-by-inventory matrix is allocated; recursive
depth is at most 33 frames, with at most 32 bounded BFS sets live at once.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error \
  -m unittest test_route_furniture_joint -v
```

All 15 actual Python tests pass with no skips. Tests execute the unchanged
allocator, sequencer, raw-map decoder and plan codecs, not producer doubles.
Coverage includes all 512 small obstacle masks and 150 deterministic random
domains against independent exhaustive injections/topological orders, constrained
item backtracking, 32 targets, 65,536 candidates, stair opt-in, source/selector
refusals, exact work boundaries and every callback interruption for a fixture.
All four imported source modules were matched to their Git blob IDs before
execution. This is component evidence, not native acquisition, live DFHack,
Rust/MCP, SDK/plugin, full-workspace qualification or production admission.
