# Native excavation-conditioned clock control — development 1.18

`bridge/dfhack-excavation-run-v1_18` makes the engine in `EXCAVATION_RUN.md`
accessible through six fixed DFHack RPC methods. This closes the native gap
between designating a small area and advancing the game until its observed floor
condition or another stop trigger occurs. It does not perform designation itself.

The profile is **unadmitted development source**. The actual translation unit
has compiled/executed against explicit SDK/protobuf API doubles on GCC and Clang.
There is no real DFHack SDK build, live-game campaign, Rust adapter/MCP endpoint,
durable external coordinator, qualified artifact or production admission here.
Do not substitute this native-lifetime state for the required durable intent,
current capability/clock-lease policy and reconciliation in a future integration.

## Operator gates and fixed methods

Build the directory with an exact matching DFHack SDK and its unchanged relative
`../common` headers. The game process requires:

```text
DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18=1
DFMCP_EXCAVATION_RUN_TOKEN=<32..256-byte operator secret>
DFMCP_EXCAVATION_RUN_ALLOW_CLOCK=1
```

The clock switch is required for new preparation and commit and is rechecked
immediately before an unpause. Revoking it leaves authenticated observe/query/
cancel available; revoking either switch never disables an already-owned safety
stop callback. Production `DFMCP_ADMITTED_BRIDGE_PROTOCOL` is refused. Use only
trusted local native transport, a disposable fortress, and no competing clock
controller. This profile does not fence UI input, another plugin or another process.

The protobuf package is `dfmcp.excavation_run.v1_18`, the plugin
`dfmcp_excavation_run_v1_18`. Exactly Handshake, ObserveRun, PrepareRun, CommitRun,
QueryRun and CancelRun are registered with zero flags. There is no arbitrary
command, Lua, job enum, native address, filesystem path or method selector.
All game access runs under native suspension. No game pointer survives a call.

Every request has required bearer/nonce/version fields. Nonces are 16..64 bytes;
version is exactly 1.18. Unknown fields, missing required fields and surplus or
missing operation-specific fields fail closed. Parsed requests are at most
2048 bytes; full replies at most 4096. DFHack's pre-parse transport bounds remain
an upstream requirement and are not established by this application-size check.

Handshake has no optional fields. ObserveRun has exactly x/y/z/width/height.
PrepareRun additionally carries key, game_ticks, wall_ms, expected_capture,
plan_digest, stable_samples, stable_ticks, interval_ticks and max_gap_ticks.
CommitRun and CancelRun have only key, plan_digest and prepare_token. QueryRun
has only key and plan_digest. Failure codes are 1 authority, 2 protocol,
3 shape/bounds, 4 source/precondition, 5 native/capacity, 6 stale witness,
7 identity conflict, 8 ownership/drain. A failed commit reply is never proof
of nonapplication; query its original key/digest rather than repeating unpause.

## Observation and outcome meaning

The one-level rectangle is at most 8x8, within map and signed-16-bit tile space.
`Maps::getTileBlock` does not allocate absent blocks. Missing and hidden cells
carry presence tags only. Hidden cells are not passed to tile-type validation or
shape/liquid/designation interpretation. Visible cells retain normalized shape,
liquid depth and dig designation. These are not complete terrain/safety semantics.

The fixed goal is all visible FLOOR, liquid zero, dig zero. It allows constructed
floors and does not attribute change to an operation. It neither examines a halo
nor establishes miner reachability, material, temperature, structural support,
nearby water/magma, occupancy, job completion or safety. Visible dry unsatisfied
starting terrain and a paused game are mandatory. An already-true floor condition
refuses preparation rather than unpausing needlessly.

Matching samples must advance game time and respect the selected minimum interval.
Both required count and stable tick span must hold. Contrary observations reset
stability even between eligible samples; excessive capture gaps reset it too.
The retained complete latest sample and counted-window fields are native-reported
sampled evidence, not a replayable log of every intermediate sample or proof of
continuous stability. No observation from map/1.5 or dig/1.16 is rebound into this
profile merely because coordinates, folder or site happen to agree.

A trigger and a clock outcome are separate. FloorObserved records the sampled
condition before the safety pause. Only Stopped plus pause_verified proves a
historical native pause. A FloorObserved record may still be Stopping or may
later become SourceLost. Neither result discharges a mining-effect journal or
permits a repeated unpause. Hidden/missing targets, observed liquid and capture
failure request a safety stop without claiming floor satisfaction.

Clock checks are independent of terrain capture: a broken tile read can still
be followed by a same-source pause and verification. Incarnation/folder/site/map
size changes invalidate ownership without pausing the replacement. The native
setter independently checks identity, even when capture has thrown.

Game/wall limits, external pause, source changes and invalid/regressed clocks
precede a new goal assertion. At or beyond the game-tick limit there is no new
floor-goal success. Limits are stop triggers at callback opportunities, not
hard-real-time or exact-tick guarantees. Overshoot and current pause must not be
inferred away; a stopped record is historical, not a promise of present state.

## Lifetimes and unload

The existing bounded-run engine owns the one-shot unpause and subsequent safety
pause retries. Exact replay never renews the 60-second preparation lifetime or
repeats an unpause. The last sample and trigger survive a lost reply allocation.
Destroying an RPC connection leaves the native update owner intact. Query never
services the clock, acquires a new sample or changes a retained record.

Cancellation before dispatch retires preparation. Cancellation after dispatch
requests a pause; failed readback keeps Stopping and blocks new runs. Only safety
pause attempts repeat. The Idle/Busy/Closing gate vetoes SC_BEGIN_UNLOAD while an
owner remains active. That hook takes no core lock; a later update drains the
owner before unload can be retried. There is no disable hook to orphan callbacks.
Forced termination, real plugin-manager behavior and power loss remain unqualified.

Retention is 256 records with no eviction. Records survive connection loss, not
plugin unload/process death. An absent key is unknown historical effect, never
safe retry. Full durable external intent/receipt custody and explicit recovery
remain mandatory before any production-style integration.

## Canonical bytes

All integers are unsigned big-endian. Text and embedded captures have u16 byte
lengths. Keys are ASCII `[A-Za-z0-9_.-]{1,128}`. Folder is 1..512 UTF-8 bytes without
NUL. Capture is bounded to 1024 bytes; record to 3072.

Capture: magic DFMEC018, the existing 35-byte DFMRO013 clock-field encoding,
site/map-x/map-y/map-z u32, folder text, x/y/z/width/height u32, cell-count u16,
then y/x-ordered cells. Presence is 0 missing, 1 hidden, 2 visible. Visible-only
payload is shape/liquid/dig u8. Shape is 0 unsupported, 1 empty, 2 wall, 3 floor,
4 ramp, 5 ramp-top, 6 up-stair, 7 down-stair, 8 up/down-stair. Liquid and dig
are 0..7. Embedding a clock encoding does not share another profile's authority.

Spec: game_ticks, wall_ms, samples, stable_ticks, interval, max_gap as six u32.
Plan: SHA256(`dfmcp-excavation-run-plan/1` + NUL + spec + full capture).
Token: first 16 bytes of SHA256(`dfmcp-excavation-run-token/1` + NUL + key text
+ plan). Hashes are commitments/integrity checks, not signatures or capabilities.

Record: DFMER018, key text, spec, before capture length/bytes, plan32, token16,
phase/reason/unpause-attempted/pause-verified/tick-known u8, observed-tick u64,
trigger u8, stable-samples u32, first-stable/counting/last-capture ticks u64,
sample-present u8, optional sample length/bytes, receipt32. Unknown observed tick
is encoded zero. Receipt covers all preceding bytes under
`dfmcp-excavation-run-receipt/1` + NUL. The producer revalidates state/window/sample
consistency before encoding. A pending record's digest is not terminal evidence.

Clock phases/reasons retain the definitions in BOUNDED_SIMULATION_RUN.md.
Triggers are None=0, FloorObserved=1, SourceChanged=2, CaptureFailure=3,
Unobservable=4 and LiquidObserved=5. A complete goal window can coexist with an
unverified stop. All production readers must preserve that distinction.

## Executed tests

```sh
python3 scripts/test_excavation_run_engine.py --mutations
python3 scripts/test_excavation_run_bridge.py --mutations
# Repeat both with --compiler clang++.
```

Each compiler passes 7030 engine assertions and 961 actual-handler assertions
(16 engine and 12 handler scenarios), with C++17, warning denial and nonrecovering
UBSan. Five actual C++ capture/plan/token/prepared/stopped encodings agree with
independent Python struct/hashlib reconstruction. The largest legal two-capture
record is 1991 bytes; its complete response fits the 4096-byte bound.

Eight separately compiled mutants are rejected across the two suites: repeated
same-tick evidence, gap-streak preservation, treating walls as floors, ignoring
commit capture equality, exposing hidden payload, accepting unknown fields,
removing clock permission and removing independent replacement-source fencing.
Tests exercise actual engine/handler code, but native SDK/protobuf interfaces are
explicit doubles. No real SDK/plugin ABI, live fortress, Rust/MCP, physical power
loss, full repository qualification or production admission is established.
