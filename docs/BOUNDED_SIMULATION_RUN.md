# Bounded simulation runs: isolated native run/1.13 development profile

## What is implemented

The comprehensive plan's section 12.9 requires bounded unpause rather than an
indefinite pause toggle. `bridge/dfhack-run-v1_13` supplies six fixed native RPCs:
`Handshake`, `ObserveRun`, `PrepareRun`, `CommitRun`, `QueryRun`, `CancelRun`.
A committed run requests 1..1,200 game ticks and 1..60,000 milliseconds. The native
DFHack update callback owns the stop even after the RPC connection disappears.
Only one run may own this plugin's clock at once. A run must start from a paused,
loaded fortress with an exact generation/dispatch-sequence/tick/pause witness.

This is an explicitly unadmitted native development profile, **not** a new MCP
server, an admitted clock lease, durable coordinator, or production runner.
Existing pause/1.7, read profiles, compatibility registry, and production map are
unchanged. Cross-plugin/UI/remote-process exclusion is not established. Operators
must not run a competing controller against the same fortress.

## Safety and completion semantics

Preparation expires after 60 monotonic seconds; retrying does not renew it.
Commit revalidates the complete witness and retires stale preparations. Before
calling the native unpause setter, it publishes ownership, the stop deadline, and
an attempted-effect marker. A setter exception or lost reply never makes an
unpause retryable. Idempotent commits return the retained record, not another
setter call. The maximum is 256 retained records per plugin lifetime; capacity
exhaustion refuses new work instead of evicting replay protection.

Tick limit, wall limit, explicit cancellation, external pause, clock regression,
and native observation/setter failure terminate or stop the run. Cancellation
before dispatch retires preparation without touching the game. Cancellation after
dispatch requests a safety pause; failure to verify the pause keeps the record in
`stopping`, retains ownership, and blocks new runs. Only safety pauses may repeat.
A pause readback can be known even while the game tick is unknown.

Limits are **stop triggers checked at native callback opportunities**, not hard
real-time or exact-tick guarantees. The game/DFHack thread can stall. Actual
observed ticks, including overshoot, are recorded; no production completion,
goal predicate, rollback, or continuous event history is inferred. `stopped`
means a pause was observed for that source at that time, not that the fortress
must still be paused when a later query arrives.

World/map changes retire preparations and mark an active run `source_lost`
without pausing a replacement fortress. Source loss is not a verified stop.
Native records survive connection loss but not plugin unload or process death.
Absence after restart does not prove that a prior unpause never happened.

## Unload ownership

DFHack's `SC_BEGIN_UNLOAD` is the unload-veto boundary; a failure returned only
from `plugin_shutdown` is too late to preserve update callbacks. The plugin uses
an atomic Idle/Busy/Closing gate. Commit claims Busy before unpause. An unload
attempt publishes a drain request and refuses while Busy; subsequent update
callbacks attempt and verify the safety pause. Retry unload after quiescence.
No core suspension is acquired from inside the plugin-access-lock unload hook.
New preparations/commits are refused once draining begins, until plugin reload.

There is no disable hook: the update callback remains active throughout the
loaded lifetime. DFHack may emit its diagnostic about the absent enabled var;
this does not disable callbacks. Forced host termination is outside this guard.
Upstream API review used `DFHack/dfhack` PluginManager.cpp blob
`34095898db9320f6804a7264ea51e593f34fde1f`; an exact target SDK and live campaign are
still required before relying on this lifecycle in deployment.

## Development configuration

Build with an exact matching DFHack source tree using the directory's
`CMakeLists.txt`; retain the relative `../common` headers. In the **game process**:

```text
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_TOKEN=<32..256-byte operator secret>
```

Requests also carry the secret, a 16..64-byte nonce, and exactly protocol 1.13.
Presence of `DFMCP_ADMITTED_BRIDGE_PROTOCOL` is refused. Credentials are not game
compatibility or production admission. Use a trusted local connection: DFHack's
native TCP envelope is not encrypted. No arbitrary command, Lua, native pointer,
DF job enum, client-selected plugin, or method dispatch is exposed by this profile.

## Wire contract

`DfmcpRunV1_13.proto` is the envelope contract. Requests are limited to 2 KiB after
protobuf parsing; required presence, unknown fields, and operation-specific
optional-field sets are checked. Handshake/Observe have no operation fields.
Prepare has key, ticks, wall_ms, expected_observation, and plan_digest.
Commit/Cancel have key, plan_digest, and prepare_token. Query has key and digest.
The underlying DFHack server's pre-parse frame limit remains an upstream concern.

All application integers below are unsigned big-endian. Text keys are ASCII
`[A-Za-z0-9_.-]{1,128}`, prefixed with a u16 byte length.

- Observation: `DFMRO013`, generation u64, dispatch sequence u64, tick u64,
  loaded u8, clock_valid u8, paused u8. Exactly 35 bytes, booleans 0/1.
  An invalid clock has tick=0; an unloaded source has all three flags false.
- Plan: SHA-256 of `dfmcp-bounded-run-plan/1` + NUL + game_ticks u32 + wall_ms u32
  + exact observation bytes.
- Token: first 16 bytes of SHA-256 of `dfmcp-bounded-run-token/1` + NUL +
  length-prefixed key + plan. This is a commitment, not independent authorization.
- Record: `DFMRE013`, key, game_ticks u32, wall_ms u32, observation, plan (32),
  token (16), phase u8, reason u8, unpause_attempted u8, pause_verified u8,
  tick_known u8, observed_tick u64, receipt (32). At most 374 bytes.
  Unknown observed_tick is encoded zero. Receipt is SHA-256 of
  `dfmcp-bounded-run-receipt/1` + NUL + all preceding record bytes.
  Even a prepared/pending record has an integrity receipt; this is not terminal
  evidence or a signature from a separately trusted authority.

Phase codes: prepared=0, running=1, stopping=2, stopped=3, refused=4, source_lost=5.
Reason codes: none=0, tick_limit=1, wall_limit=2, cancelled=3, external_pause=4,
native_failure=5, clock_regression=6, source_changed=7, shutdown=8, stale=9.
Failure codes: credentials/opt-in=1, version=2, request=3, source/paused-state=4,
native/capacity=5, stale witness=6, identity conflict=7, ownership/drain=8.

A Query with no effect_record is an absent retained record, never proof of no
historical effect. **Any failed/absent Commit reply must be treated as potentially
dispatched** and reconciled by key/digest; failure_code alone cannot prove where
failure occurred. Reply nonce/version and record plan/token/receipt must all be
verified before using evidence. Replies carry current plugin generation; retained
records carry the generation at preparation, which can be historical.

## Executed evidence and missing qualification

```bash
python3 scripts/test_bounded_run_native.py
python3 scripts/test_bounded_run_bridge.py
```

The engine suite uses deterministic callback doubles. The bridge suite compiles
the actual plugin translation unit against clearly labeled SDK and protobuf API
doubles generated from the checked-in schema. GCC and Clang each passed 341
handler assertions with warnings as errors and UBSan. Python independently
verified three C++ canonical snapshot/plan/record vectors. Cases include client
service destruction, lost response allocation after dispatch, competing plans,
failed pause retries, cancellation, invalid clocks, wall-only stop, replacement
fortress isolation, and an unload request injected inside the unpause setter.
Source-bound reports and build products are retained by the scripts.

This does not execute real protobuf serialization, actual SDK/ABI, DFHack's plugin
manager, a live fortress, Rust, MCP, or full repository qualification. Native and
live fault campaigns, Rust authority/journal integration, the existing eleven-tool
MCP integration, and admission remain outstanding. No bead is closed by this work.
