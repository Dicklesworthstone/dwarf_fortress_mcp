# Directed bottleneck analysis

Implementation slice of **GA-DOM**, owned by bead
`df-franken-storage-mvcc-54h.4`. The bead remains open: this is not the
FrankenNetworkX/GraphDB integration or the flow-algorithm work it also tracks.

## What this adds

`dfmcp_world::graph_query::analyze_dominators` answers which vertices occur on
every directed path from a selected root. It returns immediate dominators,
strict dominated-descendant counts, and the explicitly unreachable part of the
supplied projection. This differs from selecting a shortest path: in a diamond,
neither alternative branch is unavoidable, while their reconvergence can be.

`dominator_chain_to` gives the root-to-objective sequence of unavoidable vertices.
It is **not a route**: adjacent entries need not be connected by an edge. The
impact count is how many *other previously reachable projected vertices* would
become unreachable after deleting that vertex. It is not lost production,
capacity, population, or a prediction of a game action.

Callers supply explicit edge kinds and choose `Outgoing` or `Incoming`; the latter
reverses stored edges. Empty edge-kind selections and undirected direction are
rejected. Disconnected vertices and unreachable cycles receive no dominator.
Parallel edges and self-loops are permitted. A missing root is an error.

## Safety, determinism, and cost

The input must already be authorized. No query obtains effects, observes a new
world, or turns a scoped projection into a whole-world negative claim. A new
anchor, relevant identity/generation/revision, edge, or authorization scope
invalidates reuse. The result is advisory unless an independent semantic contract
establishes the completeness and meaning required for a hard precondition.

The existing bounded graph index is reused. An iterative DFS chooses stable-ID
adjacency order, then reverse-postorder parent intersection converges to the
immediate-dominator tree. A single reverse-order accumulation computes impact;
it does not enumerate paths or materialize quadratic dominator sets. Algorithm
auxiliary storage is O(V + E). The reference fixed-point algorithm is not claimed
to be near-linear in the worst case: with P passes and dominator depth H, its
analysis work is O(P * (V + E * H)), cut off by the explicit operation budget.

The witness binds the complete source anchor, authorization/projection digests,
root, direction, selected relation semantics, policy version, supplied budgets,
DFS order, folded parent-decision transcript, results, and operation counters.
The policy is `observed-directed-dominators-rpo/1`. Changing the permitted work
budget can change the execution witness even when the mathematical answer agrees.

Vertex, edge, frontier, and work bounds fail closed: **no partial dominator tree**
is returned. `analyze_dominators_with_check` allows caller-owned cancellation and
deadline checks before/after shared index construction, every 128 analysis work
units, and immediately before return. Shared snapshot hashing/index construction
is count-bounded but not cooperatively interrupted by this callback. No thread,
background task, additional dependency, or transport authority is introduced.

## Agent-facing query

The existing structured `fortress.query` dispatcher now accepts `dominators`.
The query schema is composed into the runtime's existing query union; no new
MCP tool or native effect is added. Supply this object as the tool's `query`
argument alongside the existing session identifier:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "dominators",
    "root": "1",
    "root_generation": 1,
    "edge_kinds": ["requires"],
    "direction": "outgoing",
    "target": "5",
    "target_generation": 1
  }
}
```

The IDs above are illustrative, not stable identifiers from a live fortress.
Use the IDs and generations from the selected session's current projection.
`expected_anchor` may additionally pin the complete observation anchor.

Omit both target fields to request the full `dominator_tree` view. Supplying
both selects `target_chain`: it returns the unavoidable-node records and impact
counts for that objective, with total reachable/unreachable counts and the same
complete-analysis witness. It does not emit unrelated tree rows or the full
unreachable list (`unreachable` is null). `analysis_complete: true` and
`truncated: false` refer to complete analysis and the requested view, not to
emitting every graph vertex. A present but unreachable target reports
`unreachable_in_projection`; a missing endpoint is an invalid request, not a
proof of absence. Generation mismatches fail with `Conflict` even for an
unreachable target.

The analysis witness describes the entire rooted graph; different target views
can therefore share it. It is not a digest of the serialized presentation.
Whole-fortress Query authority is checked before parsing and again during
analysis and before return. The endpoint enforces the caller's foreground wall
deadline, entity budget, response byte/token budget, and the bounded graph work
allowance. Active-watch metadata reservation and complete-turn publication use
the existing dispatcher; failed rendering does not register work or mutate state.

Ten additional Rust regressions cover semantic dispatch, target focus, reversed
relations, generation/anchor fences, scope/cancellation/expiry, malformed inputs,
output/deadline budgets, failed publication, and runtime schema composition.
These Rust tests have **not been executed in the authoring environment**.
Python JSON Schema checks passed six accepted and sixteen rejected request cases;
source checks also matched the registered edge selectors and dispatcher/schema
linkage. Those checks do not establish Rust compilation or MCP transport success.

## Implementation and validation status — 2026-10-13

Source includes 11 Rust tests: diamond reconvergence, reversed direction, explicit
edge filtering, cycles/self-loops/multiedges, witness invalidation, exact budget
boundaries, invalid inputs, cooperative aborts, a 4,000-node chain, forged chain
reconstruction, and exhaustive vertex-removal differential testing of all simple
directed graphs through four vertices at every root.

Executed in the authoring environment:

```text
python scripts/check_dominance_reference.py
16,585 simple directed graph/root cases: passed
59,049 multigraph/root cases, including self-loops: passed
```

That script compares a Python mirror of the algorithm with an independently
structured vertex-removal reachability oracle. It validates the algorithm design;
it **does not execute the Rust implementation**. The reconstructed existing Rust
file was verified against its Git blob SHA before editing.

Rust compilation, rustfmt, Clippy, workspace tests, full qualification, and native
DF/DFHack checks were **not run**: the authoring container has no Rust toolchain or
game runtime. No live tuple, compatibility floor, production protocol map, or
release-admission status changes. Required follow-up on a Rust-equipped host:

```text
cargo test -p dfmcp-world --test graph_dominance
cargo test -p dfmcp-mcp dominance_query
./scripts/verify.sh
```
