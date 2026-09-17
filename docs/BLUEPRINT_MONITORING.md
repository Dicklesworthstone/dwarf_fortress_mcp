# Blueprint-to-watch monitoring

Blueprint previews can now supply an explicit foreground watch request over the exact
layout through the optional `monitor` object. This connects spatial planning to the
shared `terrain_count` engine described in `TERRAIN_WATCHES.md`: an agent need not
reconstruct every room, doorway and corridor or create a predicate per tile.

The preview remains read-only. It registers no watch, reserves no resources and
executes no designation. The existing eleven top-level MCP tools are unchanged.

## Request and register

Add monitor options to the existing `blueprint_layout` query:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "blueprint_layout",
    "origin": [10, 10, 5],
    "template": {
      "kind": "bedroom_cluster",
      "rooms_count": 20,
      "room_size": [3, 3]
    },
    "monitor": {
      "key": "bedroom-floor-goal",
      "deadline_tick": 42336100,
      "poll_interval_ticks": 10,
      "stable_observations": 2
    },
    "limit": 8
  }
}
```

The game tick is illustrative: choose a future deadline inside the current session's
negotiated horizon. The key is 1..64 UTF-8 bytes without NUL. Cadence defaults to one
game tick and accepts 1..1,000,000; stability defaults to two and accepts 1..64.
Unknown options, out-of-horizon deadlines and out-of-map geometry fail explicitly.

The result's `monitoring` contains `session_id`, `tool="fortress.query"` and a complete
`watch_request` query envelope. Submit that envelope as the `query` argument of
`fortress.query`, using the supplied session ID, to register deliberately. Its
`expected_anchor` binds the preview capture. A newer observation makes that request
stale; obtain a fresh preview rather than silently dropping the anchor.

Normal watch registration independently checks current authority, definition identity,
capacity, game-time horizon and complete response size. Existing key conflicts are
not bypassed. The resulting watch handle works with `poll_watch`, `await_watch`,
`await_watches`, cancellation and release. With the existing paired spatial/1.8
journals configured, its definition uses normal durable-watch custody and recovery.
A preview proposal itself is not a durable record.

## What the watch establishes

The versioned `dfmcp.blueprint-shape-monitor/1` policy declares **shape goals**:
mining templates request `floor`; channel templates request `empty` or `ramp_top`.
These are explicit preview-model criteria, not a certified native dig-mode completion
registry. The result repeats `target_shapes` so the caller can inspect the choice.

Every excavation coordinate must match. The mask includes room interiors, doorways,
row corridors and connecting corridors. A moat's enclosed interior and reserved
crossing are excluded, exactly as in the layout. Overlap is not introduced, and
negative/out-of-map corridor coordinates are neither omitted nor clamped.

A shape already present before registration can satisfy the predicate. Satisfied
monitoring therefore does **not** prove that mining occurred, an agent caused the
change, a native job completed, a bridge was built, access exists, or a site is safe.
It does not check liquid, structural support or aquifers. Such additional observed
conditions must be declared explicitly rather than inferred from a floor shape.
Native plan/effect completion remains disabled on this spatial read-only path.

The terrain engine requires coherent visible evidence at every coordinate. Hidden,
unallocated and outside-capture tiles remain unknown. Inspect the preview's coverage:
a fixed-region runtime does not automatically expand its capture, so reopen with a
region covering the complete footprint before registering a goal that needs those
tiles. Unknown evidence resets stability; repeated reads of one anchor add no sample.

## Pagination and limits

The optional request is part of the complete response budget and returned intact on
every page. One whole row and the complete proposal must fit or the query fails. No
half-mask, truncated watch or zero-progress continuation is returned. Page identity
covers the proposal and its key, deadline, cadence, stability, session and anchor.
Changing page width is allowed; changing monitor settings requires a fresh query.
Omitting `monitor` or setting it to null preserves the prior preview shape and page
identity. Generated masks inherit the existing 64-part/16,384-tile layout bounds.

## Validation status

Eight Rust tests are added: four proposal tests and four tests through actual
spatial/1.8 tool handlers with an injected coherent source. They cover mask identity,
moat exclusions, authority/options/map bounds, explicit registration and stable wait,
stale previews, complete 8,192-byte pagination with unchanged existing watch evidence,
and rejected output without registration. They are **not compiled or executed here**;
Rust, Cargo and rustfmt are unavailable.

The executed Python contract reference accepts 30 schema examples and rejects 110
malformed examples. Twelve separate reference cases check deadline, byte-length and
cadence/stability relationships beyond the schema. Its independent 24-room example
has 55 disjoint parts, 367 coordinates, 523 request nodes and 2,525 request bytes.
These sizes cover the generated request only, not the complete MCP/Agent Turn result.
The reference does not execute Rust, watch state transitions, native DFHack, stdio,
filesystem recovery or full-repository qualification.

Run the retained reference with:

```bash
python3 scripts/check_blueprint_monitor_reference.py
```

No dependency, native protocol, archive whitelist, production runner or admission
registry is changed. The separate full Rust and native/live evidence gates remain.
