# Observed route redundancy and minimum cuts

GA-FLOW source slice; bead `df-franken-storage-mvcc-54h.4` remains open for the
broader graph/Franken integration. This is read-only cognition over the existing
bounded dry cardinal floor/complementary-stair model, not new native semantics.

## What the model answers

`dfmcp_world::map_resilience::analyze` returns a maximum packing of simple routes
between distinct endpoints and an equally sized minimum separator. Two explicit
modes prevent a misleading resilience claim:

- `edges`: no two returned paths use the same undirected candidate transition.
  They may still share a failure-critical tile.
- `internal_tiles`: no two paths share an internal tile. Both endpoints are
  deliberately shared and are not counted as removable internal tiles.

For nonadjacent endpoints, the internal-tile separator consists entirely of
internal tiles. Adjacent endpoints require a special statement: no removal of
internal tiles can remove their direct transition. The certificate therefore
contains that direct edge plus a minimum set of internal tiles separating the
other routes. This mixed separator is NOT presented as a pure vertex cut.

Every path includes both endpoints. Paths are lexicographically ordered by
flattened tile indices; cuts are ordered tiles first, then ascending undirected
endpoint pairs. The source-side residual cut selects the deterministic result.
These indices belong only to the exact map region and are never durable IDs.

Excluded endpoints return `endpoint_excluded`, not certified zero redundancy.
Eligible but disconnected endpoints have zero paths and an empty cut only in
this captured model. Outside endpoints and identical endpoints are refused.
Hidden/unallocated cells, liquids, occupancy, unestablished walkability and
unsupported shapes do not become traversable. Vertical edges require observed
complementary stairs on both endpoints, matching `MapRegion::route`.

## Algorithm and certificate

All network capacities are one; there are no guessed throughput values, floats
or magic infinite capacities. Tile mode splits each internal tile into an
entry/exit pair with unit capacity. Endpoint splits are omitted, and a direct
endpoint edge also has unit capacity, so one direct path cannot be duplicated.
Breadth-first residual augmentation supports rerouting earlier choices, unlike
greedily removing one shortest path and looking for another. At most six paths
can leave a six-neighbor endpoint.

Edge mode removes opposing directed circulation before extracting undirected
edge-disjoint paths. Remaining circulations are not presented as routes. A
movement-edge cut in the split network is mapped onto an internal endpoint;
only the direct source-goal edge remains an edge in tile mode.

Before returning, a separate check verifies simple valid paths, the declared
disjointness, canonical cut elements, equal packing/cut cardinalities, and actual
disconnection after the cut. It does not trust the residual graph. Feasible
packing gives the lower bound and the equal separator gives the upper bound.
`verify_certificate` also permits rechecking the returned mathematical witness
against a supplied map. It does not authenticate source custody or work counters.

Auxiliary storage is O(tiles + edges), including bounded route materialization.
There are at most six augmentations and six extraction searches. Every phase,
including certificate checks, consumes the same structural work budget (at most
1,000,000 units). Exhaustion returns an error, never a partial minimum cut.
`analyze_with_check` checks caller-owned cancellation/deadline before work,
every 128 charged units and before publication. No recursion or detached work.

## Meaning and non-authority

One independent route is not an evacuation guarantee. The model omits native
unit movement rules, diagonals, doors, ramps, swimming, flying, danger and routes
outside the region. Unit occupancy can exclude an endpoint. A cut edge is a
graph transition, not a DF bridge building or a proposed game mutation.

Redundancy counts are not dwarves per tick, hauling throughput, structural
safety or permission to demolish/cut anything. Fresh observations, authorization,
game-semantic feasibility and independently witnessed planning remain required
before an action. No protocol, capability or production admission is widened.

## Evidence posture — 2026-10-13

Source includes 15 Rust regression tests: both disjointness modes, shared
articulation tiles, adjacent endpoints, residual rerouting, exclusions, stairs,
all 512 three-by-three obstacle masks, independent cut enumeration, exact work
limits, cooperative aborts, forged certificates, an 8,255-tile snake in a
maximum-size region and six independent stair-lattice routes.

Executed in the authoring environment:

```text
python scripts/check_route_resilience_reference.py
42,600 small undirected graph/endpoint/model cases passed
256 eligible obstacle-map/model cases passed
articulation, adjacent endpoint, long corridor and six-path examples passed
```

This Python script mirrors the design and independently enumerates separators;
it does not run the Rust source. Rust compilation, Rust tests, rustfmt, Clippy,
workspace qualification and native DFHack validation have not run in this
Rust-toolchain-free authoring environment. Required actual-source validation:

```text
cargo test -p dfmcp-world --test map_resilience
./scripts/verify.sh
```

## Shared spatial query integration

The existing spatial structured query dispatcher now accepts
`map_route_resilience` in the spatial/1.6 and citizen-inclusive spatial/1.8 read
paths. Schema discovery composes the new closed variant into the existing
query union; the eleven-tool surface is unchanged. Supply this value as the
existing `fortress.query` tool's `query` argument, with the session identifier
as usual:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "map_route_resilience",
    "start": [0, 0, 5],
    "goal": [4, 0, 5],
    "disjointness": "internal_tiles",
    "section": "cut"
  }
}
```

These coordinates are illustrative; both endpoints must be distinct positions
inside the session's captured region. `expected_anchor` can additionally pin
the complete anchor. The caller must hold current whole-fortress Query authority;
scoped or use-limited grants are not widened, and a continuation supplies no
authority. The current source fence and complete Agent Turn/watch budget rules
still apply at the owning server. No extra source read, native method, mutation,
watch, reservation or background task is created.

The default `cut` section returns at most six separator rows with the complete
analysis summary and commitment to every path. `paths` returns individual
positions, with path ID, position ordinal and total path length. `all` returns
cut rows first and then the same path-position stream. Long routes therefore
remain pageable even when an entire route would exceed a single-row budget.
`limit` is 1..128, default 16; the existing byte-aware paginator may narrow it.
Pages recompute the bounded deterministic analysis rather than retaining a new
mutable cache or independent generation.

`analysis_complete` and `certificate_verified` refer to the underlying analysis,
not whether all path positions were displayed. `truncated` refers to the selected
section's presentation. An excluded endpoint has null path/separator counts and
both analysis flags false; it is not converted to zero. Eligible endpoints with
no route have a checked zero only in this bounded observed model.

The certificate reports packing and separator cardinalities and whether its cut
contains the direct endpoint edge. Tile mode with adjacent endpoints explicitly
reports `mixed_internal_tile_and_direct_edge_cut`; `pure_internal_tile_cut_size`
is null in this case and in edge mode. Both endpoint deletion and claims of
native safety/throughput remain disabled.

The structural digest binds the full source anchor, source bytes' digest,
session identity, both endpoints, region, disjointness and route policy, supplied
work bound, actual structural work and the complete ordered path/cut witness.
Changing the section or page limit preserves that witness digest. The analysis
identity additionally binds the selected section. Continuations bind it through
the existing session/query/capture token: changing source, session, endpoint,
mode, section or work policy refuses rather than silently starting a new page.
A page limit can change without skipping positions. Query authorization and the
foreground wall deadline are checked during analysis and rendering and again
before return; final source identity and snapshot hash are rechecked.

Thirteen additional actual-source Rust regressions exercise the shared query
dispatcher with strict spatial/1.6 and spatial/1.8 fixtures, canonical publication,
compact versus expanded views, complete paging, current authority, anchor fences,
continuation invalidation, exclusions, direct-edge semantics and budgets. They
remain **unexecuted**, as do the fifteen core regressions. The verifier also now
bounds caller-supplied path lengths before lexicographic comparisons, covered
by an additional mutation in the existing forged-certificate test.

Python JSON Schema validation passed 15 accepted and 32 rejected request shapes.
The original shared dispatcher was reconstructed and its Git blob SHA verified
before making the additive module, variant, routing and schema changes. These
are source/schema checks, not MCP transport or Rust execution evidence.

Additional actual-source validation on a Rust-equipped host:

```text
cargo test -p dfmcp-mcp 'spatial_queries::resilience::tests'
```
