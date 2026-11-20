# Observed network throughput and bottlenecks

The maximum-flow/minimum-cut analysis implements the finite directed-network
subset of `GA-FLOW` in comprehensive plan section 21.10 and
`FORTRESS_GRAPH_ALGORITHMS.md` section 6.7. It answers how much flow the selected
observed capacities can carry between two exact entities, and identifies a
minimum-capacity cut that explains the limit.

This is an advisory model of one immutable canonical projection. It does not
collect native capacities, schedule hauling, move dwarves, change mechanisms,
prove fluid or power behavior, or authorize any game effect. The supplied
canonical projection must already have been filtered under the caller's
authority. Missing observations can change the actual network.

## Capacity and units

Each selected directed edge must have two qualified source facts:

- `capacity_field`: a consistently Known nonnegative `u64` or `i64` value;
- `unit_field`: a consistently Known, nonblank Text unit with at most 128 UTF-8
  bytes and no control characters.

The fields must differ. All selected edges must use exactly the same unit
string; no unit conversion or normalization is inferred. For example, an edge
recorded in `items_per_tick` cannot be combined with one in `items_per_day`.
The unit comes from the source, not from a caller's assertion. Zero capacities
are valid. Every integer, including `u64::MAX`, is finite; there is no infinity
sentinel in this operator.

Source facts follow the existing registered laboratory or bounded nonfuture
DFHack-field qualification contract. Missing, unknown, redacted, stale,
contradictory, future, negative, mistyped or unqualified facts refuse the whole
analysis. The same checks apply to selected edges in disconnected components.
An unselected edge kind does not need capacity facts, but its complete backing
record still contributes to source admission and identity checks.

An empty selected edge set has model flow zero and no established unit. This
describes the supplied projection only; it is not proof of zero real-world
throughput or complete-world disconnection.

## Directed flow and exact bottleneck proof

The source and sink must be distinct existing vertices. Stored edge direction
is preserved. Parallel and antiparallel edges retain their individual identities
and capacities. Self-loops are qualified and reported with zero flow.

The algorithm uses deterministic residual-network traversal with iterative
level construction and blocking-flow augmentation. It never recurses with
graph depth or advances one unit at a time. Stable vertex and edge identities
govern ties; the same source, scope, request and budget produce the same flow,
partition and decision witness. Residual reverse arcs permit earlier choices
to be rerouted when that is necessary to reach the true maximum.

Before returning, the implementation checks the resulting certificate:

- each edge flow is within its recorded capacity;
- every internal vertex conserves flow;
- net source outflow and net sink inflow equal the reported value;
- the residual source-side partition excludes the sink;
- the exact capacity of edges crossing from the source side to the sink side
  equals that value.

Feasible flow plus the equal-capacity cut proves optimality within the selected
finite network. It does not certify that the network represents all game
constraints. Checked arithmetic refuses a flow or cut total outside `u64`
instead of saturating it. A disconnected sink produces a zero-capacity cut
within the projection.

## Bounds, identity and publication

The complete input graph passes canonical identity/hash and vertex/edge bounds.
`GraphBudget` also bounds work and the live traversal frontier. Cancellation
and the original foreground deadline are checked through analysis and result
construction. Exhaustion returns no partial flow or partial negative claim.

Witnesses bind the source anchor, authorization scope, selected edge kinds,
source/sink, capacity and unit fields, qualified facts, deterministic policy,
budgets and exact decisions. A witness digest grants no authority. Changing a
capacity, unit, endpoint generation or edge revision changes the source-bound
analysis identity.

Core regression tests use an independent exhaustive cut oracle, and check
residual rerouting, parallel and antiparallel edges, self-loops, zero and
disconnected networks, unit/provenance refusals, exact budgets, overflow,
cancellation and deep iterative graphs. Executed evidence is recorded in
`IMPLEMENTATION_STATUS.md`; no native-game or production-admission claim follows
from these mathematical model tests.

Owning bead: `df-franken-storage-mvcc-54h.4.4`.
