# Joint item selection and delivery order

`route_furniture_joint.allocate_joint_routes` builds a NEW furniture proposal
when the nearest-items assignment cannot satisfy delivery constraints. It first
calls the existing allocator/sequencer once. A deliverable assignment is retained;
only `delivery_order_blocked` invokes bounded alternative-item/order search.
It never accepts a retained review or rewrites an existing batch.

Example: a chair must be placed at a corridor entrance before a bed farther
inside. The nearest bed is outside that entrance and becomes inaccessible in
the target-removal model. A slightly farther bed already inside can satisfy the
same request. This API can choose that bed without removing the chair-before-bed
dependency, changing a material requirement, or falsely reporting a shortage.

## Contract

Inputs and return values reuse the existing Request, Candidate, native map/1.5
bytes and `(effective_request, allocation)` pair. All item identities, material,
subtype, exclusion, endpoint/vertical distance, route length and dependency
constraints apply. The effective request only strengthens dependencies. A new
assignment explicitly lists every replacement and binds the initial decision.
The entire map/inventory commitment remains in the resulting decision digest.
Existing canonical request, route-review and plan formats are unchanged.

This fallback finds a deterministic first feasible solution; it does not claim
to minimize total initial or delivered distance across all assignments. Results
that retain the first assignment preserve its original objective. Every found
fallback is independently replayed forward with full route witnesses before
publication. Subsequent review checks freeze its NEW item choices; they cannot
replace them again automatically. No capture, reservation or native effect is
performed by the solver.

## Search and completeness

Reverse search chooses a remaining dependency sink, treating all other remaining
targets as blocked. It considers unused compatible items reachable within the
original route limit. Chosen suffix items remain unavailable to earlier steps.
The policy orders sinks by descending slot name and items by delivery distance,
then ID. Failed choices backtrack; this is not a greedy item picker.

For N remaining slots, retaining their current sink's N best unused reachable
items preserves feasibility: the other N-1 slots cannot consume all N choices.
Replacing the sink's item with an unused choice changes neither earlier target
blockers nor any other delivery. This reduction happens AFTER blocked routing
and suffix exclusion. Trimming the initial inventory first would be unsound.
One BFS is released before recursion; depth is at most 32, without a retained
slots-by-map or slots-by-inventory matrix. Search can still be exponential, so
both node and work bounds are mandatory.

`max_search_nodes` is 1..4096, including the empty successful leaf. The default
is 4096. One existing work allowance (at most 20,000,000 units) covers initial
allocation, initial sequencing, search and final verification without renewal.
The existing 32-slot, 65,536-candidate, 16,384-tile and 49,152-result-byte bounds
remain. Cooperative cancellation and budget refusal publish no partial result.

Outcomes are distinct:

- `allocated`: a complete model-feasible plan, not native placement authority.
- `delivery_assignment_blocked`: exhaustive reduced search established no solution
  for the supplied items and captured model, not global inventory absence.
- `delivery_search_exhausted`: the node cap interrupted search; no infeasibility
  claim, plan, partial sequence or partial route certificate is exported.

Initial shortage and unestablished-target outcomes remain distinct and do not
trigger search. `delivery_search` records node count, completeness, initial
decision identity, changes and policy. Its scope always names supplied inventory
and the conservative captured target-removal model. Native movement, present
safety, other observations, and future game behavior remain unproved.

## Executed validation

`python3 scripts/check_joint_furniture_delivery.py` passed 53 actual Python tests:
13 new joint-search tests plus 40 existing cross-floor/workflow regressions.
The new tests include 512 obstacle maps and 300 seeded unpruned assignment/order
oracles, protected material stock requiring backtracking, candidates beyond the
initial best-N list, 32-step execution, exact node/work cutoffs, canonical identity,
retained review behavior and callback cancellation. Five weakened implementations
fail expected assertions; syntax/import errors are not counted as mutation kills.
The runner records exact source hashes and rejects changes during execution.

Core tests use explicit map and item data. Existing workflow tests inject their
inventory, budget and outer packet dependencies. No native TCP acquisition,
private batch parent, Rust/MCP, real DFHack game or full-workspace qualification
was executed here. No wire, permission or production-admission change is made.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4/.5.
