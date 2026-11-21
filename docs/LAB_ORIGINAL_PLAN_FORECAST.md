# Joint forecasts of committed laboratory originals

The existing `fortress.query` tool can forecast one to eight **already committed,
current-session original plans together**. This answers a practical question
that independent plan comparisons cannot answer: will these originals still
finish when they share the same finite workforce, materials and consumption?

The query uses one discarded copy of the current laboratory adapter, including
its existing action receipts and obligation runtimes. It never recommits an
original or prepares another project stage. The default laboratory remains
separate from native Dwarf Fortress and DFHack execution.

## Request

Pass this closed JSON document as the tool's `mode` string:

```json
{
  "mode": "forecast_originals",
  "plan_digests": [
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  ],
  "horizon_ticks": 2000
}
```

Replace the illustrative digests with exact originals returned by this session's
commits. `horizon_ticks` is a **simulated** allowance from the current source tick;
it must fit the session's game-tick budget. Zero inspects current outcomes without
polling or dispatching even a ready deferred action.

Both fields are required. The envelope rejects unknown fields, duplicate JSON
keys, malformed lowercase digests, repeated originals and more than eight
members. Historical `at` selectors, candidate requests and arbitrary proposed
action arrays are not accepted. Requested aliases retain their requested identity
while resolving to the actual sealed original. Two aliases for the same original
cannot be counted twice. Results are sorted by actual original digest.

## Shared progression

The forecast starts from the exact current world. Every current-session open
action participates in the same original commit order, including actions of
unselected originals. Their deferred successors can compete for workers and
stock, change labor or pause the predicted world. Already-dispatched work from
other owners also evolves in the shared snapshot. Their deferred actions are not
polled because this session does not own their progression; the response states
this assumption explicitly.

An initial zero-time polling round can release already-ready original
dependencies. Subsequent slices follow the laboratory foreground wait policy:
a nominal 100-tick interval clipped to remaining horizon, each active original
obligation's cadence and deadline, and each selected exact reserve endpoint.
A complete round evaluates a deferred pause followed by an authorized resume
before deciding whether the predicted world is paused. A paused source stays
paused unless such an already-committed action changes it.

The copied obligation runtime retains its previous eligible observations and
stability interval. No deadline, identity, idempotency key, goal or obligation is
renewed. This models the existing foreground wait cadence; it does not search
for an optimal scheduling or observation policy. Fully terminal and physically
quiet actions leave the private polling set while their original receipts remain
available for outcome inspection.

## Read the result

A successful response has schema `dfmcp.lab-original-forecast/1` and a
`prediction` object with `epistemic_state: "predicted"`. Its top-level `anchor`
and the Agent Turn anchor are the actual current source. A separate predicted
frontier tick and hash describe the discarded model.

Each original separately reports:

- current predicted full-goal truth and sealed stage truth;
- its original action states and exact action/step identities;
- current physical work and whether it is provably quiet;
- the original reserve tick and its first exact endpoint truth, where applicable;
- a predicted outcome such as `complete`, `pending`, `reserve_pending`,
  `needs_continuation`, `failed`, `goal_unmet`, `work_unknown` or
  `reserve_unobserved`.

Group `all_complete` requires every selected original's full goal, Verified
original action proofs, physically quiet original work and required exact reserve
endpoint to be satisfied **at the same predicted frontier**. Earlier completion
is not latched: consumption or competing work can invalidate a goal while another
member is still progressing.

A finished resource-acquisition stage does not complete the original furnished
room or output project. The result reports `needs_continuation` when the stage
is proved and quiet but the full original goal is still false. The query does not
compile, review or commit that continuation.

The group stops when all members complete, any member needs attention, the
horizon is reached, or the predicted world remains paused. Failed proof does not
silently stop its physical effects; those effects remain visible in the
original's physical-work result.

## Exact future reserves

Different originals can have different absolute reserve ticks. The model
inspects every such frontier exactly. A retained original endpoint remains
immutable, including False or Unknown. If an earlier endpoint was never retained,
a later stock count cannot reconstruct it.

A newly predicted endpoint exists only inside this result. It is never installed
in the original goal book, snapshot retention or durable reserve journal. An
earlier true endpoint also does not establish current stock at a later group
frontier, nor continuous maintenance between observations.

## Authority, recovery and bounds

Current whole-fortress Observe, Query and Plan grants precede custody selection
and source admission. Actual live-clock authority is unnecessary for a discarded
simulation. An already-committed deferred effect still checks its real scoped
grant and expiry at the modeled frontier. For example, a deferred pause cannot
borrow a synthetic ControlClock grant. Modeled observation also respects Observe
expiry.

The query accepts only complete live original receipts owned by this session.
Pending plans, foreign originals, withdrawn or restore-abandoned pursuits,
missing original custody and recovery-only originals refuse before the model
fork. Existing original inspection and selected-wait APIs remain the way to
inspect recovered evidence; forecast does not recreate dispatch handles.

One request shares its wall deadline and negotiated source, action, tick and
output budgets. The complete selected and competing action union must fit the
action budget. Current and distinct retained reserve sources, original archives,
semantic requests and owned commit receipts participate in admission. Every
derived source is checked again against the complete retained-custody allowance.
Hard ceilings include:

| Domain | Bound |
| --- | ---: |
| Original request bytes | 2,048 |
| Selected originals | 8 |
| Original steps | Existing 64-step custody ceiling per original |
| Tracked action identities | 1,024 and current negotiated allowances |
| Private polling rounds | 4,096 |
| Action polls | 65,536 |
| Conservatively charged goal-predicate nodes | 4,000,000 |
| Canonical source and retained custody | Current negotiated bytes, capped at 16 MiB |

Repeated source inspections and simulated work are reported. Canonical snapshot
and custody bytes are not a measured heap-size count of the adapter's retained
plans, checkpoints and transcript. The full result and Agent Turn must fit
losslessly; generic row clipping cannot turn an incomplete forecast into a
successful response. Budget or authority failure discards the prediction and
returns an explicit error.

## What remains unchanged

This query advances zero actual game ticks. It does not alter canonical actions,
stock, workers, pending review, original goals, reserve history, snapshot
retention, durable progress, Tasks or observation acknowledgments. Ordinary
request/replay metadata still records the query call. The Agent Turn marks
continuity partial and predictions ineligible as mutation evidence.

To obtain actual laboratory progress and proof, explicitly call the existing
selected-plan `fortress.wait` with the original digests and a chosen tick
allowance. The forecast's next-step link identifies those originals; it does
not imply an automatic wait or authorize the predicted horizon.

## Implementation and verification

`lab_original_forecast.rs` handles closed intake, exact selected-original
custody, shared source accounting and read-only query projection.
`original_forecast_model.rs` drives the real cloned `MemoryAdapter`;
`Objective::forecast_truth_at` preserves original semantic room/output and
stock-ledger identity during pure private evaluation.

The model regression file exercises real finite worker and wood contention,
retained stability at a deadline, unselected ready deferred pause, pause/resume
rounds, zero horizon, authority, reserve invalidation by consumption, retained
false/unknown/missed endpoints and live-adapter isolation. The facade regression
file covers original/alias custody, pending review and history preservation,
source/output bounds, semantic project truth and nonpublication of predicted
reserve endpoints. Execution results and environment limitations are recorded
in the implementation status; source presence alone is not a test result.

Machine-readable contract:
[`architecture/laboratory_original_forecast_v1.json`](../architecture/laboratory_original_forecast_v1.json).
Related semantics: [foreground progress](LAB_FOREGROUND_PROGRESS.md),
[selected plan sets](LAB_SELECTED_PLAN_SET_WAIT.md) and
[independent plan comparisons](LAB_PLAN_COMPARISON.md).

