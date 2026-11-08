# Foreground laboratory condition watches

The default laboratory runtime can retain a bounded condition and evaluate it
against explicitly selected foreground observations. Agents can monitor stock
thresholds, labor settings, unit counts and building fields without remembering
raw before-state rows or treating a submitted action as proof of completion.
The same predicates can be inspected without registering a watch.

These operations use the existing `fortress.query` tool. Pass the JSON below as
the contents of its `mode` string, alongside the owning `session_id`. They share
the `temporal` wrapper with [query baselines](LAB_QUERY_BASELINES.md).

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
planning, commit and bounded `fortress.wait` operations with their ordinary
authority. Then explicitly evaluate the current published world:

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
this wrapper. No detached poller or automatic watch sampling is started.

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
stability. If separately authorized and the world is unpaused, use a short
explicit `fortress.wait` interval appropriate to `next_sample_tick` and the
deadline, then poll again. A large wait may publish several intermediate
observation sequences; the subsequent watch evaluation reports the resulting
gap and resets its streak. Clock permission and successful elapsed ticks alone
never establish a watch sample.

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
terminal release; unfinished watches also supply a cancellation step. Ordinary
observation can reconstruct these handles from session-owned stores without
resampling or relying on a presentation cache.

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
Rust tests cover the real reference source policy, lifecycle, atomic batches,
complete publication, existing game work, source/output budgets and public
labor/clock flows. Their execution and qualification status is recorded in the
repository implementation status; source registration alone is not runtime or
native-game evidence.
