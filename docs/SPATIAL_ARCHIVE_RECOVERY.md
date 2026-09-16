# Offline spatial archive recovery

The spatial/1.8 development server can now open an existing observation journal without a running
Dwarf Fortress or DFHack process. The archive is replayed into the same typed citizen, operations
and terrain state used by the live query engines, but it is never presented as current game state.

This is implemented, unadmitted development source. Rust compilation and execution have not been
established for this increment. No native protocol, dependency, production runner or admission
registry changes are involved.

## Start with the existing archive

Stop the previous owner so it releases the observation journal's exclusive lock. Keep the original
journal; do not create an empty replacement. The path remains operator configuration, not an MCP
argument. On Unix it must be an exact-mode 0600 single-link regular file beneath a canonical
exact-mode 0700 directory with the existing ownership checks.

```bash
unset DFMCP_SPATIAL_CITIZEN_JOURNAL_REPAIR
unset DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL

DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_8=1 \
DFMCP_SPATIAL_CITIZEN_JOURNAL=/absolute/private/observations.bin \
cargo run --locked --bin dfmcp-live-spatial-citizens-dev-server
```

Bridge credentials and an endpoint are not needed. The recovery branch runs before either is read
or a connection is constructed. Existing development opt-in and production-admission refusal
remain enforced.

Call `fortress.open_session` with `recovery_only: true` and the region used by this archive:

```json
{
  "recovery_only": true,
  "region": {"origin": [0, 0, 5], "size": [3, 3, 1]},
  "requested_capabilities": ["query", "doctor"],
  "max_output_tokens": 8192,
  "max_wall_millis": 5000
}
```

The region above is illustrative. The selected region and acquisition bounds must agree with the
retained capture. Replay enforces the existing profile, record, byte, entity and cooperative
wall-time bounds. Each selected older record is also checked against the session's acquisition
limits. A valid header with no observations cannot bootstrap a world.

Query is required; Doctor is optional. Requesting Observe or a mutation capability fails. Missing,
zero-byte, incomplete, corrupt, wrong-profile and concurrently owned journals are refused without
initialization or repair. Recovery mode rejects the repair opt-in and watch-journal configuration
rather than silently ignoring either one. The underlying descriptor is read-only, and its write,
flush, sync and truncate paths explicitly refuse use.

## Inspect retained facts

The usual `summary`, `citizens`, `jobs`, `buildings`, `items`, and `tiles` query modes inspect the
latest retained observation. The `schema` mode exposes the archive-only query contract: ten stateless
analysis variants plus `history` and `historical_query`. Watch and baseline operations are not
advertised or routed.

Structured archived queries support:

- entity selection/inspection, graph traversal/dependencies, aggregates and search;
- bounded terrain routes and route-aware inventory allocation;
- workforce candidates and simultaneous capacity planning.

These are the existing query/analysis engines, not a second implementation of their algorithms.
Workforce plans remain model-only: no labor assignments, reservations or native eligibility proof
are created. Hidden terrain and all other limitations of the original capture remain in force.

Every successful response includes `archive_only=true`, `historical=true`, `live=false`,
`current_freshness_proven=false`, `bridge_connection_present=false`, `native_captures=0`, and the
selected journal record with its exact observation and record digests. Agent Turn continuity is
partial with an explicit archive-not-current-state reason. Empty results refer only to the named
archived projection, never to the current fortress or events between captures.

## Select an exact earlier observation

List retained captures through the existing query tool:

```json
{
  "session_id": "<archive session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {"kind": "history", "limit": 8}
  }
}
```

Rows include record number, full anchor, source digest, record digest and predecessor digest. History
pages permit 1..64 rows. The `ar1` continuation binds the session, archive incarnation, exact head,
latest session anchor and offset. Page width may change; a token from another session is refused.
No empty-progress continuation is emitted when a complete row cannot fit.

Use the exact `record` and `record_digest` from a row:

```json
{
  "session_id": "<archive session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "historical_query",
      "record": 1,
      "record_digest": "<64 lowercase hex characters from history>",
      "query": {
        "kind": "workforce_plan",
        "demands": [
          {"key": "wood", "workers": 2, "target": [0, 0, 5], "skill_key": "CARPENTRY"}
        ],
        "limit": 8
      }
    }
  }
}
```

The inner query is stateless and executes on the selected reconstructed state. It does not replace
the archive session's latest observation. The request's optional outer `expected_anchor` names the
session's latest retained anchor; the inner record selector names the historical target. Current
session authority is checked before reconstructing an older state, so older facts do not revive
expired or exhausted grants.

Returned inventory/workforce `route_query` drill-downs are rewritten as exact-record historical
queries. Following one cannot silently switch to the newest archived terrain. The runtime checks
that the replacement wrapper fits the already reserved row space. Ordinary query continuations
retain the underlying engine's snapshot/query binding.

## Boundaries that remain closed

Archive-only sessions cannot refresh, await or evaluate watches, register monitoring, create or
release baselines, prepare/commit effects, checkpoint or restore game state. They cannot upgrade
into live sessions. Even an internally injected Observe grant does not enable the archive source's
read path. Return to live operation by stopping this server and opening a new live session with the
normal credentials and configuration.

The watch journal is deliberately not loaded. An empty active-work list is scoped to this new
archive session; `watch_evidence_loaded=false` explicitly prevents interpreting it as evidence that
persisted watches or game actions do not exist. Existing durable-watch recovery remains a live
bootstrap operation. This increment does not repair, compact, prune, migrate or rotate any journal.

Authority and file custody are rechecked before cached queries and diagnostics. Exact historical
reads additionally reverify their record bytes and replay the generation chain. The owning account,
root and canonical ancestors remain trusted; this is not hostile-host or anti-rollback protection.
Filesystem operations and replay have cooperative checks, not a hard preemption guarantee.

Full Agent Turn and selected-record metadata are reserved before filling query pages. The final
serialized packet is checked again. Output-token budgeting retains the existing byte/4 estimate,
not a claim of model-tokenizer accounting. A tiny budget yields an explicit failure without
modifying either journal or the session's archived world.

## Evidence for this increment

Fourteen Rust scenarios are registered: six adapter integration tests using real private files and
eight archive bootstrap/MCP-handler tests. They cover read-only replay, missing/empty/incomplete
files, wrong profiles and authority, injected Observe, locking and links, custody changes, query
modes, historical workforce analysis, pinned routes, bounded pages, session-bound history tokens,
stateful-tool refusal, and archive-specific schema discovery.

They have **not been compiled or executed in the editing environment**. No Rust compiler, Cargo or
rustfmt is available. Focused commands on a configured checkout are:

```bash
cargo test --locked -p dfmcp-adapter --test observation_archive_recovery_tests -- --test-threads=1
cargo test --locked -p dfmcp-mcp spatial_archive -- --test-threads=1
```

An independent Python JSON-size calculation checked 128 boundary combinations of anchor widths,
record numbers and route coordinates. The historical wrapper was always smaller than the original
anchor-bound route request; its largest growth was -22 bytes. This is only a wrapper-size reference
check. It does not execute the Rust serializer, state replay, filesystem backend, query engines,
MCP server or DFHack, and is not qualification or admission evidence.
