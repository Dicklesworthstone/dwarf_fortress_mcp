# Finite original-room clock intervals through MCP

`dfmcp-build-placement-dev-server` can advance a separately reviewed finite
run/1.13 interval for the complete original room goal. The workflow is: retain
all original placement receipts, sample the joint terrain-and-furnishings goal,
prepare and explicitly commit one interval, reconcile its original outcome, then
sample the whole room again. The existing eleven tools expose each step.

The [core clock contract](ROOM_CONSTRUCTION_CLOCK.md) specifies identity,
readiness, authority and recovery. This is an unadmitted development composition;
[implementation status](../IMPLEMENTATION_STATUS.md) records its exact tests and
remaining runtime, native and live qualification limits.

## Configure the room owner and a separate clock

Start with [room intake](ROOM_CONSTRUCTION_MCP.md) and
[joint room completion](ROOM_COMPLETION_MCP.md). Keep the original placement and
parent files, configured room journal, independent map/operations credentials,
fortress identity, endpoint and scope. Add these operator settings:

```sh
export DFMCP_BUILD_ROOM_CLOCK_CONFIG='{"path":"/private/furniture/room-clock","initialize":true,"source":{"endpoint":"127.0.0.1:5000","generation":41,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"}}'
export DFMCP_BUILD_ROOM_CLOCK_TOKEN='<matching run credential, 32..256 bytes>'
export DFMCP_BUILD_ALLOW_ROOM_CLOCK=1
export DFMCP_BUILD_MODE=control
export DFMCP_BUILD_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
```

The server accepts exactly **24 `DFMCP_` environment names**. These three room
clock settings are independent of `DFMCP_BUILD_CLOCK_CONFIG`,
`DFMCP_BUILD_CLOCK_TOKEN` and `DFMCP_BUILD_ALLOW_CLOCK`, which belong to ordinary
furniture completion. Neither clock can borrow the other's goal, credential,
review or retained authority. Both clocks may be configured, with distinct files;
unsettled or unverified custody in either blocks fresh completion samples and
new advancement in the other.
The ordinary clock still requires its first accepted ordinary completion source
before combined session opening, as specified by its existing
[activation workflow](FURNITURE_CONSTRUCTION_CLOCK_MCP.md); it does not share the
room clock's first-source bootstrap exception.

The closed clock configuration is at most 8 KiB. `path` must be normalized,
absolute and distinct from every configured placement, parent, monitor and clock
path. Use an owned real `0700` directory and exact `0600` journal. Initialization
exclusively creates an absent file; it never adopts or repairs existing bytes.
The canonical numeric loopback endpoint and exact software versions must match
the original placement source. The run generation remains independently pinned
from furniture, operations and map generations.

Native run/1.13 activation and its matching credential belong in the game
process, outside the isolated MCP environment; see
[bounded simulation runs](BOUNDED_SIMULATION_RUN.md). Readiness also uses the
original furniture credential and independent operations/map credentials. Paths,
credentials, source selectors and policy exceptions are never MCP arguments.

The default checkpoint policy is `required`. It refuses new advancement because
no verified game-checkpoint provider exists. The explicit disposable-fortress
policy permits this development workflow; it supplies no save or rollback.
`DFMCP_BUILD_ALLOW_ROOM_CLOCK` must equal exactly `1` for advancement or native
safety cancellation and is rechecked at native boundaries.

## Establish the first joint source without widening authority

The room clock may be configured before the room goal has its first sample.
Open with an explicit per-operation game-time allowance:

```text
fortress.open_session(max_wall_millis=60000, max_game_ticks=100)
```

The default allowance is zero; the maximum is 1,200 ticks. A positive allowance
requires Control mode and an independently enabled configured clock. It does
not start gameplay. To raise a retained zero allowance, release and reopen the
same original custody with the required recovery references.

Finish every original room-backed batch step with a synchronized `Placed`
receipt. Start the fixed joint monitor with `query` mode
`room_completion_start`, then take its first ordinary sample:

```text
fortress.observe(session_id, selection="room_completion")
```

When the configured clock is a new, absent, unattempted initializer, this one
sample establishes the first furniture/operations/map source association. It
creates **no clock file or owner**. The sampling context cannot manufacture Plan
or ControlClock authority. A second sample is blocked until explicit room-clock
planning opens the exact clock under the then-current retained authority.

If the monitor already has an accepted source when a combined session opens,
opening eagerly creates or reopens its exact configured clock before publishing
the session. Existing files always require acknowledged recovery pins. An
attempted initializer with a retained pin cannot silently become a fresh clock,
and an unrelated file appearing at the configured path is refused unchanged.

## Prepare and confirm one interval for the original room

```text
fortress.plan(session_id, scope="room_clock",
  clock='{"monitor_id":"<original joint room monitor id>","game_ticks":10,"wall_millis":5000}')
```

`clock` is a JSON string with exactly `monitor_id`, `game_ticks` and
`wall_millis`. Bounds are 1..1,200 ticks and 1..60,000 milliseconds, narrowed by
the retained/current session allowance, original maximum sample gap and absolute
goal deadline. Omit placement keys and observation witnesses. Omitted plan scope
preserves the existing placement/furniture-clock behavior; a room interval
requires explicit `scope="room_clock"`.

Preparation audits the original room-backed batch, every placement receipt,
room monitor and configured clock history. After durable run intent, it acquires
one complete paused receipt/map/operations/map/receipt bracket and publishes it
as readiness in the original room journal. All original furnishings must remain
viable pending construction or meet their condition. Hidden or missing required
terrain, changed source identity, removal, regression, an exhausted observation
allowance or an expired goal refuses advancement.

Known unmet terrain may remain pending under explicit global-clock review. The
full terrain counts and bounded ordered deficit list are disclosed together
with every furnishing assessment. This permits the original room goal to remain
unfinished when furniture is complete but terrain is not; it does not establish
that excavation, wall construction, hauling or another causal process exists.

Review `result.room_clock.active_run` and `result.room_clock.readiness`, including
the original monitor identity, interval, before tick, native plan digest, terrain
counts/deficits, aggregate furnishing counts and separate `review_seal`. The full
per-furnishing assessments remain in `result.room_completion.assessments`.
Explicitly confirm:

```text
fortress.commit(session_id, scope="room_clock",
  monitor_id="<original joint room monitor id>",
  review_seal="<returned room clock review seal>")
```

Commit accepts no placement key or plan digest. It consumes the process-local
seal, synchronizes DispatchStarted, and repeats the complete original custody
and fresh joint readiness checks before one native unpause attempt. A failed
publication, wrong seal, lost reply or reopening cannot restore review permission.
An uncertain original run must be reconciled; it cannot be committed again.
An identical interval at an unchanged native observation keeps its original key;
local retirement does not authorize retrying that cancelled intent.

Both readiness captures consume the original `max_observations` allowance once
each. They never establish `first_tick`, advance `counted_tick`, increment the
shared streak or satisfy the room. Fully qualifying readiness may preserve an
existing candidate; known deficits, unknowns, unfinished furniture, changed
same-tick evidence and excessive gaps withdraw unfinished stability. Choose the
original allowance for readiness plus ordinary goal samples; no later operation
renews it. Consuming the last acquisition refuses unpause.

## Reconcile, then take an ordinary joint sample

```text
fortress.wait(session_id, scope="room_clock",
  monitor_id="<original joint room monitor id>", max_wall_millis=5000)
fortress.observe(session_id, selection="room_completion")
```

Wait performs at most one native query for the original unresolved key. It never
starts another interval or samples the room. Terminal history returns locally.
Missing native history remains unresolved and blocks advancement and completion
sampling. Only after configured clocks are verified and settled can the second
call acquire ordinary joint evidence and earn positive completion credit.

Every subsequent interval requires a new plan, review and explicit commit. There
is no background polling or automatic chaining. Clock receipts, readiness and
historical room satisfaction remain separate claims. A pending clock or failed
publication withdraws unsupported current completion guidance.

## Inspect exact history and request a safety pause

```text
fortress.query(session_id,
  query='{"mode":"room_clock","monitor_id":"<original joint room monitor id>","head":"<latest room clock head>","offset":0,"limit":1}')
fortress.cancel(session_id, scope="room_clock",
  monitor_id="<original joint room monitor id>")
```

History inspection is local. Even the first page requires the exact current
head; offsets are 0..63 and page sizes are 1..8. Follow
`runs_page.next_query` unchanged. Every response retains active/uncertain state
independently of pagination and marks readiness as historical pre-dispatch
evidence with no goal sample credit. The requested page limit survives final
custody revalidation.

Before dispatch, cancellation retires local preparation. After possible dispatch,
it may request the original native safety pause under retained and current
named-fortress and independent native Query/ControlClock authority. It needs no
Plan/Observe capability, future ticks or unexpired room deadline. Within an
established owner, loss of original placement/parent/monitor proof does not strand
original-run reconciliation or safety cancellation. Intact clock custody remains
mandatory, and unavailable original proof is reported as unverified.

`scope="room_completion"` cancels only the local joint monitor, including after a
failed clock initializer. It does not pause the game or settle clock uncertainty.
Normal session close requires terminal joint work, settled clocks and resolved
original effects. Explicit `release_for_recovery=true` preserves unresolved
identities while releasing local ownership; it does not settle or cancel them.

## Restart with both original recovery references

Retain `result.room_completion.recovery` and `result.room_clock.recovery` from
successes and failures. Reopen the same original placement/parent/room files;
configure the room monitor pin and set the room clock to `initialize:false`:

```json
{
  "path": "/private/furniture/room-clock",
  "initialize": false,
  "source": {
    "endpoint": "127.0.0.1:5000",
    "generation": 41,
    "df_version": "<exact DF version>",
    "dfhack_version": "<exact DFHack version>"
  },
  "recovery": {
    "journal_id": "<original room clock journal id>",
    "head": "<acknowledged room clock head>"
  }
}
```

Complete validated descendants may recover lost replies. Rollback, substitution,
torn tails, a changed joint definition or changed first source is refused.
Reopening restores history and consumed floors, never a review seal, live read
permit, renewed deadline/allowance or downtime stability. Keep optional clock
configuration on every combined reopen: its omission cannot prove that no
previous clock exists.

Normal `offline` inspects without native contact or writes; normal `recover`
can reconcile an original run with Query authority. Neither starts a new interval
or acquires clock cancellation authority. The established Control owner retains
the separately authorized safety path described above. This increment adds no
independent room-only or room-clock-only startup mode. Existing
`clock-recover`/`clock-offline` belong to ordinary furniture histories and refuse
room configuration.

## Final publication, bounds and evidence

New advancement requires original-file, joint-monitor and both configured-clock
replay after the final runtime callback. Reconciliation and safety cancellation
always require their own intact clock history; unavailable optional original
proof is reported as unverified. Final flags and review permission come from
those checks. Decoded map, operations and run ticks narrow current
authority before later failure; older caller ticks cannot revive expired Query.
Unjournaled failed-read floors remain process-local until a later authorized
persistence boundary, and are never completion evidence.

Complete output remains bounded to 64 KiB; replay, acquisition, synchronization
and output share one shrinking foreground wall allowance and at most 1 GiB of
conservative work accounting. Oversized histories or insufficient budgets refuse
work rather than dropping original targets. Native stop limits are cooperative
triggers and observed overshoot remains visible.

This workflow adds no native method or dependency and changes no compatibility
admission or production runner. Equal paused endpoints provide bracket evidence;
they do not prove an atomic cross-plugin snapshot, shared process identity,
construction causality, present usability, room assignment, access or pathfinding.
