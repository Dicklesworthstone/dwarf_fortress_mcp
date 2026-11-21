# Minimum-cost delivery in an observed network

The `GA-MCF` analysis implements the finite, nonnegative-cost directed subset
of comprehensive plan section 21.10 and `FORTRESS_GRAPH_ALGORITHMS.md` section
6.8. Given a source, a destination and a positive requested quantity, it first
maximizes delivery up to that quantity, then minimizes the cost of that delivery.
This can compare modeled hauling or resource routes that share finite capacity.

The API is `dfmcp_world::graph_query::minimum_cost_flow`, with
`minimum_cost_flow_with_check` for foreground cancellation and deadline checks.
It analyzes one immutable canonical projection already filtered under the
caller's authority. It does not collect native capacities or costs, assign jobs,
reserve materials, dispatch movement or prove actual gameplay completion.

## Source model

`MinCostFlowQuery` names distinct existing source and sink entity IDs, explicit
edge kinds, a positive finite `u64` demand, and four distinct source field names:

| Field selector | Required fact on every selected edge |
| --- | --- |
| `capacity_field` | Nonnegative integer finite capacity |
| `unit_field` | Common source-recorded capacity unit |
| `cost_field` | Nonnegative integer cost per unit of flow |
| `cost_unit_field` | Common source-recorded cost label |

Numeric facts accept `u64` or nonnegative `i64`. There is no floating-point
coercion, implicit cost, unit conversion or infinity sentinel. `u64::MAX` is
finite. Unit labels must be nonblank Text of at most 128 UTF-8 bytes, contain
no control characters, and agree exactly within each dimension. The field names
are also bounded, nonblank and distinct. A cost counts the recorded cost units
per one capacity unit; total cost sums those contributions. Dimensional
interpretation belongs to the source model.

Both cost and capacity facts follow the existing flow qualification contract:
consistently Known values from a registered laboratory source, or a bounded
DFHack field source with a nonzero digest, observed no later than the selected
snapshot. Missing, future, unknown, redacted, stale, contradictory, unqualified,
negative or mistyped facts refuse the entire analysis. The same checks cover
disconnected components, zero-capacity edges and self-loops.

Stored directions are preserved. Parallel and antiparallel edges retain their
original identities. Self-loops are qualified and returned with zero flow.
Zero costs and zero capacities are valid. An empty selected network returns
zero delivery, the entire requested shortfall and no established units; this
does not prove that the real fortress has no route or supply.

The demand is an explicit advisory request, not observed demand or authority.
The algorithm does not add unobserved edges or accept replacement capacities,
costs or units from the caller.

## Objective and deterministic execution

The objective is lexicographic:

1. Maximize delivered flow without exceeding the requested demand.
2. Minimize the sum of `flow × cost` over the selected original edges at that
   delivered value.

A satisfied demand says nothing about additional network capacity. A shortfall
means the selected network cannot deliver more, and includes an exact
equal-capacity minimum cut explaining that model limitation.

Successive shortest residual paths use Dijkstra with integer vertex potentials.
Original costs are nonnegative; residual reverse arcs have the corresponding
negative cost so later routes can undo an earlier allocation. An augmentation
uses the path bottleneck or remaining demand, rather than advancing one unit
at a time. There is no recursion with graph depth.

The frontier contains at most one entry per vertex and orders by reduced
distance then stable entity identity. Adjacency uses stable original edge ID,
forward before reverse for self-loops. Equal-distance candidates retain the
first settled predecessor. These ties are deterministic, but do not claim a
lexicographically smallest optimal flow among every possible optimum.

Each shortest-path pass covers the entire reachable residual component.
Reachable potentials increase by their shortest distance; unreachable
potentials increase by the maximum finite distance from that pass. Updating
unreachable vertices is necessary to preserve dual feasibility on residual
edges entering the reachable component. An early exit when the sink is reached
would not supply this global certificate.

For `A` augmentations, execution costs `O(A × (V + E log V))` with `O(V + E)`
auxiliary space. There is no claim that `A` is independent of capacities.
The explicit work allowance bounds all passes, initialization, augmentation,
certificate construction and decision serialization. Input vertices and edges,
the live frontier and reconstructed paths have independent bounds. A cancelled
or exhausted computation returns no partial allocation or certificate.

## Independently checked certificates

Before returning, a separate pass over original edge records verifies:

- Every flow lies between zero and its recorded finite capacity.
- Internal vertices conserve flow; net source outflow and net sink inflow
  equal the reported delivered value.
- Total cost equals the checked sum of each original `flow × cost`.
- Every positive-capacity residual arc has nonnegative reduced cost under the
  returned global potentials.
- When demand is unmet, the returned source-side/sink-side partition separates
  the endpoints and its crossing capacity equals delivered flow.

For an original edge `u → v` of cost `c` and potentials `p`, the forward
reduced cost is `c + p[u] - p[v]`; the reverse reduced cost is its negation.
The forward inequality is required when the edge has unused capacity and the
reverse inequality when it carries positive flow. Both apply to partially
filled edges. Checking every residual edge, including disconnected components,
rules out a negative residual cycle and certifies minimum cost at the given
delivered value. A shortfall cut separately certifies maximum delivery.

The result includes every selected original edge's identity, revision,
endpoints, capacity, cost and flow; one signed potential per observed vertex;
requested, delivered and unmet quantities; and the optional shortfall cut.
Total cost uses checked `u128`; potentials and residual arithmetic use checked
`i128`. Overflow refuses instead of wrapping or saturating.

Source anchor, authorization scope, selected edge kinds and field names,
qualified facts, endpoint identity, demand, policy, budgets, augmentation trace
and returned decisions bind the projection and decision witnesses. Changing
cost provenance, source time, an endpoint generation or an edge revision
changes the source-bound analysis identity. Witnesses grant no authority.

## Interpretation and verification

The allocation is exact within the supplied finite graph. It remains an
inferred model of fortress activity. Equal unit strings alone do not establish
native rates, physical feasibility, worker eligibility, vertex capacities,
shared resources, actual transport cost or complete-world coverage.

`crates/dfmcp-world/tests/min_cost_flow.rs` uses a separate exhaustive integer
edge-assignment oracle. It enumerates feasible tiny-network flows and compares
the best delivery and cost without using shortest paths or residual search.
Additional cases cover residual rerouting, global dual feasibility, source
qualification, unit disagreement, original identity, zero cycles, parallel and
antiparallel edges, exact bounds, cancellation, overflow and deep iterative
graphs. Actual execution evidence and its scope are recorded in
`IMPLEMENTATION_STATUS.md`; model tests do not establish native-game or
production admission.

Owning bead: `df-franken-storage-mvcc-54h.4.5`.
