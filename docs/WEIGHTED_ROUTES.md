# Weighted routes over observed relationships

`dfmcp_world::graph_query::shortest_routes` computes least-cost routes from one
to 64 sources through an immutable, already-authorized relation projection.
This implements the nonnegative weighted routing and multi-source selection
described in comprehensive plan section 21.10 and
`FORTRESS_GRAPH_ALGORITHMS.md` sections 6.5 and 6.6. The existing unweighted
traversal and terrain-routing contracts are unchanged.

## Source and cost contract

`WeightedRouteQuery` names source entity IDs, explicit edge kinds, an outgoing
or incoming direction, and a cost field. Outgoing traversal follows stored
edges; incoming traversal reverses them. Undirected traversal is refused, so
one directional cost cannot silently become a symmetric movement cost.

Every edge of the selected kinds must contain a consistently Known,
nonnegative integer cost in that field. Zero is valid. Signed nonnegative and
unsigned integers are accepted without floating-point conversion. All costs
use the selected field's units; the field name does not imply distance, danger,
travel time or game ticks. Source facts must satisfy the registered laboratory
contract, or be nonfuture DFHack fields with bounded source names and a nonzero
source digest. These checks establish source shape, not native admission.

Missing, unavailable, contradictory, future, negative, mistyped or unqualified
costs refuse the whole analysis, including costs on selected edges in
unreachable components. No edge is silently dropped or assigned a default cost.
The complete input graph still passes identity, hash, vertex and edge bounds.
Authorization filtering belongs to the owner before it supplies this projection.

## Results and deterministic ties

Each reachable record retains entity identity, generation and revision, winning
source, exact total cost, hop count, parent, edge identity/revision and edge
cost. Records and unreachable IDs are returned in stable entity order.

The closed policy minimizes total cost, then hop count, then source identity,
then immediate predecessor identity and edge identity. It defines a canonical
predecessor tree rather than lexicographically comparing complete paths.
Strictly decreasing parent hop counts prevent zero-cost cycles from producing
cyclic path witnesses. Sources are normalized as a set at the core boundary.

`path_to` reconstructs a selected route with exact edge revisions and costs.
The bounded batch variant `paths_to_with_check` validates records once and
checks the caller's cancellation/deadline callback while reconstructing paths.
It also rejects malformed public witness records instead of trusting them.
Callers remain responsible for bounding aggregate path materialization and
their final output.

An unreachable result means that no path was found within this exact observed
projection. It does not prove complete-world absence, game walkability, safety,
hauling capacity or a mutation precondition. No native cost collector, movement
command or game effect is introduced.

## Bounds and evidence

The core uses a bounded multi-source Dijkstra analysis with a single live
ordered frontier entry per unsettled vertex. It uses bounded O(V + E)
auxiliary state and ordered logarithmic queue/lookup operations, without path
enumeration or recursion. Decreasing a tentative cost replaces its queue entry;
old candidates cannot amplify frontier memory.

`GraphBudget` bounds complete source vertices/edges, live frontier and total
work. Checked arithmetic refuses unrepresentable reachable cost sums, even for
alternatives that would not improve the chosen route. Budget exhaustion and
cancellation return no partial negative result. The caller check runs before
and after shared index construction, through analysis and before return; shared
source hashing/index construction remains count-bounded rather than internally
cooperatively interrupted.

Projection and decision witnesses bind the exact source anchor, authorization
scope, selected edge kinds, cost field and canonical cost facts, roots,
direction, policy, bounds and complete decisions. A witness grants no authority.

## Existing-tool query

The default laboratory runtime and generic semantic executor expose this
analysis through the existing `fortress_query` structured mode. Pass the
following object as the JSON string in `mode`:

```json
{
  "mode": "structured",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "weighted_routes",
      "edge_kinds": ["uses"],
      "cost_field": "haul_cost",
      "direction": "outgoing",
      "roots": [{"entity_id": "10", "generation": 1}],
      "targets": [{"entity_id": "42", "generation": 1}]
    }
  }
}
```

Use inspected entity handles and an actual published integer edge field. This
example does not imply that the starter world or a native profile publishes
`haul_cost`. The query accepts no caller-provided weights or fallback costs.
Roots must contain one to 64 distinct IDs with matching generations; optional
targets accept at most 64 distinct generation-bound handles. Both sets are
canonicalized for deterministic output. Direction defaults to `outgoing`;
`incoming` computes costs along the reverse of each recorded relation, without
asserting that movement in that direction is physically possible.

Omitted or empty targets return the complete shortest-path forest as parent
records. Explicit targets return selected records and complete reconstructed
paths, including exact edge versions and original qualified cost facts. Root,
target and returned entry handles carry entity generations/revisions; path
vertex IDs refer to the exact source anchor. Unreachable targets are explicit.
Every selected-kind edge must still have a
usable cost, and all source records and canonical bytes are admitted before
analysis; asking for one destination does not bypass complete-source checks.
The full result must fit the output allowance. There is no truncation or
pagination that could hide a path edge, and aggregate path reconstruction and
rendering consume bounded work under the original deadline.

The laboratory wrapper accepts an exact retained `at` hash. Roots and targets
refer to generations in that source, while current Query authority is checked
against the actual current tick, including across clock-reset restores. The
source anchor, current anchor and scope/decision witnesses stay distinct.
Reserved Agent Turn output does not reduce the separately admitted source
allowance. No clock advancement, native acquisition, work reservation or
mutation authority is created. Live profiles keep their own discovery and
history allowlists; this addition does not claim universal live-profile
support or native cost coverage.

## Verification scope

`crates/dfmcp-world/tests/weighted_routes.rs` checks the real implementation
against an independent simple-path oracle over 16,384 weighted graph, source
and direction cases. Focused regressions cover zero-cost cycles, multiple
sources, parallel edges, exact ties, a 4,000-node chain, provenance/presence,
complete-source refusal, overflow, frontier/work limits, cancellation and
corrupt path witnesses. Exact executed scope is recorded in
`IMPLEMENTATION_STATUS.md`; core tests do not establish live-game admission.

Focused MCP tests cover whole-forest and selected-path results, incoming and
multi-source decisions, generation and source anchors, original cost facts,
source/output/work limits, cancellation, restored-clock authorization, schema
composition and duplicate raw keys. Public laboratory regression sources cover
facade reachability, retained generations and current grants. Executed
component and full-facade evidence are recorded separately in the status file.

Owning bead: `df-franken-storage-mvcc-54h.4.3`.
