# Live pause control/1.7 development boundary

This profile is the first bridge-backed live mutation slice. It is intentionally tiny: only simulation pause/resume is supported. Digging, construction, labor, burrows, stockpiles, work orders, military changes, checkpoint effects, Lua, arbitrary DFHack commands, keyboard injection, filesystem effects, and network effects remain unavailable.

## Safety model

The bridge exposes exactly four fixed RPC methods: `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause`. The Rust client binds those names, plugin identity, protobuf package, and protocol 1.7 statically. No MCP argument selects a bridge method or native command.

Prepare accepts a stable idempotency key, a 32-byte sealed plan digest, desired pause state, and expected game tick. It performs no mutation and returns a fixed prepare token. Reusing the same idempotency key with different plan content or desired state is rejected.

Commit requires the same key/digest plus that prepare token. The bridge records the effect attempt before calling `World::SetPauseState`, then reads the resulting pause state and retains a stable receipt. Repeating a known commit returns the retained result without applying the effect twice.

A transport failure after commit dispatch is **indeterminate**, not failed. The caller must invoke `fortress.explain` with the same key and digest to query retained bridge state before deciding whether retry is safe. Unknown state is the only condition under which the development server reports retry as safe.

World load/unload changes the bridge generation and clears retained effect records. That deliberately prevents carrying idempotency state across a world identity boundary.

## Development runtime

Run the unadmitted server with an operator-configured token:

```bash
DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7=1 \
DFMCP_CONTROL_TOKEN='<32..256 bytes>' \
DFMCP_CONTROL_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-control-dev-server
```

The runtime rejects unrelated `DFMCP_*` environment state and production admission provenance. It grants only `ControlClock` at reversible risk. The eleven top-level tool names remain present, but only `fortress.plan`, `fortress.commit`, `fortress.explain`, `fortress.doctor`, and session opening are meaningful. Every other operation refuses.

### Prepare

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<64 hex chars>",
  "paused": true,
  "expected_game_tick": 123456
}
```

Use the returned `prepare_token_hex` for commit.

### Commit

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>",
  "prepare_token_hex": "<32 hex chars>"
}
```

If commit reports `effect_indeterminate`, do not retry it directly.

### Reconcile

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>"
}
```

The result reports whether the bridge knows an effect, whether it was applied, the observed pause state and tick, and its retained receipt digest.

## Evidence status

This source is **implemented but unqualified**. It has not been built against the real generated DFHack/protobuf environment or exercised in a disposable fortress in this editing session. No registry entry, monotonic-floor advancement, qualified server artifact, production runner, or admitted mutation capability exists for protocol 1.7.

The source therefore proves only that a deliberately narrow live control boundary exists. It does not justify widening the production map or enabling any additional action family.
