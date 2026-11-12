# Foreground laboratory condition watches

The default laboratory runtime can retain a bounded condition and evaluate it
against explicitly selected foreground observations. Agents can monitor stock
thresholds, labor settings, unit counts and building fields without remembering
raw before-state rows or treating a submitted action as proof of completion.
The same predicates can be inspected without registering a watch.

Watch lifecycle operations use the existing `fortress.query` tool. Pass the
JSON below as the contents of its `mode` string, alongside the owning
`session_id`. They share the `temporal` wrapper with
[query baselines](LAB_QUERY_BASELINES.md). To advance the laboratory while
sampling one retained condition, pass its handle as `watch_id` to the existing
`fortress_wait` tool. To preserve several independent watch cadences during the
same bounded advance, supply an explicit `watch_ids` set and an optional
`watch_policy`; see [waiting for several conditions](LAB_SELECTED_WATCH_SET_WAIT.md).

## Register and inspect a stock condition

In the `starter_fortress` laboratory scenario, the stock ledger is canonical
entity `5001`, generation `1`, and its drink field is `stock.drink`. In another
world, inspect the current entity and generation before choosing the condition.

```json
{
  "mode": "temporal",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "watch",
      "key": "drink-recovery",
      "label": "Observe drink stock recovery",
      "condition": {
        "op": "field",
        "entity_id": "5001",
        "generation": 1,
        "field": "stock.drink",
        "comparison": "ge",
        "value": {"type": "u64", "value": 60}
      },
      "deadline_tick": 1000,
      "poll_interval_ticks": 10,
      "stable_observations": 2
    }
  }
}
```

Choose a future `deadline_tick` within the session's negotiated forward
game-tick horizon. Registration evaluates the current world once and returns
`record.watch`, an opaque `watch:` handle. The same retained key and definition
replay the original record without another sample; changing that definition
under the retained key is a conflict.

Creating a watch does not unpause or advance the world. Perform any desired
planning and commit with their ordinary authority. To inspect the current
published world without advancing time, explicitly poll:

```json
{
  "mode": "temporal",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {"kind": "poll_watch", "watch": "<the complete watch handle>"}
  }
}
```

Registration, polling, listing, cancellation and release perform no native
capture and consume no simulation ticks. Ordinary observations and baseline
queries expose retained metadata without sampling a condition.

## Wait until a retained condition needs attention

Pass the exact returned `record.watch` handle to `fortress_wait`:

```json
{
  "name": "fortress_wait",
  "arguments": {
    "session_id": "<owning-session-id>",
    "watch_id": "<the complete watch handle>",
    "max_game_ticks": 200
  }
}
```

The call first evaluates the current world, then advances through bounded
foreground intervals while the fortress is unpaused. Intervals end no later
than the watch's next due sample or original deadline, as well as any earlier
action or recovery cadence. The existing watch state machine evaluates each
published frontier. The call returns early when the watch is satisfied, fails,
expires, becomes invalidated or cancelled, or encounters unknown evidence that
needs attention. Pause and the requested tick cap also stop the call.

This selects one exact retained definition: its success and failure conditions,
entity generations, deadline, cadence and required stability never change.
`plan_digest`, `watch_id` and `watch_ids` are mutually exclusive. Omitting all
three preserves ordinary duration-based waiting; selecting a plan preserves
[original-project waiting](LAB_SELECTED_PLAN_WAIT.md). No new tool is added.
Direct Rust callers can use
`fortress_wait_for_watch(session_id, max_game_ticks, watch_id)`; existing
two-argument and selected-plan Rust entry points remain available.

A watch can be selected before any action has been committed. A zero-tick or
omitted `max_game_ticks` evaluates only the current world and needs no clock
grant. A positive cap requires current whole-fortress Observe, Query and
ControlClock grants; when initially unpaused, all three must cover the entire
requested horizon before any advancement starts. The same authority and shared
wall-time deadline are checked throughout the call. A paused world stays paused.
The handle carries no authority, and a missing, foreign or released handle
never falls back to another watch.

Other retained committed work progresses during the same world time, including
authorized ready successors. The selected watch does not dispatch a new plan,
cancel work, replace a pending review, change the latest action identity, or
sample any other watch. An internal action poll or successor dispatch can
produce additional observation sequences within one progression interval.
Such gaps remain visible and reset positive stability; the waiter never
renumbers cursors or invents the missing observations.

Every successful selected response includes `selected_watch_wait` with schema
`dfmcp.selected-watch-wait/1`:

| Field | Meaning |
|---|---|
| `watch_id` / `record` | Exact owned handle and complete original definition, retained summary and bounded evidence. |
| `stop_reason` | `satisfied`, `failed`, `expired`, `invalidated`, `cancelled`, `blocked_unknown`, `paused`, or `tick_limit`. |
| `watch_satisfied` | The complete selected evaluation returned a retained satisfied outcome. Terminal evidence remains historical. |
| `evaluation_complete` | Whether this call published the complete selected result at its reported frontier. |
| `sampling_rounds` / `samples_added` | Bounded inspection rounds and actual new watch samples; repeated identical anchors add no samples. |
| `source_work` | Accumulated canonical-source inspection cost, with per-source size limits and call-wide work bounds. |

`wait_progress` reports the requested, actually advanced and remaining ticks,
action polls and successor dispatches. The Agent Turn includes the current
complete anchor and all retained baseline, watch, pending plan and game-work
catalogs. A terminal watch reuses its historical outcome; inspect
`record.evaluation_current` and `record.last_evaluated_anchor` when the current
world has moved. Satisfaction does not prove an action caused the condition,
that another project completed, or that truth held between observations.

Before any game effect, the initial selected result and complete Agent Turn
must fit the response budget. Each later world slice is persisted before the
watch's candidate update is published. The complete result must again fit
before that watch root changes, and the facade returns the accepted packet
unchanged. A later publication or authorization refusal leaves previously
published game time intact and stops further advancement; authorized error
details distinguish the actual world frontier from the last accepted watch
sample. An incomplete error marks that retained sample as not current and
never claims selected satisfaction.

As with selected-plan waiting, `max_entities` and `max_bytes` bound each
complete observed source. One shared wall-time allowance and at most 4,097
watch inspections (the initial inspection plus up to 4,096 foreground slices)
bound the whole call. Source cost accumulates in the response; the full
response independently respects `max_bytes` and `max_output_tokens`. Raw
watch handles are process-local, so replay bundles containing selected-watch
waits are explicitly nonreplayable.

## Lifecycle and batching

| Query kind | Arguments | Behavior |
|---|---|---|
| `watch` | `key`, `condition`, `deadline_tick`; optional `label`, `failure_condition`, `poll_interval_ticks`, `stable_observations` | Register one immutable definition and evaluate once. |
| `poll_watch` | `watch` | Evaluate the current world and return complete definition and bounded evidence. |
| `poll_watches` | Optional `watches` | Evaluate one bounded selected set against one current world and publish every update together. |
| `watches` | None | List all retained active and terminal summaries without evaluating them. |
| `cancel_watch` | `watch` | Make an unfinished watch terminal without changing game work. |
| `release_watch` | `watch` | Release a terminal retained record; active records must be cancelled first. |

For a batch, omit `watches` or use `null` to select all retained watches. An
explicit array must contain one to eight unique owned handles. The result
reports `sampled`, `advanced`, `terminal`, `remaining`, `all_terminal`, and
`all_satisfied`, with `atomic_watch_publication: true`. A refused batch changes
none of its selected watch states. An empty session batch does not claim all
conditions satisfied. Terminal watches retain their historical evidence.

The wrapper accepts an optional complete `expected_anchor` inside `request`.
It must equal the current session anchor before a lifecycle operation proceeds.
Historical `at`, native refresh, `await_watch`, and `await_watches` are outside
this query wrapper. Watch-aware `fortress_wait` is an explicit foreground call;
no detached poller or automatic sampling outside that call is started.

## Supported conditions and exact source policy

The closed condition grammar supports exact generation-bound `field`,
`entity_count`, `paused`, `tick_at_least`, nonempty `all` and `any`, and `not`.
Comparisons are `eq`, `ne`, `lt`, `le`, `gt`, and `ge`. Scalar literals support
null, Boolean, signed or unsigned 64-bit integer, text, and fixed point with an
exact matching scale. Type mismatches remain unknown.

An `entity_count` names `scope: "observed_projection"` and one of `unit`, `job`,
`building`, `item`, `tile_feature`, or `announcement`. Its membership predicate
supports `always`, typed `field`, and Boolean composition. Membership is
reevaluated at every sample. Unknown membership contributes a sound count
interval; a comparison succeeds only if the whole interval satisfies it. This
does not claim a complete native-world count or infer usable stock from the
number of item records.

For example, this counts units whose laboratory mining setting is known enabled:

```json
{
  "op": "entity_count",
  "scope": "observed_projection",
  "kind": "unit",
  "predicate": {
    "op": "field",
    "field": "labor.MINING",
    "comparison": "eq",
    "value": {"type": "bool", "value": true}
  },
  "comparison": "ge",
  "value": 1
}
```

The owner selects an explicit laboratory evidence policy. A field can establish
truth only through the existing `laboratory_fact_value` qualification: a known,
consistent value from `dfmcp.lab-scenario/1` or `dfmcp.reference-effects/1`, the
registered zero-digest laboratory producer convention, and an observation tick
no later than the current world. Replayed, asserted, arbitrary derived, future,
redacted, omitted, stale, conflicting and missing facts remain unknown.

Native `DfhackField` labels do not become laboratory evidence. Existing native
watch entry points retain their independent nonzero-digest, current-tick source
policy. The request cannot select or broaden a trust policy. Native stack-unit,
relationship, terrain, furniture and shelter predicates are not accepted by
this laboratory grammar.

## Stability, unknowns and recovery

Registration counts as the first due sample. Repeating the identical anchor
does not change evidence or sample counts. Changed observations at the same
game tick can expose failure or uncertainty but cannot add positive stability.
Positive samples respect the chosen cadence. False or unknown success, unknown
failure, and skipped observation sequences reset the unfinished streak. A known
failure condition takes precedence over success.

When `evaluation_current` is true, another poll of that same world cannot add
stability. If separately authorized and the world is unpaused, use
`fortress_wait` with this `watch_id` to sample at the retained cadence. Manual
short waits followed by polling remain available. An ordinary wait without a
watch selector can publish several intermediate observation sequences; a
subsequent poll reports that gap and resets its streak. Clock permission and
successful elapsed ticks alone never establish a watch sample.

Statuses are `waiting`, `candidate`, `blocked_unknown`, `satisfied`, `failed`,
`expired`, `invalidated`, and `cancelled`. The last five are terminal. An exact
entity generation change invalidates a watch even within a Boolean expression
whose other branch is decisive. A missing entity remains unknown. A later poll
invalidates an unfinished watch after an incompatible fortress, restore epoch,
clock, sequence or same-cursor identity change.

A due sample at the exact deadline may establish completion; a later sample
cannot establish late success. Terminal outcomes remain historical and are not
silently reopened when the world changes. Sampled satisfaction is evidence of
the requested predicate observations, not causal attribution, mutation
permission, an action receipt, or proof that truth held between observations.

## Complete retained-work publication

Successful temporal packets preserve existing pending plans, action state and
game obligations. They also publish three separate bounded arrays:

- `agent_turn.active_work.query_baselines`: all retained baseline descriptors.
- `agent_turn.active_work.query_watches`: all retained watch summaries, including
  terminal records that can be released.
- `agent_turn.active_work.condition_watches`: the exact nonterminal watch set.

Watch descriptors supply laboratory `mode` arguments for the next poll or
terminal release; unfinished watches also supply a cancellation step. When
current Observe, Query and ControlClock grants cover a bounded proposed
interval, an unfinished descriptor additionally offers `wait_step` with its
exact `watch_id`. This step rechecks authority when used and never unpauses.
Ordinary observation can reconstruct these handles from session-owned stores
without resampling or relying on a presentation cache.

The current session's Query authority, cancellation state, source budgets and
deadline remain binding. Full source acquisition budgets stay separate from
response budgets. The complete result, all retained catalogs, current game work
and Agent Turn must fit before any watch or baseline state changes. A refused,
clipped or inconsistent publication cannot register, sample, cancel or release
retained work. Selected watch batches use the existing atomic candidate-store
publication boundary.

There are at most eight retained watches per session and 128 per engine
instance. Success and failure share 64 predicate nodes and depth eight; cadence
is 1–1,000,000 ticks and stability is 1–64 observations. Explicit release reclaims
retention capacity. Records are process-local, including when the laboratory
world itself uses durable storage. Session disposal reclaims that session's
private allocations. A transcript containing process-local watch identities
cannot recreate those handles in another session. Laboratory handles include a
private process incarnation, so restarting the session and serial counters does
not let an old handle name a newly registered watch. This scope is not an
authority token; native durable-watch identities retain their existing format.

## Inspect a condition before retaining it

Use `condition_evaluation` under the stateless `structured` wrapper with the
same condition grammar and evidence policy:

```json
{
  "mode": "structured",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "condition_evaluation",
      "condition": {
        "op": "field",
        "entity_id": "5001",
        "generation": 1,
        "field": "stock.drink",
        "comparison": "ge",
        "value": {"type": "u64", "value": 60}
      }
    }
  }
}
```

The query accepts an optional `failure_condition`. The outer wrapper can name
an exact retained `at` state hash; current grants authorize the historical read.
The result reports condition/failure truth, status, bounded evidence and digests
without registering or sampling a watch. `stability_evaluated`,
`deadline_evaluated`, and `watch_completion_proven` remain false. A true preflight
does not substitute for subsequent stable observations or current action
preconditions.

The decoded wrappers are described by
[`lab_query_watches.schema.json`](../schemas/lab_query_watches.schema.json) and
[`lab_condition_evaluation.schema.json`](../schemas/lab_condition_evaluation.schema.json).
The additive wait arguments are described by
[`lab_wait.schema.json`](../schemas/lab_wait.schema.json), and the selected-watch
contract by
[`laboratory_selected_watch_wait_v1.json`](../architecture/laboratory_selected_watch_wait_v1.json).
The exact-set foreground variant has its own
[`laboratory_selected_watch_set_wait_v1.json`](../architecture/laboratory_selected_watch_set_wait_v1.json)
contract and [usage guide](LAB_SELECTED_WATCH_SET_WAIT.md).
Rust tests cover the real reference source policy, lifecycle, atomic batches,
complete publication, existing game work, source/output budgets and public
labor/clock flows. Their execution and qualification status is recorded in the
repository implementation status; source registration alone is not runtime or
native-game evidence.

## Replay a sampled condition over retained observations

Use the separate read-only `mode: "condition_replay"` query to run this same
condition and temporal state machine over an exact interval of already retained
laboratory versions. It can reveal a failure or stability reset hidden by an
endpoint comparison without registering or sampling a live watch. Collected
interior versions refuse, and original cursor gaps reset positive stability;
the result never claims continuous history. The request, evidence fields and
full-interval budget contract are in
[`LAB_CONDITION_REPLAY.md`](LAB_CONDITION_REPLAY.md).
