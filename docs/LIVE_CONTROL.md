# Live pause control/1.7 development boundary

This profile is the first bridge-backed live mutation slice. It is intentionally tiny: only simulation pause/resume is supported. Digging, construction, labor, burrows, stockpiles, work orders, military changes, checkpoint effects, Lua, arbitrary DFHack commands, keyboard injection, filesystem effects, and network effects remain unavailable.

It is an explicitly **unadmitted development** surface. Its existence does not widen the production protocol map, compatibility registry, deployment floor, or any admitted capability.

## Safety model

The bridge exposes exactly four fixed RPC methods: `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause`. The Rust client binds those names, plugin identity, protobuf package, and protocol 1.7 statically. No MCP argument selects a bridge method or native command.

The development runtime now requires a private durable coordinator journal. A live pause effect is refused if only process-local coordination is available.

The core ordering is:

```text
bridge prepare (no mutation)
→ fsync Prepared
→ fsync CommitStarted
→ exactly one CommitPause dispatch
→ observe bridge result
→ fsync VerifiedApplied / VerifiedNotApplied
→ acknowledge terminal result
```

If any step after `CommitStarted` is ambiguous, the effect remains reconciliation-required. Restarting the Rust process never turns that state back into a retryable prepare.

### Prepare

Prepare accepts a stable idempotency key, a 32-byte sealed plan digest, desired pause state, and expected game tick. It performs no mutation.

The bridge prepare token is the first 16 bytes of SHA-256 over a fixed domain, the current bridge generation, key, plan digest, expected tick, and desired pause state. A world-generation change therefore invalidates the old token even if the textual key is reused.

After bridge prepare succeeds, the Rust coordinator syncs a `Prepared` record before returning success. Reusing the key with different plan content, desired state, tick, bridge generation, or token is rejected.

If the Rust process dies after bridge prepare but before the durable `Prepared` record is synced, no game mutation has occurred. On a later attempt the bridge may know a key that the journal does not; the runtime refuses to adopt it implicitly and requires a new idempotency key.

### Commit

Commit requires the exact key, plan digest, and prepare token already present in the durable journal.

Before calling the bridge, the coordinator appends and syncs `CommitStarted`. If this durability boundary fails, **no bridge mutation is attempted**.

After that boundary, the runtime performs exactly one `CommitPause` RPC. It never retries a commit automatically. The native bridge marks its own process-local effect record as requested before calling `World::SetPauseState`, observes the pause state afterward, and returns a retained result on duplicate commits while that bridge incarnation remains alive.

Applied receipts are full 32-byte SHA-256 values over a fixed receipt domain, bridge generation, key, plan digest, desired pause state, and observed game tick. Terminal applied evidence is rejected if it reports the opposite pause state or lacks a 32-byte receipt.

The Rust coordinator syncs `VerifiedApplied` or `VerifiedNotApplied` before acknowledging a terminal outcome. If the bridge reply is received but that terminal journal write cannot be durably established, the MCP result is `effect_indeterminate`, not success.

### Reconciliation and restart

`fortress.explain` is the reconciliation operation. It never dispatches an effect. A poisoned connection may be reopened for this read-only query, but a mutating commit is never reissued as part of recovery.

After Rust-process restart, the journal reconstructs the exact transition chain. A retained `CommitStarted` or `Indeterminate` state remains unresolved until reconciliation.

If the same bridge incarnation still retains the effect, reconciliation records its known applied/not-applied result durably. If the bridge generation changed, the DFHack process restarted, a world load/unload cleared the bridge record, or the bridge otherwise reports the key unknown, the durable record remains **indeterminate**. The same effect is never reported safe to retry. A fresh observation and a new plan/idempotency key are required.

A `Prepared` record that never reached `CommitStarted` may still be committed once only when the current bridge generation exactly matches the generation sealed into the durable prepare. Otherwise it must be abandoned in favor of a new plan/key.

This policy deliberately prefers an unresolved durable record over the possibility of replaying a mutation whose prior outcome cannot be proved.

## Durable journal custody

`DFMCP_CONTROL_JOURNAL` is mandatory and is operator configuration, never an MCP argument. On Unix the path must be absolute and normalized, inside an existing canonical exact-mode `0700` directory. The journal file is an exact-mode `0600`, single-link regular file owned by the same account as the directory and held with an exclusive lock.

The journal is hash chained and bounded to 64 MiB, 16,384 transitions, and 4,096 distinct effects. Every record binds a global transition number, previous record digest, per-effect revision, immutable effect identity, state, and terminal evidence.

A write or sync failure fences the journal. Default startup refuses an incomplete tail without changing the file. `DFMCP_CONTROL_JOURNAL_REPAIR=1` permits truncating only an incomplete trailing frame after a verified prefix. Complete corrupt frames and broken predecessor chains are never silently repaired.

This journal is not an anti-rollback floor, signed provenance, a game checkpoint, or production admission. It makes coordinator restart behavior explicit; it does not prove what happened if both durable coordinator evidence and the bridge's retained effect evidence are lost or maliciously modified.

## Development runtime

Create a private directory and run the unadmitted server with operator-configured credentials and journal:

```bash
mkdir -m 700 /absolute/private/dfmcp-control

DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7=1 \
DFMCP_CONTROL_TOKEN='<32..256 bytes>' \
DFMCP_CONTROL_ENDPOINT=127.0.0.1:5000 \
DFMCP_CONTROL_JOURNAL=/absolute/private/dfmcp-control/effects.bin \
cargo run --locked --bin dfmcp-live-control-dev-server
```

The server creates a new journal file as `0600`. Existing files must already satisfy custody rules. `DFMCP_CONTROL_JOURNAL_REPAIR=1` is an explicit operator-only incomplete-tail repair opt-in.

The runtime rejects unrelated `DFMCP_*` environment state and production admission provenance. It grants only `ControlClock` at reversible risk. The control session namespace is distinct from the read-profile namespaces and the runtime retains at most one mutation session. The eleven top-level tool names remain present, but only session opening, `fortress.plan`, `fortress.commit`, `fortress.explain`, and `fortress.doctor` are meaningful. Every other operation refuses.

Agent Turn metadata says `runtime_admitted=false` and `mutation_admissible=false`; it separately reports that the explicitly unadmitted development mutation surface is enabled. That distinction is intentional.

### Prepare example

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<64 lowercase hex chars>",
  "paused": true,
  "expected_game_tick": 123456
}
```

Use the returned durable `prepare_token_hex` for commit.

### Commit example

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>",
  "prepare_token_hex": "<32 lowercase hex chars>"
}
```

If commit reports `effect_indeterminate`, do not call commit again for that durable effect.

### Reconcile example

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>"
}
```

The response reports the durable state, bridge generation, observed pause state/tick when established, receipt digest, journal head, and whether a new plan is required. `safe_to_retry_same_effect` remains false once a commit attempt has started.

## Evidence status

The actual native source has been compile-checked in the editing environment against explicit mock DFHack/protobuf interfaces with both GCC and Clang under C++17, `-Wall -Wextra -Werror -pedantic`. A local state-machine mirror passed prepare replay, one-shot commit, duplicate-commit suppression, query reconciliation, generation reset, and fixed-method checks. Independent Python `hashlib` calculations matched the generation-bound token and 32-byte receipt SHA-256 identities after correcting an embedded-NUL domain-separation bug. The reproducible repository harness is `scripts/test_live_control_native_mock.py`.

Those checks are **not** a real DFHack build or native qualification. The Rust journal, client, and MCP runtime have not been compiled or executed here because no Rust toolchain was available. No filesystem power-loss campaign, disposable-fort control campaign, registry entry, monotonic-floor advancement, qualified server artifact, production runner, or admitted mutation capability exists for protocol 1.7.

The next evidence-bearing step is therefore qualification of this exact narrow boundary, not adding broader mutation families.
