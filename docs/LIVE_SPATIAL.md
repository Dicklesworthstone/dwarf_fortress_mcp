# Coherent spatial/1.6 observations and route-aware inventory

The spatial profile observes jobs, buildings, items, job-item attachments and one
bounded terrain region during **one native RPC suspension**. It serializes that
capture once, then transfers immutable pages. It never joins independently timed
operations/1.4 and map/1.5 observations.

The source is an explicitly unadmitted development path. Source presence, mock
compilation and synthetic fixtures are not real DFHack compatibility, live-game
qualification or authority to start a production process. Existing native
profiles, dependency pins and the production admission map are unchanged.

## Components and entry

The new native target is `dfmcp_spatial_v1_6` under
`bridge/dfhack-spatial-v1_6/`. Its `CMakeLists.txt` and protobuf envelope must be
built within a compatible DFHack checkout with actual generated DF headers and
protobuf linking. That real native build has not been established by this tranche.

The data-only encoders live in `bridge/common/spatial_capture.h`; the existing
`retained_snapshot.h` owns immutable serialized bytes, never native pointers.
The Rust implementation uses `live_spatial`, the closed spatial RPC client, the
world reachability field, and `spatial_inventory`. The registered MCP binary is
`dfmcp-live-spatial-dev-server`.

Configure the same 32..256-byte `DFMCP_SPATIAL_TOKEN` in the DFHack and MCP process
environments. Credentials are never tool arguments, source facts or query output.
With that token already configured, the development entry is:

```bash
DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_6=1 \
DFMCP_SPATIAL_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-spatial-dev-server
```

The public library entry repeats the exact opt-in check. All other `DFMCP_*`
settings, including other profiles' credentials, journals and production admission
state, are refused. Sessions have their own process-scoped family and a two-session
retention limit. Old or raw numeric handles cannot mint a current session.

Open a session with a region that lies inside the loaded fortress map. This
example is illustrative, not a claim that these coordinates suit any fortress:

```json
{
  "region": {"origin": [0, 0, 5], "size": [4, 4, 1]},
  "max_output_tokens": 8192,
  "max_wall_millis": 5000
}
```

Other optional open arguments are `max_items`, `max_capture_bytes`, `page_bytes`
and `requested_capabilities`. Only Observe, Query and Doctor can be granted.
The region remains fixed for the session lifetime.

## One capture and one canonical world

The immutable frame contains `DFMS1600`, a length-delimited `DFMO1400` operations
payload and a length-delimited `DFMM1500` terrain payload. Those embedded formats
are reused codecs, not claims that separate services supplied the data.

The decoder requires exact agreement on world folder, site, year, tick, pause
state, bridge generation and software identity. Both components must pass all
existing roster, reference, containment and terrain validation before publication.
Every canonical fact and relationship then receives the same spatial source
digest and one combined observation anchor.

An ordinary change in either component advances the shared sequence. Observed
retirement/reappearance advances entity generation. A clock, bridge generation,
map-dimension or native identity-horizon reset advances the shared epoch. Failed
validation leaves the previous combined snapshot intact; no half-world is exposed.

Hidden terrain attributes remain redacted. Missing blocks remain unknown and are
not allocated by observation. The complete item roster does not establish complete
terrain coverage or turn an unobserved location into a known accessible tile.

## Route-aware declared allocation

Pass this envelope in the `query` argument of `fortress.query`, with the returned
session ID supplied separately:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "spatial_inventory_plan",
    "origin": [0, 0, 5],
    "quantity_unit": "stack_units",
    "demands": [
      {"key": "logs", "units": 10, "item_types": ["WOOD"]}
    ],
    "limit": 8
  }
}
```

Use observed item type keys and native raw material identifiers. Each demand can
also specify `subtype`, `material_type` and `material_index`; a material index
requires a material type. Demands are explicit caller declarations, not inferred
DF job recipes or a complete implementation of native material-filter semantics.

The analysis performs one bounded reachability traversal, not one path search per
item. It uses the same dry, unoccupied cardinal floor/stair model as map routes,
with complementary stair endpoints required for vertical movement. An excluded
origin produces an error rather than an apparently valid zero-supply result.

Container ancestry is resolved once. A contained item's candidate position is its
observed outermost ground container's raw position. Disallowed item flags, actual
job attachments and explicit building holders propagate conservatively through
container ancestry. A root without an established ground position is excluded;
a building footprint is not substituted as a guessed pickup location.

Observed items receive disjoint classifications: item/container policy exclusion,
zero-sized stack, unestablished ground location, outside the observed region,
no candidate route within the model, no declared demand match, or candidate supply.
These classifications do not prove global inaccessibility or native job blockage.

Eligible stacks enter the existing integral max-flow allocator. Shared supply is
not counted twice, and residual rerouting avoids the basic greedy-allocation trap.
Results carry flow/cut equality and a joint deficient-demand witness when the
declared model is short. This maximizes allocated units; it does **not** minimize
hauling distance, implement reservations, or produce a commit-compatible plan.

Assignment rows include item and outermost-container IDs with generations, the
candidate position and path length. Copy a row's `route_query` envelope to inspect
its candidate path. It includes the exact expected anchor, so a subsequent capture
cannot silently substitute a different route or supply world.

A direct route request is also available:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "map_route",
    "start": [0, 0, 5],
    "goal": [1, 2, 5],
    "limit": 16
  }
}
```

Candidate routes are not unit-specific paths, safety proofs or travel-time
predictions. Ramps, doors, diagonals, swimming, flight and outside-region detours
are not modeled. A failed candidate search proves only the bounded model result.

## Monitoring, pagination and authority

The runtime retains the eleven tool names and the existing sixteen semantic query
variants. Convenience modes are `summary`, `jobs`, `buildings`, `items`, `tiles`
and `schema`. Spatial schema discovery adds `map_route` and
`spatial_inventory_plan` without changing other profiles' schemas.

Inventory baselines and terrain watches operate at the same combined anchor.
For example, capture selected stack counts, register a liquid-depth condition,
then `await_watch` can acquire one capture that updates both domains before watch
evaluation and a subsequent baseline comparison. Terminal watch retries skip I/O.
Query and Observe authority are rechecked after the refresh advances the anchor.

Route and allocation results paginate at whole-row boundaries. Their opaque
`sp1` continuations bind session, complete anchor, source digest, normalized query,
policies and work budget. Page width can change without changing result identity.
The full Agent Turn and current active watches remain inside the output budget.
Cursor digests are consistency bindings, not authentication or capabilities.

Failed acquisitions fence the source and preserve the prior published world.
Local watch and baseline management remain available with explicitly stale source
continuity. Existing local-state changes still publish only after complete response
rendering succeeds. A preceding successful capture is not rolled back by a later
rendering failure.

## Bounds and remaining limitations

Native ceilings are 4,096 jobs, 4,096 buildings, 65,536 items, 65,536 attachments,
16,384 terrain cells and 16 MiB for the complete composite frame. Each terrain side
is at most 128 cells. Pages range from 16 to 256 KiB. One absolute acquisition
deadline covers all pages and the required release acknowledgement.

The unchanged cache retains at most four captures and 32 MiB of serialized payload
bytes, with fixed 120-second expiry and world-reset invalidation. Region identity
is part of cache ownership. This byte ceiling excludes temporary capture buffers,
projection objects and allocator overhead; total peak memory is not benchmarked.
Initial capture still occurs in one suspension and its latency is not measured.
Snapshot tick means capture time, not transfer-completion time.

Allocation admits at most 32 demands and 32,768 matching candidate stacks, even
though acquisition can observe 65,536 items. Excess candidates cause an explicit
budget refusal, never a silently truncated supply estimate. Work units are bounded
algorithm operations, not a measured wall-time guarantee.

Spatial history and durable watches are not implemented. Existing 1.3 archives
cannot be relabeled as spatial evidence. Citizen coverage, full native requirement
matching, unit navigation, mutable region windows and every live mutation remain
unfinished. All game plan/commit/cancel/checkpoint/restore operations refuse.

## Evidence

Both GCC and Clang compiled the actual producer, capture codecs and cache against
explicit mock DFHack/protobuf interfaces, using C++17 and warnings-as-errors.
Each run passed 786 checks and matched independent Python encoders for an 810-byte
fixture and a 40,000-item, 2,200,457-byte capture across 135 immutable pages.
Inventory, terrain and game-tick changes between page reads did not alter captured
bytes. Ownership, region, release, invalidation, malformed rosters, byte limits,
containment cycles and hidden-terrain noninterference are covered.

```bash
python3 scripts/test_live_spatial_native_mock.py --compiler g++
python3 scripts/test_live_spatial_native_mock.py --compiler clang++
```

The schema passed 59 cases: 20 accepted and 39 rejected. An independent ancestry
design oracle compared 200,721 nodes across 2,000 seeded forests. These checks do
not execute the Rust source or qualify real native behavior.

Twenty-three Rust scenarios are registered: three reachability, six composite
projection, four transport, four inventory and six actual-handler scenarios.
They include complete 8,192-byte allocation pagination retaining an active watch.
They have not been compiled or executed in this editing environment, which has
no Rust compiler, Cargo or rustfmt. No Clippy, stdio, real generated DF headers,
protobuf linking, plugin loading, live-game campaign or full repository
qualification is established. Mock compilation is not native qualification.
