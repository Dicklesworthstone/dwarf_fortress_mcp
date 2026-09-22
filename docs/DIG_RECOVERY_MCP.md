# Mining journal recovery through MCP

`dfmcp-dig-recovery-dev-server` connects an agent to an existing Rust dig/1.16
journal through the frozen eleven-tool interface. This is an explicitly unadmitted
**recovery-only** profile, not a mining-control runtime. It cannot prepare, commit,
cancel a native designation, create a journal, acquire terrain, unpause, checkpoint,
or restore the game. The missing live lease/checkpoint policy is not bypassed.

## Operator configuration

The journal must already exist in the Rust format described in
`DIG_RUST_COORDINATOR.md`. Python intent capsules and their directory registry are
not imported or migrated. Concrete storage requires Linux x86_64/aarch64, exact
0600 single-link regular files under an owned 0700 directory, no-follow path
components and exclusive custody. Do not use a valuable fortress for unqualified
native development.

```sh
export DFMCP_ALLOW_UNADMITTED_DIG_RECOVERY_V1_16=1
export DFMCP_DIG_WORLD_FOLDER=region1
export DFMCP_DIG_SITE_ID=1
export DFMCP_DIG_SCOPE='[0,0,0,63,63,7]'
export DFMCP_DIG_JOURNAL=/private/mining/journal
cargo run --locked --offline -p dfmcp-mcp --bin dfmcp-dig-recovery-dev-server
```

The scope is `[min_x,min_y,min_z,max_x,max_y,max_z]` and must match the journal's
operator binding exactly. Path, scope, fortress, mode and credentials are not MCP
arguments. Scope coordinates are integers 0..32767 with ordered corners. Site ID
uses canonical nonnegative decimal and is at most i32::MAX. Paths are normalized
absolute UTF-8, at most 4096 bytes, with no empty, dot or parent components.

By default the profile opens **Offline**: no token or native endpoint is needed,
no game connection is made, and storage is read-only down to its methods. To
permit explicit query-only reconciliation, independently set:

```sh
export DFMCP_DIG_RECOVERY_ONLINE=1
export DFMCP_DIG_TOKEN='<matching native dig token, 32..256 UTF-8 bytes>'
```

The original endpoint and native software/incarnation come from the verified
journal, never an MCP request or a replacement endpoint variable. The token is
read lazily only when a request actually needs a native query. Local bootstrap,
inspection and already-settled/permanent-Unknown lookups need no token even in
Recover mode. Online reopen re-synchronizes the existing journal, not the game.
The bridge still requires its own existing dig/1.16 opt-in and authentication.
Transport is loopback-only and unencrypted.

Only these seven DFMCP variables are accepted. Other DFMCP profile variables,
`DFMCP_DIG_ALLOW_DESIGNATE`, production admission state and non-exact opt-in values
are refused. Configuration is checked again at request and native boundaries;
changing it requires releasing the session and explicitly reopening. Initial
inspection checks exact fortress/scope through read-only custody before any
Recover-mode resynchronization, then verifies unchanged journal identity/head.

## Agent workflow

The runtime uses the repository's modern-only MCP transport; logical tool names
below use the existing fortress.* registry notation.

1. `fortress.open_session()` returns the session ID, Query-only capability,
   verified journal root/counts and any unsettled key. No journal is created and
   no native connection is opened. Optional ceilings are max_wall_millis,
   max_bytes and max_output_tokens; defaults are 10000, 1073741824 and 8192.
2. `fortress.query(session_id, query)` lists history, reads retained halo tiles or
   returns the embedded schema. `query` is a closed JSON string. Examples:

   ```json
   {"mode":"records","limit":8}
   {"mode":"records","limit":8,"continuation":"<returned 64-character token>"}
   {"mode":"tiles","idempotency_key":"dig-001","plan_digest":"<exact digest>","offset":0,"limit":16}
   {"mode":"schema"}
   ```

   Record pages carry 1..8 whole rows. Tile pages carry 1..16 cells from the exact
   retained plan, with witness and a typed next query. Hidden/missing cells have
   no attribute payload. Record continuations bind the session, journal identity,
   exact head and page width. At most 64 presentation cursors are retained;
   expiration requires restarting discovery, never reconstruction of authority.
3. `fortress.explain(session_id,idempotency_key,plan_digest)` returns the retained
   review, target and shared-block scopes, hidden-neighbor policy and verified
   native receipt. It never reobserves the game.
4. `fortress.wait(session_id,idempotency_key,plan_digest,max_wall_millis)` in Recover
   mode obtains at most one exact QueryDesignation response. The existing codec
   verifies it and the coordinator synchronizes new evidence before a known
   outcome is acknowledged. Terminal and permanent native Unknown return locally.
   Missing source retention or changed software/incarnation preserves uncertainty.
   Queried Prepared evidence is not dispatch permission. No automatic polling,
   reconnect loop, preparation, commit or native cancellation occurs.
5. `fortress.cancel(session_id,release_for_recovery)` releases this session's
   journal lock only. Unsettled or fenced history requires the explicit true flag.
   It never deletes evidence, cancels a native operation, or proves quiescence.
   Explicit release remains available after operator/I/O revocation, subject to
   the owned request's cancellation check. A running request owns the session
   mutex until its blocking operation leaves; a competing request fails promptly.

`fortress.observe` and `fortress.doctor` report verified local coordination
inventory, not current terrain or live game health. `fortress.plan`,
`fortress.commit`, `fortress.checkpoint` and `fortress.restore` return explicit
refusals. The lower `QueryOnly` source also rejects native observation,
preparation, commit and cancellation independently of dispatcher checks.
The underlying fixed RPC negotiation still binds its six existing methods and
handshakes; this profile delegates only the subsequent effect query.

## Orientation, budgets and runtime ownership

Every success/refusal uses the shared Agent Turn builder. The sole unsettled
record is included independently of its alphabetical position in history pages,
with a typed recovery step, count and receipt identity. Inventory is marked
complete only after verifying actual journal custody. If post-operation custody
fails, the response preserves prior verified pending identity as historical and
explicitly withdraws current inventory verification; it does not claim absence.
Stored designation proof is never current terrain, excavation completion,
structural safety, global controller fencing or production admission.

Requests reserve 32768 bytes for the complete response and two complete bounded
inventory reads before the action. Byte ceilings cannot exceed 1 GiB; this is a
conservative accounting allowance, not a buffer allocation. Output tokens use a
clearly labeled four-byte proxy, not measured tokenization; 8192..65536 proxy
tokens are accepted. Wall time is 1..60000 ms and includes blocking-pool queue
time. Online connection, journal, result and final inventory reservations share
one shrinking allowance. Missing budgets refuse before bridge work. Whole JSON
objects are returned; overflow never truncates a safety field. Cursor publication
is staged until both final custody verification and complete rendering succeed.

Async handlers use the inherited `Cx::spawn_blocking` and join the runtime-owned
task. They never use the free helper's fallback threads or detached std threads.
Parent/worker cancellation and I/O permissions are checked explicitly. Dropping a
request marks its worker abandoned; runtime ownership continues until the blocking
operation drains. Kernel filesystem calls are not claimed to be forcibly
interruptible, and a lost response is not proof that no journal append occurred.
A request failure cannot trigger a native mutation in this profile.

The reusable adapter DigSession separately owns the original source across the
future control lifecycle. Its `commit` has no connection factory. This recovery
profile does not expose that control path or supply a permissive mutation guard.

## Evidence and remaining work

This source adds 22 MCP/runtime regression tests and extends the adapter session
suite to 18 tests: 40 new Rust tests across the two session/recovery increments.
Tests cover pending work beyond page one, exact retained tiles, query-only
reconciliation, absorbing Unknown, missing records, sync/custody failure, revoked
runtime capability, cursor publication and complete worst-case response shapes.
**All Rust tests are uncompiled and unexecuted here; Rust/Cargo/rustfmt are absent.**
No Asupersync/MCP stdio, TCP, native SDK, live fortress, power-loss durability,
Clippy or full repository qualification is established by the new source.

```sh
cargo test --locked --offline -p dfmcp-adapter dig_designation::journal::session
cargo test --locked --offline -p dfmcp-mcp dig_recovery_server
python3 scripts/test_dig_recovery_query_schema.py
```

The Python command was executed: 24 valid and 87 invalid query-schema cases pass.
It executes the published JSON Schema only, not Rust parsing, actual routing,
rendering, I/O or journals. JSON Schema's mathematical integer semantics do not
certify Rust lexical number handling or duplicate-key/raw-byte limits. The
source-bound report is `docs/evidence/dig-recovery-query-schema.json`.
Native bytes, production runner map, dependencies and existing Python recovery
are unchanged. A mutation-enabled MCP route still requires actual host
lease/checkpoint policy, execution/qualification of these layers and its own
reviewed effect-boundary integration.
