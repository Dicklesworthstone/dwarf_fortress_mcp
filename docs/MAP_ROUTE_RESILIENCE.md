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
