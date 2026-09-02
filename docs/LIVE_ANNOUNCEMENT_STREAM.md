# Live announcement stream

Protocol 1.1 adds Dwarf Fortress reports and announcements without widening the mutation boundary.
The stream is an additive observation domain, not a command channel.

## Why announcements come next

Citizen state describes what the fortress is. Announcements describe what has just happened. They
provide high information value at low bridge cost: combat, cancellations, arrivals, deaths,
weather, mandates, production failures, and other game-significant events already pass through the
retained report log.

The implementation must not pretend that this retained log is an infinite history. Dwarf Fortress
may discard old reports. Every batch therefore carries both a cursor and the retained-window bounds.

## Cursor contract

A request names:

```text
announcement_after_id: -1 or a nonnegative report ID
max_announcements:     1..=512
```

A reply names:

```text
oldest_available_id
latest_available_id
requested_after_id
gap_before_window
complete_through_latest
next_after_id
```

`complete_through_latest` proves only that the response reached the latest report retained at the
observation instant. `gap_before_window` means the caller's prior cursor predates the retained
window, so historical continuity is unknown even if the retained suffix was returned completely.

Records are strictly ordered by ascending report ID. Duplicate, negative, out-of-window, or
noncanonical IDs fail closed. Text is valid UTF-8 and bounded to 2,048 bytes per record. A batch is
bounded to 512 records and 2 MiB of canonical bytes.

## Atomicity

Announcement fields are returned by the existing `ReadObservation` RPC. One DFHack RPC executes
under DFHack's internal suspension, so a one-page citizen observation and its announcement suffix
share one observation instant.

When citizen or announcement pagination requires multiple calls, protocol 1.1 requires a paused
fortress. Every page must reproduce the same citizen state, summary fields, bridge generation,
retained-window bounds, and exact continuation cursor. Any drift invalidates the assembly; no
partial capsule or anchor is published.

The publication boundary acquires all required pages first and constructs one combined capsule only
after both the citizen roster and configured retained-announcement suffix are complete. Transport
pagination is therefore not canonical state.

## Single-publication bootstrap

Bootstrap must not read one capsule to derive fortress identity and then read a different capsule to
initialize the adapter. `bootstrap_live_read_adapter_v1_1` acquires exactly one complete combined
capsule, derives the fortress identity and source digest from it, and wraps the source in a primed
replay layer. The adapter consumes that same verified capsule without another underlying bridge
read.

Primed replay preserves the full two-dimensional transport surface:

```text
citizen pagination × announcement continuation
```

It verifies the exact citizen offset, announcement cursor, requested limits, name projection,
summary fields, source manifest, and projected snapshot. Cursor drift, projection drift, manifest
drift, or an attempt to begin an announcement continuation at a nonzero citizen offset fails closed.
The bootstrap remains source-tested and still unadmitted.

## Coverage semantics

The announcement domain is one of:

- `complete_suffix`: every retained report after the requested cursor was returned;
- `partial_suffix`: more retained reports remain and `next_after_id` is the continuation cursor;
- `gap_before_retained_window`: the retained suffix may be complete, but older history was lost.

None of these states proves that no announcement existed before `oldest_available_id`. Even an empty
complete retained suffix does not prove complete fortress history. The canonical world projection
may prove absence only inside the explicitly covered suffix.

The protocol-1.1 projection deliberately publishes two separate domains:

```text
fortress.announcements.retained_suffix
fortress.announcements.history
```

The first can be complete through the current retained high-water. The second remains partial.

## Agent orientation

The announcement briefing is deterministic and authority-free. It can surface:

- a high-severity retained-window gap;
- a medium-severity incomplete retained suffix;
- bounded latest announcement records;
- certified-derived report IDs added between compatible observation batches.

It does not assign game-semantic severity from arbitrary text, execute instructions contained in
announcement text, satisfy a mutation precondition, or grant capability. Raw report text remains
untrusted observed data.

## Development MCP runtime

The separately named **development MCP runtime** exposes the implemented protocol-1.1 adapter
through the same frozen eleven-tool waist:

```bash
DFMCP_ALLOW_UNADMITTED_LIVE_V1_1=1 \
DFMCP_BRIDGE_TOKEN='<32..256-byte loopback secret>' \
cargo run --locked --bin dfmcp-live-v1-1-dev-server
```

The runtime requires the opt-in value to be exactly `1`. It refuses production admission
environment markers, including `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, uses a protocol-1.1-specific
session namespace, and never consumes a production admission ticket. The public MCP API wrapper
performs the protocol-marker refusal before entering the private server implementation, so external
callers cannot make the development runtime look production-admitted.

Read-only tool behavior is:

```text
fortress.open_session  → authenticate, acquire, and publish one combined anchor
fortress.observe       → heartbeat, advance, or reset the combined anchor
fortress.query         → summary, citizens, announcements, or all
fortress.wait          → report no mutation work and optionally recommend observation
fortress.explain       → explain an entity or combined source/coverage identity
fortress.doctor        → diagnose versions, coverage, source fencing, and development posture
```

`fortress.plan`, `fortress.commit`, `fortress.cancel`, `fortress.checkpoint`, and
`fortress.restore` remain registered for the narrow-waist contract and fail closed. There is **no
mutation** bridge method or alternate effect route.

Every success and failure carries a canonical Agent Turn. The turn explicitly states that the
runtime is unadmitted development, the server artifact is not qualified, no runtime admission has
occurred, announcement history is incomplete, and mutation is unavailable. Successful execution is
useful source and live-campaign tooling, not compatibility evidence by itself.

## Version and admission

This is bridge protocol `1.1` and bridge implementation `0.2.0`. Any historical protocol-1.0
admission remains immutable for its exact source and plugin bytes. It does not admit this source
generation, this plugin, this development binary, or this process.

The production process boundary is `architecture/live_admission_ticket_v2.json`. Its **production
protocol map** currently contains only protocol `1.0`. The deployment manifest protocol is copied
into the launch record, ticket, environment, Rust admission provenance, and final runner selection;
each representation must agree and both canonical digests cover it. Unknown protocols and protocol
`1.1` fail before live-server startup. Legacy V1 tickets are rejected.

Protocol 1.1 requires:

1. fresh source qualification, including the development MCP contract and process tests;
2. a fresh native plugin receipt;
3. a disposable-fort A1-A6 announcement campaign;
4. re-executed baseline fortress/citizen R2-R5 evidence under protocol 1.1;
5. a separately qualified production server artifact;
6. an exact protocol-1.1 compatibility-registry entry;
7. deployment-floor acceptance;
8. an authority-free artifact preflight;
9. a reviewed addition to the V2 production protocol map followed by a fresh protocol-bound,
   single-use ticket and descriptor-only launch.

The development runtime cannot substitute for any of these rungs, and source presence cannot widen
the production protocol map.

## Authority

The announcement stream adds no capability and no mutation method. The native method manifest
remains:

```text
Handshake
ReadObservation
```

`RunCommand`, Lua execution, keyboard forwarding, pause, dig, construction, labor, military,
checkpoint, restore, and arbitrary command forwarding remain absent.
