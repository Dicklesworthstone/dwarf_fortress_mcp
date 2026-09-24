# Excavation-conditioned bounded clock runner

The native-independent engine in `bridge/common/excavation_run.h` composes the
unchanged bounded-run clock owner with a strict floor-goal monitor. This increment
provides an executable C++ engine. The subsequent isolated native RPC integration
is specified in `EXCAVATION_RUN_NATIVE.md`; Rust/MCP integration is not supplied.
It advances `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-action-coordinator-exec-ero.4`; their broader acceptance remains open.

## Goal and source

The selected 1..8 by 1..8 rectangle at one z-level must become entirely visible
FLOOR, zero liquid, and no dig designation. This is the same narrow endpoint
condition as the standalone excavation monitor, not evidence of mining causality,
structural safety, pathfinding, material value or absence of nearby hazards.
The initial selection must be visible and dry, the game paused, and the goal
not already true. An already true goal does not justify unpausing to sample it.

A complete capture binds native incarnation, dispatch sequence, game tick,
folder/site, map dimensions, exact rectangle and all cells. Missing and hidden
cells have presence tags and no attribute payload. Equality revalidation occurs
before prepare and again before the first unpause. No field is borrowed from a
different plugin's observation universe. The engine treats the supplied plan
identity as opaque; the 1.18 wire boundary recomputes its separate commitment.

## Native-owned progress and stopping

Limits are 1..1200 game ticks, 1..60000 milliseconds, 1..128 required samples,
0..1200 stable ticks, positive minimum sample interval and maximum sample gap.
The earliest eligible sampling window must fit strictly inside the run horizon.
Same-tick reads never manufacture samples; contrary evidence resets the streak
even between polling intervals. An excessive observation gap resets stability.

Observed wet, hidden or missing target cells trigger a safety stop rather than
waiting through unobserved or explicitly wet terrain. Capture failure also stops.
Clock and source reads are independent of terrain acquisition, so a failed map
read cannot prevent a pause attempt or pause readback. Game/wall limits, external
pause and clock/source changes take precedence over a new goal claim.

The complete latest sample and trigger are retained before attempting a safety
pause. A goal trigger does not imply the pause succeeded: failed verification
retains Stopping and exclusive local ownership, blocks new runs, and permits only
safety-pause retries. Cancellation before commit retires preparation without an
effect. Cancellation/shutdown after commit use the same stop/drain owner.
Incarnation/folder/site/dimension changes never pause a replacement source.

Both one-shot unpause and prepared/terminal replay use the existing clock engine.
Preparation expires after 60 monotonic seconds; replay cannot renew it. Capacity
is 256 keys with no eviction. Terminal state/sample history is immutable. This
is native-lifetime idempotency, not durable recovery, a global controller fence,
a game checkpoint or continuous history. An integration must retain durable
intent before dispatch and reconcile unknown effects without repeating unpause.

## Executed engine evidence

Run `python3 scripts/test_excavation_run_engine.py --mutations`, also with
`--compiler clang++`. Both compilers pass 16 scenarios / 7030 actual C++ assertions
with C++17, warnings denied and nonrecovering UBSan. Coverage includes all 576
shape/liquid/designation combinations, every eight-sample Boolean trace, stale
captures, repeated/contrary samples, observation gaps, failed pause ownership,
source substitution, invalid clocks, ambiguous unpause, cancellation and capacity.
Four separately compiled weakened implementations fail their regressions.
No sleeps or real clocks are used by the engine tests.

The tested unchanged `bounded_run.h` matches repository blob
`b8f54a21bee65670389e34ed2f69e2b0d1adc4fc`. This is not real DFHack SDK/plugin,
live-fortress, Rust/MCP, physical power-loss or full repository qualification.
Existing native profiles, dependencies and production admission are unchanged.
The separate 1.18 handler also has executed SDK-double tests, not a real SDK build.
