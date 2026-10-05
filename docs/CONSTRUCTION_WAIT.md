# Bounded foreground construction waiting

`track_construction_plan.py wait` drives an existing receipt-linked goal through
multiple complete, durable observations in one foreground call. It does not
create a monitor, place furniture, advance game time, or start a background worker.

```sh
python3 scripts/track_construction_plan.py wait \
  --journal /private/construction/bedroom.plan-monitor \
  --wait-samples 8 --poll-ms 100 --timeout-ms 10000
```

Use the unchanged isolated query environment in `RECEIPT_CONSTRUCTION_PLAN.md`.
The original receipt selection, endpoint, game deadline, cadence, stability and
lifetime observation allowance remain immutable. Wait-specific flags are refused
for every other operation. A terminal goal returns offline without native contact
or journal writes; an active goal requires current query authority.

## One owner and one allowance

`scripts/construction_wait.py` retains the caller's opened journal and the same
shrinking wall, RPC, network, disk and work budget across all samples and delays.
Each acquisition uses the existing one-shot receipt bracket and complete verified
operations capture. It is a new explicit sample, not a retry of a failed read.
No sample receives renewed allowances. A full 32-target acquisition needs at
least 72 RPC calls, so the existing 327-call budget can stop a wait before its
requested sample count. Additional pages consume that same allowance.

Every sample reserves the whole final response, synchronizes read intent before
native contact, verifies the complete sample, reserves its resulting output, and
synchronizes evidence before acknowledgement. Query authority and original goal,
endpoint and file custody are rechecked during delays, before native acquisition,
around publication and after final serialization. A failed read or append aborts
the call without another acquisition. The retained read intent remains unknown;
a later explicit call uses normal interrupted-read replay and resets stability.

The default slice allows eight samples, bounded at 1..32. Wall polling delay is
100 ms by default, bounded at 10..5000 ms, with cooperative checks at intervals no
longer than 50 ms. This delay is not the immutable game-tick sampling cadence.
No sleeps are used to establish timing assertions in the deterministic tests.
Two consecutive samples without an advancing game tick stop the slice. That is
not a claim that the game is currently paused and never grants unpause authority.

## Result and continuation

The existing complete 64-KiB response includes `result.wait`, with schema
`dfmcp.construction-wait/1`, requested limits, newly published sample count and:

- `terminal`: the retained goal reached or already had a terminal outcome;
- `sample_limit`: this foreground slice used its requested sample count;
- `game_tick_not_advanced`: repeated sampled game ticks;
- `rpc_allowance` or `wall_allowance`: insufficient remaining allowance to start
  another acquisition or delay. Wall margin is cooperative, not hard real time.

An ordinary slice stop is not goal success. Read the existing progress phase;
only the original all-target condition and stability evaluator can establish
sampled satisfaction. Another explicit `wait` resumes the same durable goal.
Terminal failure, expiry, invalidation and cancellation remain immutable.
Neither satisfaction nor stopping discharges or retries original placement effects.

## Evidence scope

On this increment, 20 deterministic executor tests and 12 TCP/private-journal/CLI
tests passed. The real command ran in subprocesses against a joined protocol peer,
including a paged 32-target inventory, durable completion and offline replay,
interrupted trailing receipt recovery, revocation, source drift, sample and RPC
limits, and unchanged start/sample/inspect/cancel behavior. The peer and placement
records are explicit fixtures, not a real DFHack plugin or executed game effects.
This is focused Python development execution only: no Rust/MCP build, native SDK,
live game, physical power-loss, full-workspace qualification or production admission.
Owning beads `df-dfhack-bridge-plane-c-pic.4` and `.5` remain open.
