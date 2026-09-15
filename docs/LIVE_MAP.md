# Bounded terrain observation and candidate routes: map/1.5

The map profile connects a bounded DFHack terrain read to canonical tile facts,
typed queries, endpoint comparisons, foreground conditions, and candidate route
inspection. It is an explicitly unadmitted development profile, not a live
qualification or a game-control path. All existing bridge generations remain
unchanged.

## Entry and acquisition

The native plugin is `bridge/dfhack-map-v1_5/dfmcp_map_v1_5.cpp`; its supplied
CMake target is `dfmcp_map_v1_5`. Build it within a named DFHack source checkout
with the real generated DF headers and protobuf toolchain. The package is
`dfmcp.map.v1_5`. Only `Handshake` and `ReadObservation` are registered, both
with flags zero, so the region is acquired under one native RPC suspension.

Set an identical operator-selected 32..256-byte `DFMCP_MAP_TOKEN` in the DFHack
and MCP processes. It is not an MCP argument and never appears in responses.
With the token already configured, the development binary is:

```bash
DFMCP_ALLOW_UNADMITTED_MAP_V1_5=1 \
DFMCP_MAP_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-map-dev-server
```

The public Rust library entry repeats the development gate. Other `DFMCP_*`
variables, including production admission, old-profile credentials and journal
configuration, are refused. Endpoints must be numeric loopback addresses with a
nonzero port. Native framing, strict protobuf decoding and absolute call
Deadlines reuse the owned existing transport rather than introducing a runtime.
Bootstrap and the first observation are separate bounded calls.

`fortress.open_session` takes one fixed region and optional output-token,
wall-time and read-capability narrowing. Coordinates are zero-based map tile
coordinates, not screen pixels or block coordinates. Size is an extent, not an
inclusive upper corner. For example:

```json
{
  "region": {"origin": [0, 0, 0], "size": [32, 32, 4]},
  "max_output_tokens": 8192,
  "max_wall_millis": 5000,
  "requested_capabilities": ["observe", "query", "doctor"]
}
```

Each side is 1..128 tiles, volume at most 16,384 tiles, coordinates below 32,768,
and the entire region must fit the actual observed map dimensions. No clipping
or partial-success substitute is performed. Native payloads are at most 1 MiB;
output-token limits are 2,048..65,536 and call deadlines 1..60,000 milliseconds.
At most two sessions are retained. Changing region requires opening another
session; session storage is currently released at process exit, not via a close
operation. Neither that lifecycle nor whole-map streaming is implemented here.

## Presence and fields

The producer calls `Maps::getTileBlock`, never `ensureTileBlock`. It does not
allocate terrain blocks, reveal hidden cells, change designations, pause/unpause,
move units, or mutate saves.

Every cell has exactly one source presence tag:

- **Visible:** actual tile type, normalized shape, liquid depth/type, traffic,
  dig designation, building/unit occupancy, cached walkable-region ID and both
  raw temperature values are encoded.
- **Hidden:** only its hidden presence tag is encoded. No terrain, occupancy,
  liquid, walkability or temperature attributes are serialized. The canonical
  fields are `Redacted`, not zero-valued known facts.
- **Unallocated:** only the missing-block presence tag is encoded. The canonical
  attributes are `Unknown`, not empty space or an absence proof.

The canonical entity kind is `tile_feature`, with `position`, `visibility`,
`tiletype`, `shape`, `liquid_depth`, `magma`, `traffic`, `dig_designation`,
`building_occupancy`, `unit_occupancy`, `walkable_region`, `temperature_1_raw` and
`temperature_2_raw`. Unit occupancy is a two-bit observation: standing and
on-ground presence. It does not expose a unit identity. Temperatures and native
numeric enum fields are raw game values, not converted physical units.

Shape tags are explicitly normalized to other, empty, wall, floor, ramp,
ramp-top, up-stair, down-stair and up/down-stair. An unknown shape is not a floor.
The cached walkable-region ID is a raw observation, not a fresh pathfinder result.

Tile handles identify physical map cells; they are not IDs of any dwarf or
building that might occupy the coordinate. A tile's observed shape or visibility
can change without replacing its coordinate identity. Handle generations advance
on epoch resets; revisions advance with observations. Callers should use returned
handles and generations rather than constructing them.

## Publication and provenance

The Rust decoder checks count/volume agreement, bounds, enum tags, strict
Booleans, UTF-8, manifests, exact requested region and trailing bytes. One source
digest seals all facts from the read. Materialization finishes before the visible
snapshot is replaced. A malformed read preserves the last published anchor and
fences the source.

Exact repeated observations are heartbeats. Changed observations advance the
sequence. Clock regression, bridge-generation change or map-dimension change
advances the epoch. World, site, software or requested-region changes require
reopening. Missing or hidden cells stay explicit throughout these transitions.

This is **not a join with operations/1.3 or operations/1.4**. Matching fortress IDs
or coordinates do not authorize combining independently acquired snapshots into
one apparently coherent world. This profile has no citizen, inventory, job,
material-requirement, or durable-journal integration.

## Query and monitor

The existing eleven tool names are retained. `fortress.observe` and
`fortress.wait` acquire one region observation without controlling game time.
Queries ordinarily inspect the last published snapshot and do not refresh.
The returned game tick is observation time, not a promise of current game state.

`fortress.query` modes are `summary`, `tiles`, and `schema`. Structured queries
reuse the sixteen existing variants: typed entity filtering/inspection, graph
inspection, aggregates, lexical search, baselines and foreground watches. Only
the map runtime adds the seventeenth variant, `map_route`.

For example, pass this envelope in the query argument to group observed liquid
depths, preserving redacted/unknown fact presence rather than calling it dry:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "aggregate", "kinds": ["tile_feature"],
    "group_by": {"kind": "field", "field": "liquid_depth"}
  }
}
```

The usual `capture`/`changes` operations compare selected terrain facts at two
observations. A field watch can monitor an observed liquid depth, designation,
shape or visibility using a returned tile ID and generation. An `await_watch`
validates the handle before I/O, refreshes at most once, reauthorizes at the new
anchor and evaluates the condition. Terminal retries skip the bridge read.

Known zero liquid depth is only that fact, not a safe-route or safety proof.
Hidden/unknown terrain cannot satisfy field comparisons, including negation.
After source failure, local listing, cancellation and release remain available
with stale source continuity. Rendering must fit the complete Agent Turn before
baseline/watch updates are published. A preceding successful observation may
already have advanced the world; response failure does not undo that read.

## Route model

`map_route` runs deterministic, unit-weight breadth-first search with stable
flattened tile-index tie breaking. It supports horizontal cardinal neighbors and
vertical moves with complementary observed stairs on both endpoints.

A candidate tile must be visible, dry, have no building or unit occupancy, have
a nonzero native cached walkable-region value, and be a floor or stair. Unknown
and hidden cells, empty space, walls, ramps, ramp tops and unsupported shapes are
excluded. The policy intentionally omits diagonals, doors, swimming, flight,
climbing, digging, unit abilities, traffic costs and temperature safety.

Coordinates below are illustrative and must lie in the session's region:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "map_route",
    "start": [10, 10, 1], "goal": [20, 20, 1],
    "limit": 16, "max_work": 100000
  }
}
```

Statuses distinguish an excluded endpoint, a found candidate and no route in
this observed model. All explicitly report `unit_path_proven=false`,
`safety_proven=false` and `global_unreachability_proven=false`. The endpoint and
route policy are deliberately conservative: an occupied starting cell is also
excluded. Routes that require unobserved space or unsupported mechanics may
exist in the game even when this model has no path.

Search work is bounded by one million counted node/edge operations. Exhaustion
returns an error, never a negative result. The result includes visited tiles,
work units, whether search touched a region boundary, total path vertices and
unit-weight model steps. These are not game-time or travel-time estimates.

Route vertices are output as whole rows. Opaque continuations bind session,
fortress, epoch, sequence, tick, state hash, policy, endpoints and work budget;
page width can change. Retry does not advance the world or mutate a route.
Digest bindings are consistency checks, not authentication. Every page is
independently authorized. Required Agent Turn coverage and active watches are
reserved before route rows consume the remaining output budget.

## Evidence and remaining work

The actual native producer compiled against mocked DFHack/protobuf interfaces
under both GCC and Clang with C++17 and `-Wall -Wextra -Werror -pedantic`. Each
run passed 110 checks and matched an independent 455-byte Python encoder. Checks
include real 16x16 block boundary crossings, hidden-attribute noninterference,
missing blocks, shape normalization, malformed requests, generation changes and
the 16,384-cell capture ceiling. Exact tested producer SHA-256:

```text
b4e148c3827936115ed1b389c50d2d936e01a8bb773cda366ac8f5cc09e0be14
```

Reproduce the producer check without a game:

```bash
python3 scripts/test_live_map_native_mock.py --compiler g++
python3 scripts/test_live_map_native_mock.py --compiler clang++
```

An independent Python design comparison passed 512 exhaustive 3x3 obstacle maps
and 500 seeded 3x3x3 shape maps against distance relaxation. The route schema
passed 64 cases, with 19 accepted and 45 rejected. These are design/schema checks,
not execution of the Rust source.

Seventeen new Rust tests are registered: six route/model, three native
codec/projection, two RPC and six actual-handler tests. They include every
truncated prefix of the native fixture, redaction, atomic resets, shared
baseline/watch refresh, route pagination with current work at 8,192 bytes,
continuation misuse, authority and source failure.

No Rust compiler, Cargo or rustfmt was available in the editing environment.
Rust compilation/tests, Clippy, stdio, real generated DF headers, protobuf
linking, native loading, suspension behavior, live-game correctness and full
repository qualification remain unestablished. No latency or whole-process
memory claim is made. The bounded packet is smaller than its expanded canonical
fact representation.

Still unfinished: a coherent combined terrain/operations/citizen profile, full
unit-specific navigation, mutable region windows, durable terrain history,
spatial planning tied to native effects, and all live game mutations. Old
bridge bytes, dependency pins, production admission and the active migration
bead are unchanged.
