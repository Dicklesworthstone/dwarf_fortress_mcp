# Fortress-bound conditional simulation through MCP

`dfmcp-live-order-run-dev-server` connects the existing native **order-run/1.14**
plugin to the typed Rust adapter and durable Rust coordinator through the eleven
`fortress.*` tools. It does not spawn a Python client, shell or extra runtime.
The existing pinned modern FastMCP/Asupersync entry owns the server runtime.

**Evidence: implementation source and independent Python model/static checks.
Rust, Cargo and rustfmt were unavailable. The Rust tests, generated handlers and
MCP process have NOT been compiled or executed here.** Real SDK/ABI, live-game,
power-loss and full repository qualification remain outstanding. This is not
production admission. Use disposable fortresses only after the necessary native
and runtime qualification; concurrent controllers are not globally fenced.

Read `ORDER_CONDITION_RUN_RPC.md`, `ORDER_CONDITION_RUN_RUST.md` and
`ORDER_CONDITION_RUN_COORDINATOR.md` for the native, typed evidence and durable
ordering contracts. The new machine contract is `architecture/order_run_mcp_v1_14.json`.
Existing timer-only run/1.13, read profiles, native schemas, production runners and
compatibility admission are unchanged. The Rust binary journal is deliberately
not interchangeable with the standalone Python developer client's JSONL journal.

## Operator configuration and fixed modes

Only the following `DFMCP_*` variables are accepted:

```text
DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14=1
DFMCP_ORDER_RUN_WORLD_FOLDER=region1
DFMCP_ORDER_RUN_SITE_ID=7
DFMCP_ORDER_RUN_JOURNAL=/absolute/private-0700-directory/order-runs.bin
DFMCP_ORDER_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_ORDER_RUN_TOKEN=<32..256-byte operator credential>
DFMCP_ORDER_RUN_ALLOW_CLOCK=1
```

The endpoint defaults to numeric loopback port 5000. There is no DNS or remote
plaintext credential transport. The game process independently requires the
native development opt-in and matching token. Clock enablement is a separate
operator decision and is required only for control. Other `DFMCP_*` state,
including production admission, is refused. Tokens are not written to journals
or returned in results. Journal paths and exact fortress selection are not MCP
arguments; the caller may select an order only within the configured fortress.

The final directory/file require exact modes 0700/0600 and private ownership.
The storage implementation currently supports Linux x86_64/aarch64. It takes an
exclusive file lock and refuses links, special files, changed custody and torn
frames instead of silently repairing evidence.

Source entry point, **not built or executed here**:

```bash
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-order-run-dev-server
```

`fortress.open_session` accepts `mode`, `max_wall_millis`, `max_bytes`,
`max_output_tokens`, and `max_game_ticks`. Defaults are offline, 5,000 ms, 32 MiB,
65,536 output token-proxy units, and 1,200 game ticks. Maxima are 60,000 ms, 64 MiB,
65,536 units, and 1,200 ticks. A token-proxy unit means four output UTF-8 bytes,
not actual model tokenization. Requested limits can only narrow session budgets.

Modes remain fixed until close:

- **offline** opens an existing read-only journal. No transport credential lookup,
  endpoint resolution, source construction, native call, initialization or write
  occurs. Query, explain, doctor and stored terminal waits remain available.
- **recover** opens an existing writable journal with fresh Query authority.
  Explicit waits can obtain and retain one native receipt. No clock/Plan grant
  is restored from stored bytes; prepare, commit and cancellation remain denied,
  including attempts with injected broader grants.
- **control** requires clock opt-in and grants fresh named-fortress Query, Plan
  and guarded ControlClock authority. Opening may create matching private storage.
  It establishes a native software/generation/endpoint binding, but does not yet
  observe an order or prove that the declared fortress selection is present.
  The first observation checks the exact configured folder/site against native
  capture, and preparation/commit revalidate that same source and order.

One session/file owner is retained per server process. Online operations create
one bounded foreground connection and drop it after the operation. There is no
automatic reconnect or retry. Native stopping does not depend on that connection.

## Observe, plan, confirm, run, and reconcile

The macro attributes explicitly register the dotted names. The pinned macro's
default is the Rust function name, which would otherwise retain underscores.
There is no twelfth tool or underscore alias in this profile.

```json
{"tool":"fortress.open_session","arguments":{"mode":"control"}}
```

First inspect pending work with `fortress.query`. Opening returns no selected
native capture. A new conditional run starts with one read:

```json
{"tool":"fortress.observe","arguments":{
  "session_id":"<session>","native_order_id":9
}}
```

Use **`result.observation.witness`**, not the Agent Turn's journal-root hash.
`condition` is a JSON string with a closed schema, limited to 2 KiB:

```json
{"tool":"fortress.plan","arguments":{
  "session_id":"<session>","idempotency_key":"order-9-approval",
  "expected_witness":"<observation witness>",
  "condition":"{\"order_id\":9,\"predicate\":\"approved\",\"game_ticks\":1200,\"wall_millis\":10000,\"stable_samples\":2,\"interval_ticks\":10}"
}}
```

Required condition fields are `order_id`, `predicate`, `game_ticks` and
`wall_millis`. Optional `threshold`, `stable_samples` and `interval_ticks` default
to 0, 1 and 1; null optional fields use those defaults. Unknown/duplicate fields,
wrong types and invalid bounds are refused. Predicates are approved, active and
remaining_at_most; flag predicates require threshold=0. Run limits are 1..1,200
ticks and 1..60,000 ms, samples 1..16 and interval 1..1,200 ticks. Sample-count
multiplied by interval cannot exceed the tick horizon. An already-true condition
is refused instead of unnecessarily unpausing. No arbitrary query, reaction,
Lua, command or native enum becomes a condition.

The server constructs a sealed plan from its retained exact capture. Preparation
re-observes, checks authority and synchronizes intent before native PrepareRun;
it does not unpause. Review **`result.effect.plan.plan_digest`** before commit:

```json
{"tool":"fortress.commit","arguments":{
  "session_id":"<session>","idempotency_key":"order-9-approval",
  "plan_digest":"<reviewed plan digest>","confirm":true
}}
```

Commit checks the original paused source/order and full game-tick authority, then
synchronizes dispatch before the sole unpause attempt. Runtime I/O restrictions,
cancellation, exact operator fortress selection and clock enablement are checked
again immediately before native effects. A failure after the dispatch marker is
conservatively unresolved even if the actual request was never sent. No timeout,
missing native record, output failure or journal reopen makes commit retryable.

```json
{"tool":"fortress.wait","arguments":{
  "session_id":"<session>","idempotency_key":"order-9-approval",
  "plan_digest":"<reviewed plan digest>","max_wall_millis":5000
}}
```

Wait performs **one QueryRun**, verifies the complete native receipt, synchronizes
it and returns. It does not poll, acquire a fresh order observation, advance time,
re-prepare, retry unpause or extend the native run. Stored terminal results return
without a native connection. A running receipt is pending; absent evidence stays
unknown. Unsettled/source-lost work blocks new control in this journal.

The receipt separately presents sampled predicate evidence, reported stability,
clock stop reason, verified historical pause, actual observed tick and overshoot.
None proves continuous truth, goods produced, present pause or arbitrary goal
completion. Native callback limits are stop triggers, not hard real-time promises.

## Cancellation, session release and transcript-free recovery

`fortress.cancel(scope="effect", idempotency_key=..., plan_digest=...)` retires
undispatched local intent without a native call, or synchronizes cancellation
before asking the native owner for a safety pause. Only that safety request may
repeat. Failed pauses keep native ownership. Source loss never pauses a replacement
fortress and remains explicitly unresolved. Operator revocation blocks new control
requests but does not cancel the native owner's already committed bounded stop.

`fortress.cancel(scope="session")` normally refuses while unsettled evidence
remains. Explicit `release_for_recovery=true` releases even fenced or unresolved
custody, without requiring the old runtime/clock grant. It does **not** cancel
an effect, pause the game, erase the journal, transfer native ownership or claim
quiescence. A poisoned process mutex may still require process restart; persisted
coordination remains the recovery source.

Open a fresh offline or recover session to rediscover records:

```json
{"tool":"fortress.query","arguments":{
  "session_id":"<new session>","state":"unresolved","limit":2
}}
```

Query accepts all, pending, unresolved and terminal, with 1..8 whole records per
page. Continuations bind session, journal identity, exact head, filter and limit.
The cache retains 64 issued cursors; expiry, reopen or head drift requires a new
first page. Records' keys and plan digests survive restart. Healthy local history
remains available when a native connection fails. Explain retrieves one exact
record; doctor checks local custody/counts, not live-world health. Checkpoint and
restore refuse because an effect journal is not a game save.

Every handler result carries the shared Agent Turn spine. Its anchor is the
**coordination journal root**, with named fortress ID, source-generation epoch,
transition sequence, journal hash and explicit tick=0 sentinel—not canonical
world state. The native capture and witness remain separate. Game-history
continuity is indeterminate. Bounded active-work references and omission counts
support handoff without transcript memory; unverified empty arrays prove nothing.
No production provenance is inherited into this isolated projection.

Each request reserves 16 KiB base plus 16 KiB per requested row, and separate
pre/post journal-view budgets, before native or journal work. Complete packets
are checked against that reservation. Native handshake/calls have separate byte
allowances, and socket/coordinator deadlines only narrow. Storage latency remains
cooperative; output overflow after an effect directs recovery, not retry.

## Validation and remaining gaps

Twelve Rust dispatcher, presentation, mode/authority and tool-definition groups
are registered. Across this adapter, journal and MCP increment there are **39 Rust
regression groups, all uncompiled and unexecuted here**. The actual-dispatcher test
source covers lifecycle, offline reopen, lost commit, no redispatch, confirmation,
injected grants, cancellation, output budgets, cursors, native failure/history,
per-effect revocation and exact dotted generated tool definitions.

```bash
python3 scripts/check_order_run_rust_vectors.py
python3 scripts/check_order_run_journal_reference.py
python3 scripts/check_order_run_mcp_reference.py
```

The new eight-group MCP reference checks explicit names/registration and entry
wiring lexically, 160 accepted/488 rejected condition cases, duplicate/type/bound
refusals, 2,056 whole-row pagination cases, cursor-binding and reservation models.
Twenty-seven deliberately conservative packet models measure 8,028..73,963 bytes,
below the 147,456-byte eight-record reservation. These are **not actual Rust
serialization, macro expansion, MCP dispatch or native/runtime execution**.
The earlier six journal-reference groups and canonical fixture reference also pass.

Remaining work includes actual pinned-toolchain compilation/format/Clippy/tests,
modern MCP process execution, real native SDK/ABI and live-fort campaigns,
power-loss recovery, full qualification and admission. Global cross-controller
leases, production/goods-completion proofs and journal format migration are not
implemented or implied by this development integration.
