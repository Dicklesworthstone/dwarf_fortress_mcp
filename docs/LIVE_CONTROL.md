# Live pause control/1.7 development boundary

This profile is the first bridge-backed live mutation slice. It is intentionally tiny: only simulation pause/resume is supported. Digging, construction, labor, burrows, stockpiles, work orders, military changes, checkpoint effects, Lua, arbitrary DFHack commands, keyboard injection, filesystem effects, and network effects remain unavailable.

It is an explicitly **unadmitted development** surface. Its existence does not widen the production protocol map, compatibility registry, deployment floor, or any admitted capability.

## Safety model

The bridge exposes exactly four fixed RPC methods: `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause`. The Rust client binds those names, plugin identity, protobuf package, and protocol 1.7 statically. No MCP argument selects a bridge method or native command.

The development runtime requires a private durable coordinator journal. A live pause effect is refused if only process-local coordination is available. An explicit recovery-only session can inspect an existing journal without contacting DFHack or acquiring mutation authority.

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

Applied receipts are full 32-byte SHA-256 values over a fixed receipt domain, bridge generation, key, plan digest, desired pause state, and observed game tick. Terminal applied evidence is rejected if it reports the opposite pause state or lacks a 32-byte receipt. The journal enforces matching pause-state evidence during replay as well as live reconciliation.

The Rust coordinator syncs `VerifiedApplied` or `VerifiedNotApplied` before acknowledging a terminal outcome. If the bridge reply is received but that terminal journal write cannot be durably established, the MCP result is `effect_indeterminate`, not success.

### Reconciliation and restart

In a live session, `fortress.explain` is the reconciliation operation. It never dispatches an effect. A poisoned connection may be reopened for this read-only bridge query, but a mutating commit is never reissued as part of recovery. A live reconciliation may append coordinator evidence; it is distinct from a read-only recovery session.

After Rust-process restart, the journal reconstructs the exact transition chain. A retained `CommitStarted` or `Indeterminate` state remains unresolved until reconciliation. `fortress.query` discovers the retained keys, plan digests and states without needing the previous conversation or action handles.

If the same bridge incarnation still retains the effect, live reconciliation records its known applied/not-applied result durably. If the bridge generation changed, the DFHack process restarted, a world load/unload cleared the bridge record, or the bridge otherwise reports the key unknown, the durable record remains **indeterminate**. The same effect is never reported safe to retry. A fresh observation and a new plan/idempotency key are required.

A `Prepared` record that never reached `CommitStarted` may still be committed once only in a live session when the current bridge generation exactly matches the generation sealed into the durable prepare. Otherwise it must be abandoned in favor of a new plan/key.

This policy deliberately prefers an unresolved durable record over the possibility of replaying a mutation whose prior outcome cannot be proved.

## Durable journal custody

`DFMCP_CONTROL_JOURNAL` is mandatory and is operator configuration, never an MCP argument. On Unix the path must be absolute and normalized, inside an existing canonical exact-mode `0700` directory. The journal file is an exact-mode `0600`, single-link regular file owned by the same account as the directory and held with an exclusive lock.

The journal is hash chained and bounded to 64 MiB, 16,384 transitions, and 4,096 distinct effects. Append and replay both enforce the distinct-effect bound. Every record binds a global transition number, previous record digest, per-effect revision, immutable effect identity, state, and terminal evidence.

A write or sync failure fences the journal. Default startup refuses an incomplete tail without changing the file. In live mode, `DFMCP_CONTROL_JOURNAL_REPAIR=1` permits truncating only an incomplete trailing frame after a verified prefix. Complete corrupt frames and broken predecessor chains are never silently repaired. Recovery-only mode refuses the repair opt-in entirely.

Recovery uses a read-only file descriptor, retains the same exclusive lock and custody checks, and never creates, initializes, truncates, syncs or appends a journal. All mutation entry points reject recovery mode even if a caller supplies a `ControlClock` context. Public MCP operations recheck authority and journal custody before exposing even an idempotent or terminal result.

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

Opening a default live session creates a new journal file as `0600` when needed. Existing files must already satisfy custody rules. `DFMCP_CONTROL_JOURNAL_REPAIR=1` is an explicit operator-only incomplete-tail repair opt-in for live mode.

The runtime rejects unrelated `DFMCP_*` environment state and production admission provenance. Live sessions grant only `ControlClock` at reversible risk; recovery-only sessions grant only `Query` at read-only risk. The control session namespace is distinct from the read-profile namespaces and the runtime retains at most one control or recovery session. The eleven top-level tool names remain present. Session opening, `fortress.query`, `fortress.explain`, and `fortress.doctor` work in both modes; `fortress.plan` and `fortress.commit` require live mode. Other operations refuse.

Agent Turn metadata says `runtime_admitted=false` and `mutation_admissible=false`. Live mode separately reports `development_mutation_enabled=true`; recovery-only mode reports false and advertises no supported effects. Both explicitly distinguish coordinator evidence from current game facts.

### Offline recovery without DFHack

Stop the previous server so it releases its exclusive journal lock. Keep the original journal at its operator-configured path. Do not create an empty replacement. Neither bridge credentials nor an available DFHack process are needed:

```bash
unset DFMCP_CONTROL_JOURNAL_REPAIR

DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7=1 \
DFMCP_CONTROL_JOURNAL=/absolute/private/dfmcp-control/effects.bin \
cargo run --locked --bin dfmcp-live-control-dev-server
```

Call `fortress.open_session` with:

```json
{"recovery_only": true}
```

The recovery branch does not read bridge credentials or parse a bridge endpoint and constructs no connection. Missing, empty, corrupt, incomplete or concurrently locked journals are refused. An existing valid empty journal header is readable and produces an empty effect listing; a zero-byte file is not a valid journal.

Use `fortress.query` with the returned session ID:

```json
{
  "session_id": "<session>",
  "state": "reconciliation_required",
  "limit": 8
}
```

The default state filter is `all`. Other filters are `nonterminal`, `reconciliation_required`, `prepared`, `commit_started`, `indeterminate`, `verified_applied`, and `verified_not_applied`. `nonterminal` includes prepared effects; `reconciliation_required` includes only started or indeterminate attempts.

Results are sorted by idempotency key and contain complete durable records, total/matching counts, counts for each state, and the journal ID/head. The result reports `current_freshness_proven=false`, `reconciliation_performed=false`, and `commit_permitted=false`. A historical applied receipt does not establish that the game is still paused now.

A page may contain fewer records than `limit` to fit the negotiated response budget. When `truncated=true`, pass the returned `continuation` with the same session and state filter. The fixed-size continuation binds the session, journal incarnation, exact head, filter and offset. A changed head, different session/filter, malformed token or invalid offset requires restarting the listing. Continuations are consistency checks, not credentials or mutation authority.

`limit` must be 1..128 and is narrowed by the session entity budget. Optional `max_bytes` and `max_output_tokens` can only narrow the session response limits. Successful pages include the entire Agent Turn in byte accounting; tokens are explicitly estimated as `ceil(UTF-8 bytes / 4)`, not counted with a model tokenizer. A budget too small for one complete matching record and required metadata returns `BudgetExceeded`, never a zero-progress continuation. No fields or authority metadata are silently removed to fit a page.

Recovery-mode `fortress.explain` returns the selected stored record without querying the bridge or changing the journal. `fortress.doctor` reports no bridge connection and an unknown/null current bridge generation. To perform live reconciliation, stop this server and open a new live session with bridge credentials. A recovery session cannot upgrade itself or dispatch an effect.

### Prepare example (live mode)

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

### Commit example (live mode)

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>",
  "prepare_token_hex": "<32 lowercase hex chars>"
}
```

If commit reports `effect_indeterminate`, do not call commit again for that durable effect.

### Explain/reconcile example

```json
{
  "session_id": "<session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same digest>"
}
```

The response reports the durable state, recorded bridge generation, observed pause state/tick when established, receipt digest and journal head. In live mode it may reconcile and report whether a new plan is required. In recovery-only mode it reports only stored evidence and never permits commit. `safe_to_retry_same_effect` remains false once a commit attempt has started.

## Evidence status

The existing native-source evidence reports compile checks against explicit mock DFHack/protobuf interfaces with GCC and Clang under C++17, `-Wall -Wextra -Werror -pedantic`. The reproducible repository harness is `scripts/test_live_control_native_mock.py`. Those checks are not a real DFHack build or native qualification and were not rerun for the recovery-only increment.

The recovery increment adds and registers 14 Rust regression tests: seven adapter journal tests, four bounded-query tests, and three Unix MCP-handler tests. They cover read-only replay and no-write behavior, write refusal despite supplied clock authority, custody/locking, missing and incomplete journals, effect-count and applied-evidence replay validation, canonical paging and state filters, session/filter/head/tampering rejection, full-packet budgets, and actual offline query/explain/doctor/plan/commit behavior.

The Rust tests have **not been executed in the editing environment**, which has no Rust toolchain. Focused validation commands on a configured checkout are:

```bash
cargo test --locked -p dfmcp-adapter control_effect_journal
cargo test --locked -p dfmcp-mcp live_control_server
```

These focused commands are not a substitute for the repository's full verification and qualification requirements. No Rust compilation/test pass, filesystem power-loss campaign, disposable-fort control campaign, registry entry, monotonic-floor advancement, qualified server artifact, production runner, or admitted mutation capability is claimed by this increment. Protocol 1.7 remains unadmitted.

The next evidence-bearing step is qualification of this exact narrow boundary, not adding broader mutation families.
