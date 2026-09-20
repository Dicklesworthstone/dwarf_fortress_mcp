# Bounded simulation through the eleven-tool MCP interface

`dfmcp-live-run-dev-server` connects the existing native **run/1.13** bridge to
`dfmcp_adapter::bounded_run` through a typed Rust RPC client and durable effect
coordinator. It does not spawn the Python developer client, a shell, or another
runtime. The source uses the existing pinned modern FastMCP/Asupersync entry.

**Evidence level: implementation source plus independent Python model/static
checks only. Rust/Cargo/rustfmt were unavailable; the new Rust tests and MCP
process have not been compiled or executed here.** No actual SDK build, live
fortress campaign, full qualification, compatibility admission or production
runner change is established. Use disposable forts only after the necessary
native/runtime qualification. The older read and pause profiles are unchanged.

Read `BOUNDED_SIMULATION_RUN.md`, `BOUNDED_SIMULATION_RUST.md`, and
`BOUNDED_SIMULATION_COORDINATOR.md` for the native ownership and durable ordering
contracts. In particular, limits are callback stop triggers, not exact-tick or
hard real-time promises; another plugin/UI/controller is not fenced.

## Operator configuration and fixed modes

The server accepts only these `DFMCP_*` variables:

```text
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_JOURNAL=/absolute/private-0700-directory/runs.bin
DFMCP_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_RUN_TOKEN=<32..256-byte operator credential>
DFMCP_RUN_ALLOW_CLOCK=1
```

Endpoint/token are used only in online modes. The separate clock opt-in is
required only for `control`; omit it otherwise. Any other `DFMCP_*` state,
including production admission state, is refused. The game process independently
requires the native run/1.13 development opt-in and matching token. Journal paths,
endpoints, secrets, protocol versions and method names cannot be selected by MCP
arguments. Current file custody supports Linux x86_64/aarch64 only.

Source entry point, **not executed or qualified here**:

```bash
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-run-dev-server
```

`fortress.open_session` accepts `mode`, `max_wall_millis`, `max_bytes`,
`max_output_tokens`, and `max_game_ticks`. Defaults are `offline`, 5,000 ms,
16 MiB, 16,384 output token-proxy units, and 1,200 game ticks. Limits are bounded
by 60,000 ms, 16 MiB, 65,536 token-proxy units and 1,200 game ticks. Whole-response
output budgeting uses four UTF-8 bytes per token-proxy unit, not measured model
tokenization. Lower request limits only narrow the retained session budget.

The three modes are fixed until session close:

- `offline`: open an **existing** read-only journal. No endpoint/token read,
  native source construction, journal creation, repair, or native call occurs.
  Query/explain/doctor and stored terminal wait results remain available.
- `recover`: open an existing writable journal with fresh Query authority.
  Explicit waits can query the native source and retain verified receipts;
  prepare, commit and cancellation remain forbidden even with injected grants.
- `control`: require the operator clock opt-in, establish the exact native
  source and software binding, and open/create the matching journal. Plan and
  guarded ControlClock grants are fresh session authority, never restored bytes.

Only one session/file owner is retained per server process. Online operations
create one foreground connection and drop it when the operation ends. They do
not automatically retry. The native owner, not an MCP connection or timer,
continues to enforce an already committed run's stop.

## The control loop

Use the existing tool names; no twelfth tool is added.

```json
{"tool":"fortress.open_session","arguments":{"mode":"control"}}
```

The open result includes `session_id` and a retained control observation. A fresh
`fortress.observe` captures the clock without unpausing:

```json
{"tool":"fortress.observe","arguments":{"session_id":"<session>"}}
```

Use **`result.observation.witness`**, not the Agent Turn's coordination hash, to
prepare a finite run:

```json
{"tool":"fortress.plan","arguments":{
  "session_id":"<session>","idempotency_key":"run-001",
  "game_ticks":100,"run_wall_millis":5000,
  "expected_witness":"<observation witness>"
}}
```

The coordinator re-observes the exact paused source, syncs sealed intent before
native preparation, and syncs the native preparation result before replying.
Preparing does not unpause. Commit uses the returned `result.effect.plan_digest`:

```json
{"tool":"fortress.commit","arguments":{
  "session_id":"<session>","idempotency_key":"run-001",
  "plan_digest":"<sealed digest>","confirm":true
}}
```

Commit checks current authority, the complete game-tick allowance, exact source
and paused witness, then syncs `dispatch_started` before the sole unpause
attempt. No response, malformed evidence, lost connection, output failure or
uncertain sync can make that attempt replayable. An unresolved run blocks new
control in this journal, including a new key. Do not interpret acknowledgements
as production/goal completion.

```json
{"tool":"fortress.wait","arguments":{
  "session_id":"<session>","idempotency_key":"run-001",
  "plan_digest":"<sealed digest>","max_wall_millis":5000
}}
```

Wait makes **one QueryRun sample**, persists its verified receipt, and returns.
It does not loop, unpause, re-prepare, retry commit, or extend the native run
budget. A still-running result is pending. Missing native records are unknown,
not non-application. A stopped record is historical pause evidence, not proof
that the game remains paused. Observed tick overshoot is exposed.

## Cancellation, close, and handoff

`fortress.cancel(scope="effect", idempotency_key=..., plan_digest=...)` retires
an undispatched local intent without a native call, or syncs cancellation intent
before requesting the native safety pause. Only a safety pause can repeat;
unpause cannot. Disabling operator clock authority prevents further control
requests without revoking the native owner's already committed bounded stop.

`fortress.cancel(scope="session")` releases custody only when no pending or
unresolved record remains. Explicit `release_for_recovery=true` releases even
unresolved/fenced custody so a new process/session can reopen the journal. It
**does not cancel effects, pause the game, erase the journal, transfer native
ownership, or claim quiescence**. The retained record and native stop remain.

A new offline/recovery session discovers work without transcript memory:

```json
{"tool":"fortress.query","arguments":{
  "session_id":"<new session>","state":"unresolved","limit":2
}}
```

Query is local and supports `all`, `pending`, `unresolved`, and `terminal`, with
1..8 whole records. Continuations bind session, journal identity, exact head,
filter, limit and offset. The bounded 64-entry cursor cache can expire older
handles; reopening or head changes require restarting pagination. Keys/digests
inside verified records remain discoverable after restart. `fortress.explain`
retrieves one exact record; `fortress.doctor` verifies custody and reports counts.
These do not acquire native observations. Healthy local history remains usable
after a native failure. `fortress.checkpoint` and `fortress.restore` refuse: a run
journal is not a game checkpoint or restore facility.

## Authority, orientation and budgets

Native run/1.13 lacks a fortress folder/site identity. The control domain is
therefore explicitly NIL/source-bound. A named-fortress grant is not accepted
as proof that this native endpoint is that fortress. No named-fortress, global
clock lease, cross-controller exclusion or production mutation admission is
invented. Software/generation/actual connect endpoint are bound to the journal.

Every success/error uses the shared Agent Turn builder. The response anchor is
the published **coordination journal root**, not canonical world state. Its
epoch is the bound source generation, sequence is the journal transition count,
hash is the journal head, and tick=0 is an explicit non-game-time sentinel. The
native precondition observation is separate. Continuity remains indeterminate
for game history. Current pause and goal completion are explicitly unproved.
Production provenance is never inherited into this isolated packet.

Current Query authority is required to project retained records. Active-work
counts and up to four references remain visible, with explicit omitted counts
and local-only scope. An unverified/unbound empty array does not prove absence.
Read-only discovery recommendations carry cost/risk/evidence fields but grant
no authority. Recovery never promotes itself to control.

Each request reserves complete response space before journal/native work: 8 KiB
base plus 4 KiB per selected row. Complete journal verification, the fixed
handshake and native calls have separate conservative byte reservations. Native
connect/read/write and the coordinator's work share a narrowing wall-time
allowance; socket deadlines cannot be renewed by later steps. Runtime I/O masks,
cancellation, development isolation and operator clock enablement are checked
at effect boundaries. Storage calls remain cooperative, not hard real-time.

## Executed checks and unexecuted tests

Twelve Rust MCP dispatcher/presentation regression groups are registered, bringing
this run integration to 34 Rust groups across codec, RPC, journal and MCP source.
They cover the lifecycle and offline reopen, ambiguous commit recovery without
recommit, mode/authority isolation, local/native cancellation, output refusal,
whole-row cursors, failed-source history access, absent-record uncertainty, and
runtime/environment refusal. **All remain uncompiled and unexecuted here.**

Six independent Python model/static groups passed:

```bash
python3 scripts/test_bounded_run_mcp_reference.py
```

They check the eleven registered tool names and binary/module wiring, specification
boundaries, response/work reservation arithmetic, 2,056 pagination cases, and 27
conservative reference packets measuring 6,288..14,791 UTF-8 bytes. These are
**not actual Rust serialization, MCP dispatch, native RPC or live-game tests**.
The separate seven journal-reference groups also pass. Full pinned-toolchain
build/Clippy/tests, real private-file campaigns, actual modern MCP process tests,
DFHack SDK/plugin qualification, live-fort fault campaigns and admission remain
required. No production runner or existing wire generation is changed.
