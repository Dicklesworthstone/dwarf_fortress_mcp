# Explicit spatial source recovery

The unadmitted spatial/1.8 server can reconnect a fenced read source **without
closing the session or discarding its process-local watches and baselines**.
This is an explicit foreground operation, not an automatic retry loop. It uses
the existing authenticated connection and complete-observation publication path.
No native method, wire generation, dependency, top-level MCP tool, game effect,
production runner or compatibility admission is added.

## Request

Use the complete retained session anchor from its last successful observation or
query. A caller cannot choose another endpoint, credential, protocol or journal.
The following is illustrative; substitute the exact real anchor, not a nearby tick.

```json
{
  "session_id": "<current spatial/1.8 session>",
  "query": {
    "schema": "dfmcp.query/1",
    "expected_anchor": {
      "fortress_id": "<retained fortress id>",
      "epoch": 0,
      "sequence": 12,
      "game_tick": 42336012,
      "state_hash": "<retained 64-character state hash>"
    },
    "query": {"kind": "recover_source", "max_wall_millis": 5000}
  }
}
```

The source must already be fenced. Healthy sources use `fortress.observe` or
normal watch awaiting instead. Current Query **and** Observe grants are required,
including at the newly captured tick even in a session without an observation
journal. `max_wall_millis` is optional and may only narrow the current allowance;
it must be 1..60000. A stale expected anchor refuses before any watch transition.
Schema discovery includes the request only for live spatial sessions. Archive-only
and closed sessions cannot acquire live authority through recovery.

The operator's existing `DFMCP_SPATIAL_CITIZEN_ENDPOINT` and
`DFMCP_SPATIAL_CITIZEN_TOKEN` are used. The endpoint still must be loopback and
credentials retain their existing 32..256-byte bound. Source identity, software,
profile, region and acquisition bounds remain checked by the existing spatial
normalizer and publication path. A different fortress is not silently adopted.

## Ordered progress boundaries

The enclosing session mutex remains held throughout the operation:

```text
validate current grants, exact anchor, bounds and journal custody
-> stage interruption of unfinished monitoring
-> reserve complete success/failure responses
-> sync the optional watch checkpoint and publish the interrupted watch root
-> connect/handshake once
-> acquire one complete coherent native capture
-> revalidate both read grants and capture identity/bounds
-> sync the optional observation journal, then publish the new observation
-> return the complete recovery result with current active-work metadata
```

Connection, handshake, capture and prepublication validation share the foreground
wall-time allowance. There is no retry, sleep loop, detached task, mutation,
checkpoint repair, history pruning or archive conversion. Synchronous filesystem
operations retain the existing cooperative, not hard-cancellable, deadline model.

A network failure **does not undo the monitoring interruption**. `ok=false` reports
the bounded error, connection/capture attempt counts, retained anchor and
`source_gap`. The source stays fenced. Repeating recovery with the same retained
anchor does not reseal unchanged interruption evidence or consume another watch
checkpoint merely because the network is still unavailable.

A response too small for the prospective full success/failure packet rejects
before recording a gap or connecting. The reservation uses the same final
renderer, worst-case integer widths and escaped bounded diagnostics. The final
packet is also checked. Failure of the optional watch checkpoint prevents
connection. Failure of capture validation or observation persistence does not
publish a candidate world or burn its generation history. An uncertain durable
write is not repaired automatically. Already synced progress is not hidden behind
a fallible post-sync deadline check.

The two journals are ordered publications, not an atomic two-file rollback. A
successful observation may remain published when a later external custody fault
prevents the final response. Neither a request error nor a missing acknowledgement
proves that nothing reached disk. Existing close/reopen recovery remains available.

## Watch semantics

Session identity, watch handles, definitions, original deadlines, prior sample
counts and last actual sample ticks survive. Unfinished success streaks reset to
zero and become `blocked_unknown`; expired or incompatible work is not revived.
Previously terminal evidence is untouched. A source gap is not a sample, predicate
evaluation, cancellation, game-effect failure or successful goal completion.

The response includes `source_gap.records`, preserved handle/evidence links,
`changed_watches`, `pending_watches`, `terminal_watches`, and explicit continuity
limitations. Active-work metadata does not claim that predicates were evaluated
during recovery. `source_recovery.capture_outcome` distinguishes heartbeat,
advancement and epoch reset. Even successful recovery reports partial continuity,
not an uninterrupted event history. Failure reports stale continuity.

Recovery does not evaluate watches against the reacquired capture. An explicit
subsequent `poll_watch`, `poll_watches`, `await_watch` or `await_watches` performs
normal evaluation. A changed capture may supply the first new eligible sample;
a byte-identical capture at the same anchor cannot. Existing cadence and original
deadline still apply. A bridge/world epoch reset invalidates old unfinished watches
when they are evaluated; it does not rebind them to recycled entities.

Process-local baselines remain retained endpoint comparisons, not continuous
outage history. Their existing epoch, anchor, retention and continuation rules
continue to apply. No old page continuation is promoted to a new anchor.

When paired watch persistence is configured, the gap record is synced before
network work. A process restart can replay it from the same observation archive;
ordinary restart semantics assign new handles and reset unfinished stability
again. In-place reconnect itself preserves the existing handles.

## Implementation status and validation

Source and **18 Rust regression scenarios** are registered: six interruption
state/serialization tests and twelve spatial recovery tests. They exercise the
real watch publication and spatial normalization helpers with injected connection
and capture boundaries, including private paired journals. Cases cover stable
progress reset, idempotent failures, exact heartbeats, terminal preservation,
expired/cancelled grants, output/deadline refusal, corruption, epoch changes,
UTF-8 diagnostics and replay after a failed reconnect. These are not live-game or
stdio tests.

The editing environment has no Rust, Cargo or rustfmt. **The Rust scenarios have
not been compiled or executed.** No Rust/Clippy/runtime, filesystem power-loss,
real DFHack/protobuf, live-campaign or whole-repository qualification is claimed.
Existing qualification gates and the production protocol map are unchanged.

Executed `python scripts/test_source_recovery_schema.py`: **51 envelope cases**
(6 accepted, 45 rejected), plus **6 independent conditional-composition cases**.
Schema SHA-256:
`3d30a6698ffacc9a39ed2d75fa278ba0292e7881889168dacffed0b30bdb386d`.
This checks the isolated request envelope and an independent reference for its
conditional anchor requirement. It does not execute the Rust schema composer,
canonical-anchor comparison, grants, journal custody, reconnect or monitoring.

Focused commands on a configured checkout:

```bash
python scripts/test_source_recovery_schema.py
cargo test --locked -p dfmcp-mcp watch_source_gap -- --test-threads=1
cargo test --locked -p dfmcp-mcp spatial_source_recovery -- --test-threads=1
```

These do not replace the repository's complete verification and native/live
qualification requirements. This increment supplies unadmitted development source.
