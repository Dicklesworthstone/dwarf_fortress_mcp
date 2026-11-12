# Wait for several laboratory conditions

The existing laboratory `fortress_wait` tool can follow an explicit set of one
to eight retained watches through the same foreground world advance. Each watch
keeps its own original condition, failure condition, cadence, deadline and
stability requirement. This makes it possible to monitor several prerequisites
or stop when any selected outcome is satisfied without losing the other
selected watches' due samples.

Register conditions through the existing
[condition-watch query operations](LAB_CONDITION_WATCHES.md), then pass their
exact returned handles:

```json
{
  "name": "fortress_wait",
  "arguments": {
    "session_id": "<owning-session-id>",
    "max_game_ticks": 200,
    "watch_ids": [
      "<first complete watch handle>",
      "<second complete watch handle>"
    ],
    "watch_policy": "all_satisfied"
  }
}
```

`watch_ids` must contain one to eight unique, retained, session-owned laboratory
handles. Empty sets, duplicates, foreign handles, released handles and malformed
handles refuse the whole request before advancement or sampling. Selection is
explicit: omitting the field never means all current watches. Records and
triggering handles are returned in canonical handle order, independent of the
caller's order.

`watch_policy` accepts only `all_satisfied` or `any_satisfied`. Omitted or null
means `all_satisfied`; a non-null policy requires `watch_ids`. The selectors
`plan_digest`, `watch_id` and `watch_ids` are mutually exclusive. Existing
duration, selected-plan and single-watch calls preserve their behavior and
response shapes. The public surface still has eleven tools.

## Completion and attention

The call evaluates the current published world before advancing. At every
subsequent frontier it evaluates the selected set atomically and applies this
deterministic order:

1. Stop for a selected `failed` watch.
2. Otherwise stop for `expired`, then `invalidated`, then `cancelled`, then
   `blocked_unknown`.
3. If no selected watch needs that attention, apply the requested policy:
   `all_satisfied` needs every selected retained outcome satisfied;
   `any_satisfied` needs at least one.
4. Otherwise stop if paused or at the requested tick limit.

`triggered_watch_ids` identifies all selected members matching the winning
attention status, or the satisfied members when the group policy succeeds.
Attention wins even when another selected member is satisfied. Watches remain
retained after any stop; this call neither cancels nor releases them.

**Group success aggregates retained watch outcomes.** A terminal watch keeps
its historical result when time moves or its predicate later changes.
Consequently `all_satisfied` does not prove all selected predicates are true
now, or that they were true at the same time. Each record retains its own
`last_evaluated_anchor`, `evaluation_current`, definition and evidence. The
response marks `completion_scope: "retained_watch_outcomes"`,
`terminal_evidence_is_historical: true` and
`simultaneous_current_truth_proven: false`. For a condition that must hold
together at one observation, register an appropriate Boolean `all` condition
as one watch.

## Independent cadences and deadlines

Every bounded progression interval ends no later than the earliest next due
sample or original deadline among unfinished selected watches. Existing action
and recovery deadlines, the ordinary slice ceiling and the remaining requested
ticks can shorten it further. Each frontier is one complete shared source;
the watch engine decides which members can add positive samples at that tick.

For example, suppose both watches are registered at tick `T` in an unpaused
world without intervening action-generated observation gaps:

| Watch | Success threshold | Cadence | Required stable samples | Satisfies at |
|---|---|---|---|---|
| A | Tick at least `T + 10` | 7 ticks | 3 | `T + 28` |
| B | Tick at least `T + 8` | 5 ticks | 2 | `T + 15` |

The shared frontiers include `T + 5`, `7`, `10`, `14`, `15`, `21` and
`28`. With `any_satisfied`, the wait returns at `T + 15`. With
`all_satisfied`, it continues to `T + 28`, retaining B's terminal evidence
from `T + 15`. Terminal members no longer constrain the next interval.

Repeated identical anchors add no samples. Same-tick changes cannot add
positive stability. False or unknown success, unknown failure and skipped
observation sequences reset an unfinished streak. Internal action polls or
successor dispatches may introduce sequence gaps within a progression slice;
these gaps remain visible. A due sample at the exact deadline can satisfy a
watch, while a later sample cannot establish late success.

## Complete response and atomic publication

A successful call includes `selected_watch_set_wait` with schema
`dfmcp.selected-watch-set-wait/1`:

| Field | Meaning |
|---|---|
| `watch_ids` / `watch_policy` | Exact immutable membership and chosen group policy. |
| `records` | Complete original definition, retained summary and bounded evidence for every selected watch; no transient batch row flags. |
| `stop_reason` / `triggered_watch_ids` | Winning attention, group success, pause or tick limit, and the members responsible. |
| `group_satisfied` | A complete published evaluation satisfied the requested group policy without a higher-priority attention condition. |
| `selected` / `satisfied` / `all_satisfied` / `any_satisfied` / `all_terminal` | Counts and aggregate retained status; these do not substitute for `group_satisfied`. |
| `evaluation_complete` | Whether the complete selected evaluation was published at the reported frontier. |
| `sampling_rounds` / `samples_added` | Inspection rounds and aggregate new samples accepted during this call. |
| `source_work` | Accepted source-inspection cost, per-source size ceilings and call-wide limits. |

The existing `wait_progress` reports actual elapsed ticks, action polls and
successor dispatches. The current Agent Turn preserves all baseline and watch
catalogs, pending plans, actions and game obligations, including unselected
work. Unselected watches are not sampled.

At each frontier the watch engine prepares all selected updates in one
candidate store. It renders the complete records and current Agent Turn under
the current response budget before replacing the watch root. A refusal changes
none of that frontier's selected watch states; no valid prefix is committed.
After successful publication the facade returns that exact packet without
further fitting or clipping.

The initial complete packet must fit before any game-time progression. Each
later accepted world slice is persisted, or a durability fence is exposed,
before another slice proceeds. If a later selected publication refuses, real
time and earlier accepted samples remain published. Authorized error details
keep actual `wait_progress` and the current world separate from the last
accepted watch evidence. Incomplete records are marked not current, and
`needs_poll` reflects whether an unfinished record's last evaluated anchor
differs from the world. The complete selected error and progress survive every
fitting output tier; an impossibly small cap uses the existing explicit
irreducible-error response.

## Authority, budgets and Rust entry points

Current whole-fortress Observe and Query grants are required. Positive
requested ticks also require ControlClock. When the world is initially
unpaused, the entire requested horizon must be authorized before progression;
each sampled frontier is checked again. A zero-tick call needs no prior commit
or clock grant. A paused world stays paused.

Source limits apply to each complete observed world. The call shares one
wall-time deadline and permits at most 4,097 inspection rounds: the initial
inspection plus at most 4,096 foreground slices. One set shares the same
predicate-evaluation budget at each frontier. The full response separately
must fit `min(max_bytes, max_output_tokens * 4)`. Existing
[same-session response negotiation](LAB_SESSION_OUTPUT_BUDGET.md) can make a
later retry possible when the initial packet is too large.

Direct Rust callers may use:

```rust
fortress_wait_for_watches(session_id, max_game_ticks, watch_ids, watch_policy)
```

The new registered `fortress_wait_for_targets` handler accepts all six optional
wire arguments. The earlier four-argument `fortress_wait_for_selection`,
two-argument wait, selected-plan and scalar-watch wrappers remain available,
as do their existing handler aliases.

Watch state and handles remain process-local and non-durable. Raw set selectors
and policy are recorded exactly, but make the session replay bundle
nonreplayable. This feature adds no native method, detached poller, automatic
continuation, unpause, mutation authority or production admission.

The argument schema is
[`lab_wait.schema.json`](../schemas/lab_wait.schema.json); the machine-readable
contract is
[`laboratory_selected_watch_set_wait_v1.json`](../architecture/laboratory_selected_watch_set_wait_v1.json).
The dedicated regression module covers mixed cadences, deadlines, attention
precedence, historical terminals, authority and budgets, exact membership,
atomic publication refusals, preserved peer work and replay handling.
Compilation and runtime evidence are recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md); authored tests alone
do not establish runtime qualification.

Owning bead: `df-lab-structured-query-f7q.3`.
