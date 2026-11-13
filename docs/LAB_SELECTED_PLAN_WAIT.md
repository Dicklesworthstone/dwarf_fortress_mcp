# Wait for one original laboratory project

The existing laboratory `fortress_wait` MCP tool accepts an optional
`plan_digest`. It identifies one original commit owned by the current session,
or an exact original recovered from a durable fortress with complete step evidence.
The wait advances the same shared laboratory world and observes the same
retained actions, but returns at a meaningful outcome for that selected project
instead of always consuming the entire requested time allowance.

```json
{
  "name": "fortress_wait",
  "arguments": {
    "session_id": "<owning-session-id>",
    "plan_digest": "<64-lowercase-hex-original-commit-digest>",
    "max_game_ticks": 2000
  }
}
```

Use the original commit's digest even when a later plan is now the session's
most recent work. The requested and actual sealed digests are reported
separately when witness rebase created a retained alias. A pending review,
another live session's commit, or restored source history cannot substitute for
the exact original and its complete action evidence.

Omitting `plan_digest`, `watch_id` and `watch_ids` preserves the existing
duration-based wait. The additive [single-watch selector](LAB_CONDITION_WATCHES.md)
and [exact watch-set selector](LAB_SELECTED_WATCH_SET_WAIT.md) are mutually
exclusive with `plan_digest`; they do not change selected-plan semantics. Direct Rust
callers may continue using `fortress_wait(session_id, max_game_ticks)`;
`fortress_wait_for_plan(session_id, max_game_ticks, plan_digest)` exposes the
additive selector. The registered handler keeps the wire name `fortress_wait`
and the public surface still has eleven tools.

## Completion is the original goal

Every successful targeted response contains `selected_plan_wait`, with schema
`dfmcp.selected-plan-wait/1`. It distinguishes:

| Field | Meaning |
| --- | --- |
| `requested_plan_digest` / `plan_digest` | Requested retained alias and actual original sealed commit. |
| `goal_truth` | Current truth of the complete original project. |
| `stage_goal_truth` | Current truth of this stage's sealed terminal predicate. |
| `actions_verified` / `actions_terminal` | Original action proof, independently of current goal truth. |
| `physical_quiescent` | Whether this original commit's physical work is known quiet; unknown is explicit. |
| `reserve_frontier` | Evidence at the original absolute reserve tick, when requested. |
| `complete` | Original goal, verified actions, quiet physical work and required reserve evidence all agree. |
| `stop_reason` | Why this call returned at its observed frontier. |
| `continuation_request` | A separately reviewed continuation, only when the existing current-authority oracle permits it. |
| `source_work` | Bounded source inspection accounting. |

For a staged resource-backed production or room project, a completed harvest
does not establish the finished room or output quota. When that stage is quiet
and a later pursuit is eligible, the wait returns `needs_continuation` with
the ordinary review request. It does not compile or dispatch the next stage.
Continuation descendants are separate commits and are never silently included
in the selected action set.

An ordinary production goal can become true and later false as the fortress
consumes food or drink. A selected wait returns when completion is established
at an observed frontier, leaving the unused tick allowance unspent. This is
sampled completion evidence, not a promise that the goal remains true forever.

## Stops and future reserves

`complete` identifies proven selected completion. `tick_limit` means this
request used its time allowance while selected work remained unfinished;
`paused` means no further time could advance. An explicit later wait can
continue the same original commit under current authority.

`needs_continuation`, `goal_unmet` and `goal_unknown` preserve the distinction
between quiet stage work and current original-goal truth. `failed`, `cancelled`,
`cancellation_pending`, `indeterminate` and `work_unknown` return control for
inspection or reconciliation. Such a stop does not cancel remaining physical
work and does not claim that other projects are finished.

A future reserve keeps its original absolute tick. The progression interval is
clipped to that frontier so its evidence can be sampled exactly. Before it,
current stock alone does not establish future completion. A known deficit may
need a separately reviewed continuation. False or unknown exact-endpoint stock
produces `reserve_unmet` or `reserve_unknown`. A past frontier with no exact
observation produces `reserve_unobserved`; stock observed later cannot repair
the missing endpoint. Reserve inspection never extends sealed action deadlines.

An exact observation is retained with the original owner and its bounded
snapshot history, so later bounded waits can reuse the same endpoint. The
response identifies reuse explicitly and charges the historical snapshot
alongside the current source. This read-derived cache is process-local:
`durable` and `survives_restore` are false. Restart or restore does not recreate
past reserve evidence from current stock.

## Authority, bounds and recovery

### Resume an interrupted durable project

After reopening a durable fortress, pass the original `plan_digest` returned
before the restart. The new session can monitor that exact project with current
Observe and Query authority; positive game ticks additionally require
ControlClock. Designate, Construct, production, labor and Plan permissions are
not reconstructed from historical custody. Existing physical world work can
progress without a new dispatch of its original action.

The recovered selector verifies the original source and sealed archive, the
accepted recovery lineage, and every original step's archived receipt or
matching carried obligation. This includes verified prerequisite steps that
have already retired from active monitoring. Unfinished obligations keep their
original deadline and cadence and require new stability observations after
recovery. A current true goal does not substitute for missing action evidence.

Recovered responses use `selection_scope: "exact_recovered_original"` and
`selection_origin: "durable_recovery"`. They retain the originating owner as
historical metadata and report `owns_original_dispatch: false` and
`dispatch_handles_restored: false`. The exact original digest is required;
recovery does not infer live request aliases or adopt another live session's work.

Deferred steps that were never dispatched remain untouched. Once the actual
world work is quiet, the response reports the unmet original goal and
`not_dispatched_actions`. A supported semantic goal can offer the existing
separately reviewed continuation. A raw action list requires a new explicit
review. Selected waiting never executes a recovered deferred step.

After a carried project completes, a zero-tick selected wait can inspect the
same recovered outcome without committing more work. Restore abandons the old
pursuit and makes it ineligible. Missing, duplicated, contradictory or
unanchored step evidence refuses before time advances. Old fully retired
commits whose durable history no longer contains complete action receipts
remain ineligible: verified historical goal achievement alone does not prove
all original actions.

### Live selection and shared bounds

Selection validates the complete original commit, action identities and step
order before progression. Current Observe and Query authority admit the
inspection; positive requested ticks also require ControlClock authority.
Every deferred effect retains its original scoped capability, precondition,
deadline and idempotency checks. The request uses the existing foreground
slice, action-poll, deferred-dispatch, source, wall-time and game-tick bounds.

Each successful foreground slice publishes world state, proof and original-goal
history before the next slice. Publication failure stops further advancement.
Other committed plans can make progress during this shared world time, and
their receipts remain visible. Selection neither replaces the pending review
nor changes the latest-action identity.

`wait_progress` reports requested, advanced and remaining ticks. The selected
outcome and these elapsed-work counters survive every fitting response-budget
tier together. Replay records the optional selected digest and reproduces the
same early frontier. A malformed or unavailable target never falls back to
the latest plan and never advances time.

The implementation is part of the deterministic laboratory. It does not add
native DFHack methods, production protocol admission, a background scheduler,
or new plan authority. Validation evidence is recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md).

Owning bead: `df-lab-selected-plan-wait-s4q`.
