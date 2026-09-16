# Explicit spatial session closure and same-process recovery

## Added

- `fortress.cancel(scope="session")` in the spatial/1.8 development runtime,
  with ready close requests in live and archive opening responses. Omitted scope
  retains the previous refusal of game-effect cancellation; no twelfth tool exists.
- Serialized release of bridge connections, observation/watch journal locks,
  cached world state, local grants and the two-session capacity permit. Already
  resolved references are fenced and cannot reacquire a context or refresh.
- Explicit `discard_process_local_work=true` consent before volatile watches or
  baselines can be discarded. All consent checks and complete acknowledgement
  rendering precede release; refusal preserves every registry and resource.
- Durable watch journals remain unchanged. Reopening uses existing exact replay,
  new watch handles and reset unfinished stability; close does not cancel saved
  intent, repair evidence, append a checkpoint or claim game-effect completion.
- Resource-only cleanup remains available for a resolved session after source
  failure, read-grant expiry, request-ID exhaustion or an individual session mutex
  panic. This path reads no cached game facts and does not clear registry poison.
- A bounded cache of 32 close acknowledgements, retained in completion order,
  makes retries idempotent without letting stale IDs select newer sessions.

Usage and detailed boundaries are in `docs/SESSION_LIFECYCLE.md`. Closure waits for
an existing foreground call; it does not abort I/O, start background work, or create
an automatic reconnect/session-promotion path. Existing operator configuration
and admission checks still apply at reopening. Other development profiles are not
newly close-capable through this change.

## Implementation and evidence status

Fourteen new Rust regression functions are registered, including real private-file
recovery and actual cancel/query handler paths with injected observations. Cases
cover both session slots, live/offline reopen, durable-watch preservation, voluntary
loss of local work, failed rendering, stale references, concurrent closers, poison,
expired grants, receipt order/eviction, cross-session isolation and 130 reuse cycles.

**No new Rust tests have been compiled or executed in the editing environment.**
Rust, Cargo and rustfmt are unavailable. Main-branch workflow discovery returned
no runs, and no CI, native, live-game, crash or complete-repository qualification
is claimed. Source changes and their final GitHub branch identity were reviewed;
that is not execution evidence.

The formal phase and evidence posture in `IMPLEMENTATION_STATUS.md` do not advance:
this is source-present, unadmitted development functionality. Native protocols,
closed dependencies, the eleven-tool surface, game-effect authority, compatibility
registry and production runner remain unchanged.
