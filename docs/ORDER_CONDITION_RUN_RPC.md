# Native order-run/1.14 RPC

The [order-condition engine](ORDER_CONDITION_RUN.md) now has an isolated native
DFHack plugin at `bridge/dfhack-order-run-v1_14`. Its six operations are
`Handshake`, `ObserveRun`, `PrepareRun`, `CommitRun`, `QueryRun`, `CancelRun`.
The machine contract is `architecture/order_run_v1_14.json` and the protobuf
schema is `DfmcpOrderRunV1_14.proto`. This does not widen run/1.13 or any read,
creation, pause or production-admission generation. It is not a Rust/MCP runner.

## Operator and authority boundary

Build against one exact matching DFHack source tree, retaining the relative
`../common` headers. The game process requires:

```text
DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14=1
DFMCP_ORDER_RUN_TOKEN=<32..256-byte operator secret>
```

Requests need the matching token, 16..64-byte nonce and exact protocol 1.14.
Presence of `DFMCP_ADMITTED_BRIDGE_PROTOCOL` is refused. Use only trusted
loopback transport: the native DFHack envelope is not encrypted. No arbitrary
commands, Lua, paths, method names, job enum or reaction strings are accepted.
Use a disposable fortress; no real SDK build or live campaign is established.

The selected order is captured under the same native suspension as generation,
world-folder/site identity, tick and pause state. All queue IDs are validated
before absence is reported. Only the existing four fully recognized finite wood
furniture templates can be used for running. An unknown template can be observed
but cannot become a running goal. Every setter independently rechecks the exact
source identity, including during failures of order/clock reads. This is not a
global lease; competing UI/plugins/controllers must not control the same game.

## Protocol flow

Handshake takes only required envelope fields. ObserveRun additionally takes
`native_order_id`. PrepareRun takes key, game_ticks, wall_ms, expected_capture,
plan_digest, native_order_id, predicate, threshold, stable_samples, interval_ticks.
Threshold is explicitly zero for approved/active. CommitRun and CancelRun take
key, plan_digest and prepare_token; QueryRun takes key and plan_digest only.
Unknown fields and wrong presence sets are rejected. Request size is at most
2 KiB after protobuf parsing; DFHack's pre-parse frame limit is upstream.

Preparation verifies the plan checksum and exact currently captured source and
order. Commit revalidates again and invokes the existing one-attempt clock owner.
The native update callback checks stop conditions without an RPC client. A
read-only QueryRun does not capture progress, service timers, or touch the clock.
A cancelled/stopped/replaced record is historical and cannot be replayed to run.
No receipt, rejected reply or missing native record can prove that a failed
CommitRun was never dispatched. Reconcile by key/digest rather than retrying it.

A predicate-triggered safety pause uses the underlying clock's `cancelled`
reason, plus the distinct `predicate_observed` trigger and its exact sample.
These are separate facts: observing the predicate does not prove the pause, and
observing a pause does not prove the predicate. Clock limit, external pause,
source loss and failed reads cannot manufacture a predicate-triggered result.
The stability count is a native-reported sampled count, not a full sample trace
or proof of continuous truth. Negative samples reset it, including early ones.

A run that cannot verify a safety pause retains ownership and refuses a new run.
The Idle/Busy/Closing atomic unload gate is claimed before unpause.
SC_BEGIN_UNLOAD requests drain and vetoes unloading while Busy; the callback
continues pausing until verified, then unload can be retried. A failure returned
only from plugin_shutdown would not provide this protection. No disable hook or
background thread is added. Host termination/stall remains outside this guard.

## Canonical wire bytes

All integers are big-endian. `field` is u16 length followed by exact bytes. All
hash domains below are terminated by NUL. Checksums are not signatures or grants.

Capture (62..573 bytes): `DFMOR014`, generation u64, dispatch sequence u64, tick
u64, site u32, field(folder UTF-8), paused u8, target ID u32, next_order u32,
present u8, recipe u8, total i32, remaining i32, status u32. Captures require a
loaded valid clock; absent target backing fields are zero. Recognized recipes
require bounded coherent finite counters and only known status bits.

Plan (93..604 bytes): `DFMOP014`, game_ticks u32, wall_ms u32, predicate u8,
threshold u32, samples u32, interval u32, field(capture). Plan digest is SHA-256
of `dfmcp-order-run-plan/1` plus NUL plus plan bytes. Token is the first 16 bytes
of SHA-256 of `dfmcp-order-run-token/1` plus NUL plus field(key) plus plan digest.

Record (at most 1,425 bytes): `DFMOE014`, field(key), field(plan), plan digest
(32), token (16), phase u8, clock reason u8, trigger u8, attempted u8, verified
pause u8, known tick u8, observed tick u64, stable count u32, counted tick u64,
field(last/trigger capture or empty), receipt (32). Unknown observed tick is zero.
Receipt is SHA-256 of `dfmcp-order-run-receipt/1` plus NUL plus preceding record
bytes. Maximum exact sizes are exercised by the actual C++ encoder.

The retained sample freezes once stopping begins. It is not necessarily the
stop readback; these ticks and pause flags can differ. A terminal record or a
receipt from an older generation does not establish current pause or freshness.
Native records survive client disconnection, not plugin/process restart. No
durable Rust coordinator or MCP integration for this profile is claimed.

## Executed validation

```bash
python3 scripts/test_order_run_engine.py
python3 scripts/test_order_run_bridge.py --mutations
```

GCC and Clang each compile the **actual plugin translation unit** and pass 529
handler assertions under warning denial and UBSan using explicit SDK/protobuf
API doubles. Python independently verifies three C++ capture/plan/record vectors.
Three separately compiled mutants are rejected on both compilers: removing
folder/site setter fencing, weakening full template recognition, and counting
same-tick callbacks as fresh stability. Twenty template-field drift cases,
malformed queues, disconnected client, post-unpause response allocation loss,
replacement fortress and unload-during-unpause scenarios execute.

This is not real protobuf serialization, SDK/ABI, DFHack plugin-manager execution,
a live game, Rust/MCP, full repository qualification or compatibility admission.
Before deployment those exact native, runtime, live-fort and admission campaigns
remain required. The baseline engine's separately executed evidence is retained
in ORDER_CONDITION_RUN.md. No existing receipt or bead is promoted by this work.
