# Terrain connectivity and single-point failures

`map_connectivity` answers two operational questions on the selected coherent
terrain capture: which candidate areas connect, and which single candidate tile
or edge would split one of those areas? It implements the physical-terrain slice
of `FORTRESS_GRAPH_ALGORITHMS.md` sections 6.1 and 6.2. It does not certify native
unit pathfinding, evacuation, a construction proposal or a game effect.

## Query

Pass this envelope as `fortress.query`'s `query` argument, supplying the session
ID separately. Coordinates are illustrative: select real coordinates within the
session's captured region, normally entrance/approach tiles rather than occupied
workshop footprints.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "map_connectivity",
    "landmarks": [
      {"key": "workshop-approach", "position": [1, 2, 5]},
      {"key": "stockpile-approach", "position": [12, 2, 5]},
      {"key": "refuge-approach", "position": [8, 9, 5]}
    ],
    "section": "all",
    "limit": 8,
    "max_work": 1000000
  }
}
```

Optional `expected_anchor` uses the existing complete-anchor comparison. The
query acquires no native capture. To inspect a newly changed map, first observe
through the existing foreground source path. A fenced live source does not
provide fresh connectivity; verified historical queries remain available.

The shared live spatial query dispatcher exposes this query in spatial/1.6 and
spatial/1.8. Spatial/1.8 also exposes it through `historical_query` in live
journal-backed sessions and through archive-only sessions. Historical results
use exactly the selected record's map and generation universe, including mixed
raw/delta archives; they do not replace the current world or sample watches.
The older spatial/1.6 historical allowlist is not expanded by this increment.

For a historical request, discover exact record identities with history listing
and use the existing wrapper:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_query",
    "record": 1,
    "record_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "query": {"kind": "map_connectivity", "section": "bottlenecks", "limit": 8}
  }
}
```

Replace the placeholder digest with the returned record digest. No path,
credential, protocol, new bridge method or new top-level MCP tool is accepted.

## Results

Every result summarizes the complete bounded graph: candidate tiles, model edges,
connected components, articulation tiles, graph bridges, ordered tile-exclusion
counts and structural work. `analysis_complete=true` concerns this derivation,
not completeness of native movement rules or the surrounding world.

`section` selects `all`, `landmarks`, `bottlenecks`, `bridges`, or `components`.
Whole-row pages retain the full summary; omitted detail rows use an ordinary
`sp1` continuation. Continuations bind session, full anchor, source, normalized
landmarks, section, policy and work allowance. Page width may change. No partial
row or partial graph computation is returned when budgets fail.

For `all`, named landmarks appear first, sorted by key. Bottlenecks follow in
highest-impact order, then graph bridges in highest-impact order, then components
by their least flattened tile index. Ties use canonical tile/endpoint order.

A component row gives its representative position, candidate-tile/edge counts,
bounding box and whether it touches any captured-region boundary. Representative
coordinates are labels within this result, not durable region identities.

A bottleneck row gives the candidate tile and the sizes of all remaining pieces
if that one vertex is removed, in descending order. The removed tile is not counted.
`separated_tile_pairs` counts unordered pairs that were connected before and lie
in different pieces after removal. For a five-tile corridor, the central tile
leaves pieces of size `[2,2]` and separates four tile pairs. This measures model
structure, not numbers of stranded citizens, items or endangered lives.

A `graph_bridge` is a cut **edge**, not a Dwarf Fortress bridge building. Its
`side_sizes` align with its two canonically ordered endpoint coordinates. Edge
removal does not remove either endpoint. The impact is the product of the side
sizes, again measured in unordered candidate-tile pairs.

Landmarks have unique ASCII keys of 1..64 bytes; at most 128 are accepted. They
report a component representative only when their tile is a candidate. Hidden,
unallocated, excluded or outside-region coordinates remain `unestablished` with
an explicit reason. No landmarks, or any unestablished landmark, yields null for
`all_connected_in_observed_model`, not vacuous success or a claimed disconnection.
With all landmarks established, this field states whether they share one component
in the named model. It never proves global in-game connectivity or its absence.

## Movement and uncertainty boundary

Vertices and edges deliberately match `observed-dry-cardinal-floor-stairs/1`, the
existing `map_route` model: observed dry floors and stairs with no building/unit
occupancy and nonzero walkability. Horizontal edges are cardinal only; vertical
edges require complementary observed stair shapes. There are no diagonal, ramp,
door, flight, swimming, digging, burrow, traveler-size or threat rules.

Hidden and unallocated cells have no underlying attributes in this analysis.
Exclusion of a candidate route is not native inaccessibility. Conversely, lack of
a cut in this graph is not a safety guarantee: omitted native rules, hazards,
occupancy changes and routes outside the region still matter. A graph deletion is
an advisory counterfactual; it does not edit terrain or authorize demolition.
No occupancy exception is applied to landmarks, unlike specialized workforce
endpoint handling. Select an appropriate observed approach tile explicitly.

## Implementation, budgets and identity

The world-layer engine uses iterative low-link DFS, with fixed degree-six
adjacency and explicit stacks. Component discovery and cut analysis take O(V+E)
structural work; canonical bridge ordering and presentation impact ranking add
bounded sorting. Memory is O(V+E), with at most 16,384 cells and six neighbors per
cell. It does not retain a world per hypothetical deletion or run a flood for
every bottleneck. Small-map tests use those slower deletion floods as an oracle.

`max_work` accepts 1..1,000,000 structural units. These units count bounded graph
operations, not CPU instructions or wall milliseconds. Cooperative checks run
through graph construction, DFS, digest construction and rendering. A complete
result must fit current acquisition, output and deadline bounds. Current Query
authority and exact anchors are checked by the existing spatial dispatcher, with
no Observe grant needed for pure analysis.

`structural_digest` binds the complete result structure, both policies, region,
source digest and snapshot hash using explicit binary field lengths. It is
independent of page size, section, landmarks and incidental request/session IDs.
`analysis_digest` additionally binds the normalized query/session for pagination.
These are identity checksums, not signatures or external correctness certificates.
Existing live Agent Turns retain current active-work metadata without advancing
watches. Historical responses remain historical and use existing custody checks.

## Evidence and status

Fifteen Rust test groups are registered: eight world-layer tests, six spatial-query
tests and one multi-stage actual spatial/1.8-handler test. Coverage includes
exhaustive 3x3 vertex/edge deletion oracles and route comparisons, all vertical
shape pairs, maximum maps/long corridors, impact ordering, landmark uncertainty,
strict parsing, pagination/anchor binding, authority/deadline/output refusal,
8,192-byte Agent Turns with active watches, unchanged paired journal bytes,
fenced-source history, archive reopen and corruption refusal.

**None of the Rust tests has been compiled or executed here.** Rust, Cargo and
rustfmt are unavailable, and container network access cannot retrieve a toolchain.
This is implemented unadmitted development source, not a Rust/runtime, native,
live-game, filesystem-fault or whole-repository qualification claim.

Executed independent checks:

```bash
python scripts/test_map_connectivity_reference.py
python scripts/test_map_connectivity_schema.py
```

The Python low-link reference matched destructive BFS on **5,120 graphs**, with
**57,344 component/vertex/edge checks** (all 4x3 induced grids and all five-vertex
simple undirected graphs). This confirms the reference mathematics, not the Rust
implementation or native movement semantics. The standalone query schema passed
**71 cases: 19 accepted, 52 rejected**; it does not execute Rust schema composition,
unique-key validation, authority, archive replay or MCP serialization.

Focused Rust commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-world map_connectivity
cargo test --locked -p dfmcp-mcp spatial_connectivity -- --test-threads=1
```

No dependencies, native protocols, mutation grants, admission registry entries,
production runners or archive formats are changed. Existing qualification gates
and the unresolved stdio-lifecycle evidence gap remain unchanged.
