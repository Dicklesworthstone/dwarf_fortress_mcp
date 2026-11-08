# Bounded game time for one original workshop

Work item: `dfmcp-reality-wiring-proof-4bf.14.6`, under the original first-fortress
production prerequisite. This component reuses Workshop/1.20 and Run/1.13. It
introduces no native methods, wire generations, dependencies or admission.

## Durable original clock owner

`dfmcp_adapter::workshop_construction::clock` associates separately reviewed
finite runs with one exact, acknowledged original Carpenter placement. Its
immutable target contains the Workshop journal identity, source binding, full
canonical `Placed` receipt and an explicit absolute clock deadline. Workshop
placement itself has no persisted game-time goal deadline; this clock horizon
is separately selected by the operator and never described as inherited goal
authority.

Every retained Run key binds that target and its Run journal identity. Changing
the original workshop, placement receipt, source, horizon or journal cannot
adopt old run history. The same loopback endpoint and DF/DFHack software pair
must match, while Workshop and Run plugin generations are independently pinned.
Run/1.13 has no fortress folder or site identity. Matching these pins and a
paused tick is not proof of the same owned game process or an atomic snapshot.

Named-fortress authority and native NIL clock authority are supplied separately.
Both the current and opening grants are checked, and request budgets can only
narrow the original allowance. Each run is at most 1,200 game ticks and 60,000
native wall milliseconds, and its whole requested horizon must fit the frozen
absolute deadline. Callback stop limits are not exact-tick or hard real-time
guarantees. Observed overshoot remains visible.

Preparation reobserves the exact paused clock, synchronizes the original Run
intent, and invokes the supervising fresh-workshop guard before native
preparation. Commit consumes its process-local review, reobserves the exact
clock, synchronizes `DispatchStarted`, then invokes the guard again before the
single native unpause attempt. A lost reply, failed guard, storage failure,
changed source, rejected confirmation or wrong seal cannot recreate that review.

The guard must return a complete fresh Workshop progress observation for the
original building and material. Only the exact original `InProgress` verdict
at the selected clock tick can support another interval. A complete, absent,
contradicted or unknown original refuses advancement. Readiness exposes stage,
maximum stage and ConstructBuilding job count. It does not prove worker
availability, job suspension state, scheduler viability or construction causality.

The existing append-only Run journal retains the complete native records.
External recovery requires the independently preserved journal identity and
acknowledged head; complete descendants are accepted, rollback and unrelated
histories are refused. Recovery grants Query/reconciliation only, never another
unpause. Offline inspection has no native calls. Unresolved and terminal-unknown
runs block new intervals. Cancellation in the original authorized control session
can retire an unattempted interval or request a bounded native safety stop; it
never repeats unpause or deconstructs the workshop.

The adapter bounds history to 64 intervals. An integrating host must also respect
the original Workshop journal's separate 64-sample capacity. It should durably
record and acknowledge progress at both external guard boundaries so consumed
source continuity survives process recovery. Goal evidence must be sampled
independently after a settled run: a Run receipt is not workshop completion.

## Use the existing Workshop MCP server

`dfmcp-workshop-v1-20-dev-server` optionally owns the clock in the same eleven-tool
session. Keep the ordinary Workshop configuration and add these independent
operator settings:

```text
DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK=1
DFMCP_RUN_TOKEN=<separate operator-held Run/1.13 credential>
DFMCP_RUN_ALLOW_CLOCK=1
```

Add the following closed object to `DFMCP_WORKSHOP_MCP_CONFIG`:

```json
{
  "clock": {
    "initialize": true,
    "generation": 41,
    "deadline_tick": 18000,
    "recovery": null
  }
}
```

These numbers are examples, not observed source identities. Pin the accepted Run
incarnation and an absolute deadline later than the original placement and no
later than the configured authority expiry. The endpoint and software pair come
from the Workshop configuration; no MCP request can select another native source.
The separately held `workshop.clock.journal` lives in the same private directory
as `workshop.journal` and `workshop.ack`. Initialization requires an absent clock
file; it never adopts, repairs or silently replaces existing custody.

The default game-tick allowance remains **zero**, including with this profile
enabled. Open the session with an explicit allowance, for example:

```json
{
  "mode": "control",
  "max_game_ticks": 100
}
```

This permits a reviewed interval of at most that many ticks; opening, observing,
planning and querying still never unpause. The clock profile defaults to 256 MiB
of bounded request work. Each control request prepays the complete Run operation,
original-workshop guard and final custody/output inspection before contacting
either native source. The original Workshop material policy, including the
acknowledged Tree supply profile, remains usable with this option.

1. Observe, plan, review and commit the original Carpenter as usual. Preserve the
   original Workshop acknowledgement and the separate clock recovery pin in every
   successful response. `Placed` remains insertion evidence only.
2. Call `fortress_wait` with `scope="goal"` for one fresh progress sample. When
   the exact original remains incomplete and the clock is idle, the response
   offers a bounded `run_workshop` request through the existing `fortress_plan`:

```json
{
  "kind": "run_workshop",
  "target_id": "<result.clock.target_id>",
  "game_ticks": 100,
  "wall_millis": 5000
}
```

3. Review `result.clock.active_run`, including its own `review_seal`, exact native
   plan, ticks, wall limit, original target and recovery identity. This seal is
   distinct from the construction review. Confirm it through the unchanged
   `fortress_commit(session_id, review_seal, confirmed=true)` arguments.
4. Call `fortress_wait(session_id, scope="clock")`. Each call makes at most one
   original-run reconciliation. While it is running or unknown, no new workshop
   completion sample or interval is offered. A verified settled run triggers one
   fresh original-building sample in the same foreground request. Only that
   Workshop sample can establish completion.
5. If fresh evidence still says `in_progress`, another interval requires another
   plan and review. The recommendation is clamped to the unchanged deadline and
   explicit session allowance. Historical completion, a lost source or a consumed
   guard prevents further advancement. Exhausted bounds produce a blocker,
   never a renewed horizon.

Both preparation and commit guards durably append their fresh progress samples
and synchronize the Workshop acknowledgement before returning to the run owner.
Those samples support the exact incomplete-original precondition; clock actions
do not advertise them as a fresh completion verdict. Each new interval reserves
room for its two guards plus a later goal sample within the existing 64-sample
Workshop limit. `remaining_workshop_samples` exposes that separate capacity.

`fortress_cancel(scope="clock")` in the original authorized control session
retires a local unattempted interval or requests the native safety stop. It does
not cancel or deconstruct the workshop. A consumed review recommends retirement
when that authority is available. Terminal unknown and permanently fenced clocks
recommend inspection of their blocker instead of waiting without useful work.

## Recovery and failed observations

Retain all three custody files, the independently acknowledged Workshop pin and
the clock's `{journal_id, head}` pin. Reopen with Workshop `initialize=false` and
its original acknowledgement. Set `clock.initialize=false` and supply its exact
`recovery` pin. Optional `clock.recovery_generation` pins a separately accepted
later Run incarnation for querying; it does not restore original continuity or
control. `mode="recover"` permits original outcome reconciliation and fresh
Workshop evidence after settlement. `mode="offline"` inspects only local files,
without reading either native credential or contacting either endpoint.

A failed native connection or original progress read can consume a generation
or tick that the RPC cannot safely return. The host therefore sets a sticky
`advancement_fenced` flag after such failures, including failed guard reads,
run reconciliation contacts and post-run or goal progress reads. Restoring old
configuration or a regressed source cannot re-enable another interval. This
fence leaves independently authorized safety stop and read recovery available.
Decoded Run receipt ticks are retained even if a later policy check fails.
Fresh Workshop progress must also be at or beyond the maximum consumed Run and
Workshop tick. The typed Workshop RPC refuses older completion evidence before
the goal journal can retain it; restoring a stale source cannot make completion
green after a later clock receipt.

A new independent clock can also be attached to an already retained original
workshop: close its old session, keep Workshop `initialize=false` and the exact
acknowledgement, configure a genuinely new clock file with `initialize=true`,
and open `mode="control"` with independent clock permission and explicit ticks.
This restores no construction permit and needs no Workshop build opt-in. An
existing clock file cannot use this path to regain control after restart.

`scope="session"` releases all local locks and sockets without cancelling game
effects, deleting custody or claiming pause. Success, failure, oversized output
and session-close packets preserve both original recovery identities and clock
hazards. Current pause, shared native process identity and construction causality
remain unproved; the response keeps those distinctions explicit.

## Evidence scope

The component has independent Rust tests for exact target binding, lifecycle,
guard ordering after durable markers, confirmation consumption, lost replies,
external recovery lineage, deadline and budget limits, authority revocation,
safety stop, corruption, consumed source floors, and complete bounded history.
Execution results are recorded in `IMPLEMENTATION_STATUS.md`; these tests use
the actual adapter owner with injected Run sources and journal storage. They do
not execute the modern MCP transport, real DFHack SDK or a game process.

The host has pure configuration and recommendation regressions plus ten
actual-server stdio scenarios using joined protocol peers and real private files.
Authored tests are not execution evidence: consult `IMPLEMENTATION_STATUS.md` for
the exact commands that completed and the builds blocked by resource limits.
Full source/native/game qualification remains separate. This source does not
establish production admission or close the original untouched-fortress
workshop-and-furniture-output criteria.
