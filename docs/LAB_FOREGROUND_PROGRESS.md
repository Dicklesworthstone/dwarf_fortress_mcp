# Complete dependent laboratory work in one bounded wait

`fortress.wait(max_game_ticks=...)` now advances already committed laboratory
work through intermediate observations. A dig → build → production plan can
complete inside one sufficiently long wait: excavation receives time, its
proof releases construction, and construction's proof releases production.
The caller need not divide a reviewed wait into many smaller tool calls to
make its dependency graph progress.

This is the existing deterministic laboratory and existing eleven-tool API.
There is no background worker, automatic new plan, native bridge dispatch,
or new mutation permission. The original sealed actions and obligations remain
the only work eligible to dispatch.

## Progress at observation boundaries

The first round observes the current frontier without advancing time. This
allows already eligible successors to begin before the requested time is
consumed. Later rounds advance by at most 100 ticks, shortened by any active
step's sealed poll interval, exact remaining deadline, recovered obligation
interval, or remaining requested time. The reference default obligation cadence
is 10 ticks; 100 is only the outer ceiling and quiet-world interval.

Each round polls retained owned actions in their original commit order, with
prerequisites before dependents. Multiple earlier plans remain included when
a later plan becomes the session's most recent one. A shared member's wait can
advance physical world time, but does not take over a peer's action handles or
dispatch authority.

Current scoped effect authority and preconditions are rechecked when a deferred
step becomes ready. Cancellation, Unknown predecessor evidence, failure and
original deadlines retain their existing semantics. There is no new preparation
or deadline extension. A pause dispatched by a ready successor stops further
time advancement during the same wait. A zero-tick call performs one observation
round; unresolved recovered work can be observed without a live action handle
or clock authority.

The requested duration remains an upper bound on elapsed game time, including
all rounds together. Completing an action does not consume the entire remainder
at once: later world changes and goal observations continue in bounded rounds
while the fortress remains unpaused. A satisfied goal can subsequently become
false through consumption; both facts remain visible.

## Publish before continuing

A round executes on a laboratory clone and becomes current only after its
advancement, ordered polls, current authority checks and cooperative deadline
checks all succeed. A failed round publishes no partial laboratory state.
Earlier successful rounds remain real progress.

The owning session then observes recovered physical work and publishes the
world, step dispositions and first original-goal achievements together. This
happens before another round may advance time or dispatch a successor. Durable
publication failure fences the remaining wait. The existing durability gate
must save the current frontier before a later call can cause more effects.

Intermediate original-goal achievement is retained even if consumption makes
the goal false before the same wait returns. In a durable fortress, that first
achievement survives reopen. This is sampled observation history, not a claim
that predicates were continuously true between samples.

The final Task projection renders the already observed action receipt. Response
formatting does not poll again, dispatch additional work, or move a historical
proof to the final world anchor. `polled_actions` contains the latest result of
each action seen during this call in original commit order, including actions
that finished and retired before the final round.

## Bounds and partial progress

One request pins its owning session, exact frontier and aggregate action budget.
The tracked action set can shrink but cannot gain identities during the call.
The driver refuses duplicate IDs and reversed prerequisite order before effects.
It allows at most 1,024 tracked actions, 4,096 successful rounds and 65,536 action
polls. The negotiated entity and deferred-action limits may be lower. Wall time
uses one cooperative deadline spanning the whole foreground operation; checks
occur at round and poll boundaries, not by preempting arbitrary code.

The result adds `wait_progress`:

| Field | Meaning |
| --- | --- |
| `requested_game_ticks` | The duration admitted for this call. |
| `advanced_game_ticks` | Actual elapsed game time across successful rounds. |
| `remaining_game_ticks` | Requested time that has not elapsed. |
| `observation_rounds` | Successful atomic rounds, including the initial zero-time observation. |
| `action_polls` | Aggregate polls across those successful rounds. |
| `deferred_actions_dispatched` | Original prepared actions dispatched during this call. |
| `method` | `bounded_foreground_dependency_progression`. |

Error responses retain the original error and these counters, plus
`partial_progress`. When current Observe authority permits, they also carry the
actual world anchor, pause state, latest action receipts and physical work.
The Agent Turn adopts that authorized frontier even though the wait failed.
`durability_pending` distinguishes a frontier awaiting successful persistence.
Each action row keeps its original receipt's `observed_anchor`; its current
`work_state` is separate, and forecast comparisons use the original proof tick.

A work-budget stop does not make the entire requested time elapse. An explicit
later wait can use the remaining duration after its current authority and any
durability fault are resolved. Recommitting is unnecessary.

## Validation boundary

Actual-source tests exercise the real StaticPlanner, MemoryAdapter, original
obligation runtime and finite effect timeline. They compare a single coarse
dig/build/brew wait with explicit smaller waits, check exact deadlines and
stability samples, stop on a deferred pause, retain original proof history,
and refuse changed authority, stale frontiers, invalid action domains and
exhausted aggregate work.

MCP regressions additionally cover the final response/Task state, first goal
achievement before consumption and durable reopen, publication-failure fencing,
and zero-tick recovered observation. Full MCP facade execution is separate from
the actual-source module harness and remains unverified until its dependency
graph can compile. No native or live qualification is established.
