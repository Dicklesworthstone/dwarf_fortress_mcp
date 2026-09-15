# Durable coherent spatial history

The spatial/1.6 development server can retain complete coherent captures and
replay an exact past capture for ordinary queries, candidate routes, and
route-aware inventory allocation. Historical reads never replace the live world
or evaluate current watches against old facts. This is source-present,
unadmitted development functionality, not native or production qualification.

## Configure the existing spatial server

Configure `DFMCP_SPATIAL_JOURNAL` in the MCP server's environment before opening
a session. The value must be an absolute normalized file path under an existing
real private `0700` directory. Existing files must be single-link regular `0600`
files owned by the directory's owner. The existing private Unix backend retains
an exclusive writer lock. No archive path or repair option is an MCP argument.

With `DFMCP_SPATIAL_TOKEN` already configured for the existing spatial bridge:

```bash
install -d -m 700 "$HOME/.local/state/dfmcp"
DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_6=1 \
DFMCP_SPATIAL_JOURNAL="$HOME/.local/state/dfmcp/spatial-observations.bin" \
cargo run --locked --bin dfmcp-live-spatial-dev-server
```

Use the same fixed region when reopening a spatial archive. Open the session as
usual, with Query and Observe authority. Bootstrap first acquires a fresh native
capture. The journal then reconstructs the retained spatial generation chain and
appends that fresh capture, unless it is an exact heartbeat. The recovered state
becomes visible only after a successful append/sync. Old session IDs, grants,
baselines and watches are not restored.

An offline-only startup mode is not implemented. An already open session whose
native source becomes fenced may still read its verified archive under current
Query authority. A second active session cannot concurrently write the same file;
the current runtime releases its session-owned file lock at process exit.

## Find and query retained captures

Use `fortress.query` with `mode="history"`, or this structured query argument:

```json
{
  "schema": "dfmcp.query/1",
  "query": {"kind": "history", "limit": 8}
}
```

Each metadata row identifies `record`, `record_digest`, `source_digest`, full
`anchor`, predecessor digest, and encoded record size. Use the actual returned
record number and digest in a `historical_query`; these identities are not
interchangeable with game ticks, query continuations, or session handles.

For example, the following is the request shape for an archived spatial
allocation. The digest shown is illustrative; replace it with the value from the
selected history row, and choose an origin inside the archived region.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_query",
    "record": 1,
    "record_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "query": {
      "kind": "spatial_inventory_plan",
      "origin": [0, 0, 5],
      "quantity_unit": "stack_units",
      "demands": [{"key": "wood", "units": 1, "item_types": ["WOOD"]}],
      "limit": 8
    }
  }
}
```

Historical nested query kinds are limited to `entities`, `inspect`, `traverse`,
`dependencies`, `aggregate`, `search`, `map_route`, and
`spatial_inventory_plan`. Their existing schemas and work limits still apply.
Watches, captures, baseline changes, live refreshes, effects, and nested history
are rejected. Historical allocation is still a declared stack-unit/terrain-model
analysis, not proof of native requirements, actual unit navigation, safety,
reservations, or successful production.

An archived allocation's `route_query` is itself a complete historical envelope
pinned to the same record and digest. Follow it as returned. It cannot silently
switch to the current map even when the live observation has advanced.

History listings use opaque `sh1` continuations bound to the current session,
live anchor, archive identity and archive head. Page width may change. An append
invalidates an old listing continuation. Historical result pagination uses the
nested query's existing continuation, with the same outer record and digest.
No read consumes a page, advances a baseline, or refreshes the bridge.

## Anchors, authority and active work

Historical responses name both the archived `anchor` and `current_live_anchor`.
The Agent Turn identifies a historical source, its journal/record/source digests,
partial temporal continuity, and `current_freshness_proven=false`. Current active
watches remain attached using the current context; their predicates and retained
records are not replayed or advanced.

Authorization is checked at the current live anchor before archive access. A
past capture does not revive expired capability grants. Native replay uses the
session's negotiated capture-byte allowance, not its smaller result-page budget.
Selected archived sources must also fit current roster and fixed-region limits.
Result producers reserve complete Agent Turn metadata and current work before
emitting whole rows. Failed result rendering does not change archive or live
state. A preceding successful live observation may already have been persisted;
response failure does not undo that observation.

## Durability and recovery

The common `ObservationJournal<S, P>` engine has sealed fixed codecs for
operations/1.3, operations/1.4 and spatial/1.6. The original
`OperationsJournal<S>` and `open_private_journal` APIs still select only 1.3.
The legacy header, record layout and digest domains are retained. New 1.4 and
1.6 files have separate magic and incarnation domains. Opening another profile's
file fails before incomplete-tail repair; this is not a migration facility.

Spatial records retain the complete composite source. Replay reconstructs every
record through `LiveSpatialState` and verifies its recorded anchor, preserving
terrain, inventory, relationships, retired identities and generation changes as
one version universe. `state_at` returns a separate typed reconstructed state;
it does not overwrite the journal's current state.

Changed captures are written and synced before publication. A partial write or
failed sync fences the journal and native source without publishing the candidate
anchor. A complete record left by an uncertain sync may be recovered on reopen;
the failed operation is not described as having definitely written nothing.

The default recovery mode preserves an incomplete suffix and refuses opening.
The operator-only `DFMCP_SPATIAL_JOURNAL_REPAIR=1` setting permits truncating an
incomplete trailing record after a verified prefix. Complete corrupt frames,
invalid length checksums, wrong profiles, and nonreproducible anchors are not
silently discarded. This uses the existing Unix custody/locking backend; it is
not hostile-host protection, authenticated provenance or an anti-rollback floor.

Retention defaults to 64 MiB and 1,024 changed captures, with explicit refusal at
capacity. There is no automatic pruning, rotation, compressed delta storage,
checkpoint index, or constant-time random access. Replays walk the retained
prefix under byte/entity and cooperative wall-time limits; filesystem operations
are not claimed to have a hard cancellation bound.

The 1.4 archive codec is available as a Rust library API. The separate paged-1.4
MCP entry is not wired to a journal by this change. Existing read bridges,
production admission and the separately developed pause-control profile are
unchanged.

## Validation status

Fourteen Rust regression scenarios are added and registered: eight journal/profile
cases and six Unix tests of the actual spatial query handlers. They cover exact
replay, profile isolation, legacy framing, retirement/reappearance, resets,
partial writes and failed syncs, current authority, large captures, restart,
disconnection, historical route drill-downs, changed storage and 8,192-byte
historical allocation pagination with current work.

These Rust tests were not executed: no Rust compiler, Cargo or rustfmt was
available in the editing environment. No Rust compilation, Clippy, stdio,
filesystem crash campaign, live DFHack execution, or repository qualification is
claimed. JSON Schema meta-validation and 50 checks of the new wrapper fields and
stateless-kind gate passed; the complete composed query schema was not executed.
Sixteen independent JSON-length cases checked that archived route wrappers fit
within the original route envelope size. None of those checks executes Rust.
