# Live pause control/1.7 development boundary

This profile is the first bridge-backed live mutation slice. It is intentionally tiny: only simulation pause/resume is supported. Digging, construction, labor, burrows, stockpiles, work orders, military changes, checkpoint effects, Lua, arbitrary DFHack commands, keyboard injection, filesystem effects, and network effects remain unavailable.

It is an explicitly **unadmitted development** surface. Its existence does not widen the production protocol map, compatibility registry, deployment floor, or any admitted capability.

## Safety model

The bridge exposes exactly four fixed RPC methods: `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause`. The Rust client binds those names, plugin identity, protobuf package, and protocol 1.7 statically. No MCP argument selects a bridge method or native command.

The development runtime requires a private durable coordinator journal. A live pause effect is refused if only process-local coordination is available. An explicit recovery-only session can inspect an existing journal without contacting DFHack or acquiring mutation authority.

The core ordering is:

```text
bridge prepare (no mutation)
→ verify prepare identity
→ fsync Prepared
→ fsync CommitStarted
→ exactly one CommitPause dispatch
→ observe bridge result and verify terminal receipt identity
→ fsync VerifiedApplied / VerifiedNotApplied
→ acknowledge terminal result
```

If any step after `CommitStarted` is ambiguous, the effect remains reconciliation-required. Restarting the Rust process never turns that state back into a retryable prepare.

### Prepare

Prepare accepts a stable idempotency key, a 32-byte sealed plan digest, desired pause state, and expected game tick. It performs no mutation.

The bridge prepare token is the first 16 bytes of SHA-256 over a fixed domain, the current bridge generation, key, plan digest, expected tick, and desired pause state. A world-generation change therefore invalidates the old token even if the textual key is reused. Rust independently recomputes this identity before accepting a new preparation.

After bridge prepare succeeds, the Rust coordinator syncs a `Prepared` record before returning success. Reusing the key with different plan content, desired state, tick, bridge generation, or token is rejected.

The native record keeps the desired pause state and expected prepare tick separate from observed pause state and observed tick. Replaying the same preparation after time advances or after a commit returns the original token and retained evidence. It does not reseal the effect at a newer tick. A new preparation still requires the exact current tick.

If the Rust process dies after bridge prepare but before the durable `Prepared` record is synced, no game mutation has occurred. On a later attempt the bridge may know a key that the journal does not; the runtime refuses to adopt it implicitly and requires a new idempotency key.

### Commit

Commit requires the exact key, plan digest, and prepare token already present in the durable journal.

Before calling the bridge, the coordinator appends and syncs `CommitStarted`. If this durability boundary fails, **no bridge mutation is attempted**.

After that boundary, the runtime performs exactly one `CommitPause` RPC. It never retries a commit automatically. The native bridge checks world/fortress availability and a valid, nonregressed clock before the setter. It marks its own process-local effect record as requested before calling `World::SetPauseState`, then reads the actual pause state and game tick. Duplicate commits return the retained result without invoking the setter again while that bridge incarnation remains alive.

Terminal receipts are full 32-byte SHA-256 values over the existing fixed receipt domain, bridge generation, key, plan digest, desired pause state, and observed game tick. The live coordinator independently recomputes the receipt, requires the observed tick not to precede preparation, and requires the applied flag to agree with whether the observed pause state equals the desired state. A returned prepare token, when present, must match the durable token.

Both terminal applied and terminal not-applied results require a complete matching receipt. A setter that returns while the game remains in the opposite pause state produces observed not-applied evidence. A clock failure or exception after the setter leaves no complete terminal receipt and must remain indeterminate. The native record remains requested, so a duplicate cannot invoke the setter again.

**A known key without a receipt is not proof that an effect failed.** It may be only a native prepare, or a commit whose observation did not finish. Live commit/reconciliation therefore never promotes that reply to `VerifiedNotApplied`; an unresolved durable attempt stays indeterminate. Merely recording `CommitStarted` also does not prove the bridge ever received the request.

The wire decoder requires explicit applied, paused and observed-tick fields for a known record. Missing fields are not converted into observations of false or zero. Present tokens and receipts must have their exact lengths. Any failed wire call fences the connection, including budget failures that leave unread frame bytes; a later request cannot consume those bytes as its reply. Connection establishment and handshake share one timeout allowance.

The Rust coordinator syncs `VerifiedApplied` or `VerifiedNotApplied` before acknowledging a terminal outcome. If the bridge reply is received but that terminal journal write cannot be durably established, the MCP result is `effect_indeterminate`, not success.

### Reconciliation and restart

In a live session, `fortress.explain` reconciles one effect and `fortress.wait` performs a bounded pass over selected effects. Neither dispatches a game mutation. A poisoned connection may be reopened for read-only bridge queries, but a mutating commit is never reissued as recovery. Live reconciliation may append coordinator evidence; it is distinct from an offline read-only recovery session.

After Rust-process restart, the journal reconstructs the exact transition chain. A retained `CommitStarted` or `Indeterminate` state remains unresolved until reconciliation. `fortress.query` discovers the retained keys, plan digests and states without needing the previous conversation or action handles.

If the same bridge incarnation retains a complete matching terminal receipt, live reconciliation records the applied/not-applied result durably. If the generation changed, the key is unknown, or no terminal receipt exists, the durable record remains **indeterminate**. Invalid or contradictory evidence is rejected without resolving the existing attempt. The same effect is never reported safe to retry. A fresh observation and a new plan/idempotency key are required for a new effect.

A `Prepared` record that never reached `CommitStarted` may still be committed once only in a live session when the current bridge generation exactly matches the generation sealed into the durable prepare. Otherwise it must be abandoned in favor of a new plan/key. Reconciliation does not commit a prepared record.

This policy deliberately prefers an unresolved durable record over the possibility of replaying a mutation whose prior outcome cannot be proved.

## Durable journal custody

`DFMCP_CONTROL_JOURNAL` is mandatory and is operator configuration, never an MCP argument. On Unix the path must be absolute and normalized, inside an existing canonical exact-mode `0700` directory. The journal file is an exact-mode `0600`, single-link regular file owned by the same account as the directory and held with an exclusive lock.

The journal is hash chained and bounded to 64 MiB, 16,384 transitions, and 4,096 distinct effects. Append and replay both enforce the distinct-effect bound. Every record binds a global transition number, previous record digest, per-effect revision, immutable effect identity, state, and terminal evidence.

A write or sync failure fences the journal. Default startup refuses an incomplete tail without changing the file. In live mode, `DFMCP_CONTROL_JOURNAL_REPAIR=1` permits truncating only an incomplete trailing frame after a verified prefix. Complete corrupt frames and broken predecessor chains are never silently repaired. Recovery-only mode refuses the repair opt-in entirely.

Recovery uses a read-only file descriptor, retains the same exclusive lock and custody checks, and never creates, initializes, truncates, syncs or appends a journal. All mutation entry points reject recovery mode even if a caller supplies a `ControlClock` context. Public MCP operations recheck authority and journal custody before exposing even an idempotent or terminal result.

The receipt-verification increment governs newly received live evidence. It does not migrate or cryptographically requalify terminal records already persisted by earlier code. Existing journal replay still checks framing, chain, transition identity and applied pause-state consistency. A retained terminal record is stored historical evidence, not a new claim of current game truth.

This journal is not an anti-rollback floor, signed provenance, a game checkpoint, or production admission. Receipt hashes bind identity; they are not signatures and do not make a compromised bridge trustworthy. The journal does not prove what happened if durable coordinator evidence and the bridge's retained effect evidence are lost or maliciously modified.

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

The runtime rejects unrelated `DFMCP_*` environment state and production admission provenance. Live sessions grant only `ControlClock` at reversible risk; recovery-only sessions grant only `Query` at read-only risk. The control session namespace is distinct from the read-profile namespaces and the runtime retains at most one control or recovery session. The eleven top-level tool names remain present. Session opening, `fortress.query`, `fortress.explain`, and `fortress.doctor` work in both modes; `fortress.plan`, `fortress.commit`, and `fortress.wait` require live mode. Other operations refuse.

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

### Bounded reconciliation with fortress.wait (live mode)

Select real keys returned by `fortress.query`:

```json
{
  "session_id": "<live session>",
  "idempotency_keys": ["pause-maintenance-001", "pause-maintenance-002"],
  "max_wall_millis": 2000
}
```

The pass accepts 1..16 unique known keys within the session entity budget. It validates the complete selection, sorts by key, and reserves the worst-case complete response including the Agent Turn before any bridge query or journal transition. An unknown/duplicate key or inadequate response budget rejects the whole selection before work. Select fewer keys when a large selection cannot fit. Optional `max_bytes`, `max_output_tokens`, and `max_wall_millis` only narrow the existing session allowances.

Only `CommitStarted` and `Indeterminate` records are queried. Prepared records are returned unchanged, never committed; terminal records are returned as stored evidence without a query. The recovery transport interface has only a query method, not prepare or commit.

A pass uses one shared wall-time allowance rather than resetting the budget per key. It stops querying on the first error or exhausted deadline and marks later unresolved records deferred. Each accepted result is synced individually before reporting a terminal transition. Successful earlier transitions are retained even when a later query fails. A sync failure fences the journal and cannot be acknowledged as a newly terminal result. The deadline is cooperative around storage operations; synchronous filesystem calls are not claimed to have a hard cancellation bound.

The response contains complete per-key records, `queried`, `deferred`, per-key errors, aggregate counts, `journal_head_before`, `journal_head_after`, and a stop reason. `pass_complete` means the pass finished without being stopped; it does not mean every effect resolved. Check `unresolved` and `all_terminal`. The response always says `mutation_dispatched=false`, `safe_to_retry_same_effect=false`, and `current_freshness_proven=false`.

This is one foreground pass, not a background polling task or a temporal game-goal obligation. A caller may perform another read-only reconciliation pass later, but must not retry the mutating commit. Changed journal heads invalidate old discovery continuations as usual. Recovery-only sessions refuse `fortress.wait` without connecting or writing.

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

The recovery-only increment added 14 Rust regression tests: seven adapter journal tests, four bounded-query tests, and three Unix MCP-handler tests. They cover read-only replay, authority/custody, no-write behavior, retention/replay bounds, pagination and actual offline handlers.

The receipt-verified reconciliation increment adds 19 Rust tests: ten coordinator/identity tests, seven wire tests, and two complete-response reservation tests. The existing offline handler tests also cover refusal of live wait, including injected clock grants. Covered cases include missing or corrupted receipts, native prepares mistaken for failed effects, generation loss, opposite observed state, canonical batch ordering, preflight refusal, partial progress, shared deadlines, explicit required wire fields, exact binary lengths and oversized-frame fencing.

These Rust tests have **not been compiled or executed in this editing environment**: Rust, Cargo and rustfmt are unavailable. Independent Python SHA-256 calculations verified the checked-in prepare/receipt test vectors, but that is not Rust execution.

The changed actual native producer was compiled and executed through `scripts/test_live_control_outcomes_native_mock.py` with both GCC and Clang, each using C++17 and `-Wall -Wextra -Werror -pedantic`. Each run passed 75 C++ assertions plus three independent Python SHA-256 comparisons. The tested producer SHA-256 is `80bb0c427ce94ecee41b86c52ea721e979cd15b1f371e0a4a7a27f5aa410a6ff`. Tests exercise time-advanced prepare replay, actual observed setter failure, incomplete post-set clock evidence, post-set exception, duplicate suppression, generation reset, auth refusal and the fixed RPC method set. The local harness used the existing mock interfaces and hash helpers; this is not a generated-protobuf or real DFHack build.

Reproducible focused commands on a configured checkout:

```bash
python3 scripts/test_live_control_outcomes_native_mock.py --compiler g++
python3 scripts/test_live_control_outcomes_native_mock.py --compiler clang++
cargo test --locked -p dfmcp-adapter control_effect_journal
cargo test --locked -p dfmcp-adapter pause_reconciliation
cargo test --locked -p dfmcp-adapter live_control_rpc
cargo test --locked -p dfmcp-mcp live_control_server
```

These commands are not a substitute for the repository's full verification and qualification requirements. No Rust compilation/test pass, filesystem power-loss campaign, disposable-fort control campaign, registry entry, monotonic-floor advancement, qualified server artifact, production runner, or admitted mutation capability is claimed. Protocol 1.7 remains unadmitted. Its existing method/field layout and receipt domains are unchanged; these source fixes do not inherit qualification from older plugin bytes.

The next evidence-bearing step is qualification of this exact narrow boundary, not adding broader mutation families.
