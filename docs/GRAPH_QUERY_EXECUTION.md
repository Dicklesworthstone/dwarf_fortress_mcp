# Bounded observed-graph analysis

The public `dfmcp_world::graph_query` module implements reference cognition over
an already-authorized canonical snapshot. It adds no transport or dependency,
and cannot dispatch or authorize a game effect.

## Traversal and paths

`traverse_graph` supports multiple roots, a set of edge-kind selectors, outgoing,
incoming, or undirected interpretation, and an explicit maximum depth. Empty
kind selection means all observed edge kinds. Roots use ascending stable entity
IDs. Adjacency ties use destination entity ID, then edge ID. Parallel edges
retain their identity; a shortest path records the selected edge and revision.

The result includes breadth-first visits with entity generation/revision, source
root, depth, parent, and edge witnesses. `observed_path_to` reconstructs a bounded
path and rejects malformed parent records. An unvisited target is not proof of
unreachability in Dwarf Fortress.

`depth_frontier` lists visited depth-bound vertices that still have an observed
neighbor outside the visited set. A fully explored cycle does not falsely report
a depth cutoff. Vertex, edge, frontier, and operation-budget exhaustion return
an error, not a partial successful negative answer.

## Dependency diagnosis

`analyze_dependencies` interprets selected edges as **dependent -> prerequisite**.
It uses iterative strongly connected components, avoiding recursive DFS stack
growth. Each component has a minimum-stable-ID identity and sorted members.
Components are ordered by the condensation DAG, prerequisites before dependents,
with minimum stable ID breaking ties between ready components.

Self-loops and multi-vertex SCCs are cycles. `blocked_by_cycles` includes both the
cycle members and every observed entity depending on a cycle. Cyclic inputs do
not receive a fake entity-level schedule or critical chain.

Acyclic inputs return prerequisite-first order and a deterministic longest
unweighted dependency chain. The chain is dependent-first and counts edges, not
game ticks, labor duration, capacity, or resource contention. It is not a timed
production schedule or proof of in-game completion.

## Witnesses, bounds, and authority

Results carry their exact source anchor, caller-supplied authorization-scope
digest, canonical projection digest, decision-path digest, and operation counters.
Built-in and custom edge kinds with the same display name remain distinct.

The supplied scope digest does not grant authority or filter data. The caller
must provide a snapshot whose vertices and edges were already capability-filtered.
All results describe observed relationships in that projection. Missing edges
are not complete-domain absence evidence, and paths are not walkability proofs.

The implementation caps input vertices/edges, frontier size, traversal depth,
kind-name bytes, and algorithm work units. Work units count explicit node/edge
processing, not every comparison performed by ordered containers or sorting.
Canonical snapshot hash validation still encodes the entire supplied snapshot.
These are conservative algorithm bounds, not measured latency guarantees.

## Evidence and integration status

Eleven Rust regression tests cover shortest-path ties, parallel edges, duplicate
and multiple roots, reverse/undirected traversal, depth cutoffs, prerequisite
order, critical chains, cycles and dependent blockers, self-loops, disconnected
and empty graphs, input/budget failures, scope identity, and corrupt path records.
One test exhaustively compares every directed three-vertex graph with independent
closure and shortest-distance oracles.

An independent Python algorithm-design oracle also checked 512 exhaustive graphs
and 200 seeded multigraphs during implementation. This was not execution of the
Rust source. Rust compilation, formatting, Clippy, native tests, live-game
validation, and graph admission remain unestablished by this editing session.

The APIs are available to Rust callers. MCP graph-query modes and live projection
coverage are separate integration work; the eleven-tool waist is unchanged.
