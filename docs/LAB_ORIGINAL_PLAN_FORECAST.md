# Joint forecasts of committed laboratory originals

The existing `fortress.query` tool can forecast one to eight **already committed,
original plans together**. The default scope selects current-session live
originals; complete-project forecasting also accepts recovered, verified quiet
roots and their admitted continuation chains as described below. This answers a practical question
that independent plan comparisons cannot answer: will these originals still
finish when they share the same finite workforce, materials and consumption?

The query uses one discarded copy of the current laboratory adapter, including
its existing action receipts and obligation runtimes. The default scope never
recommits an original or prepares another project stage. An explicit
complete-project scope includes an admitted chain of already committed
continuations and can model separately reviewed future stages on that same
discarded copy. The default laboratory remains
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

## Complete projects under shared resources

Add `"forecast_scope": "complete_project"` to predict subsequent stages of an
ordinary production, provisioned-room or connected-room root. The default is
`"current_stage"`; omitting the field preserves the original `/1` response.
Live complete-project responses use `dfmcp.lab-original-forecast/2`. A request
containing an admitted recovered root uses `dfmcp.lab-original-forecast/3`.

```json
{
  "mode": "forecast_originals",
  "plan_digests": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
  "horizon_ticks": 4000,
  "forecast_scope": "complete_project"
}
```

For example, a committed BED stock goal may currently own only wood harvesting.
The default forecast can report Verified, quiet harvest actions while the
original output-stock seal remains False and the outcome is `goal_unmet`.
Complete-project prediction waits for Verified, physically quiet initial work,
then recompiles the unchanged BED goal from the wood actually present on the
shared fork. A room similarly includes residual excavation, workshop/staffing
setup, furniture production and exact installed-item custody; a connected room
also retains its entrance, routing domain and movement model.

All selected projects and existing competing work use one finite world. Ready
successors are considered in ascending actual original-digest order. One
successor commits only inside that private model, then every goal is evaluated
again before another successor is considered. This declared tie-break policy
is deterministic; it is not an optimal project scheduler. Unselected originals
receive no invented successors. The same model can therefore expose resource
shortages or earlier goal invalidation caused by later work.

A qualified active producer of the same output can block a successor's quota
review. The model waits only when the compiler identifies the exact canonical
producer and a tracked action still proves that producer's physical work is
Active. It tries other ready originals before advancing the ordinary bounded
polling frontier, then recompiles from the stock actually observed. An unrelated
long-running order does not impose a global stage barrier. Deferred reviews
consume no new stage or action allowance and are reported in
`forecast_work.stage_reviews_deferred`. A waiting original may include
`next_stage_wait` describing the predicted producer. Its possible future output
is never counted as current stock. Foreign or untracked producers, unknown work,
missing authority and all other compiler refusals remain explicit errors.

Current stock minima do not establish a future reserve. Once a stage is quiet,
the source's unchanged reserve compiler checks whether remaining future demand
requires another stage, even when the current minimum is True. Every selected
exact reserve endpoint is sampled before any hypothetical successor changes the
world at that same tick. A False, Unknown or missed endpoint cannot be rescued
by later production.

Select the ordinary root even after separately committing continuations. The
query discovers its complete retained chain by exact parent and root identities;
request order and goal-book order cannot reorder its stages. Up to eight
already committed stages, including the root, are admitted per project. Every
predecessor must already have Verified, physically quiet actions; the final
committed stage can still have prepared or active work. Its real receipts,
remaining work, prior stability observations and absolute deadlines are copied
into the shared model. No committed stage is prepared or committed again.

Each live stage in a continuation chain must belong to this session and retain the unchanged typed
production, room or connected-room request, its exact sealed plan and source
snapshot, and complete action custody. The root's qualified stock-ledger
identity and generation must remain the same in every stage source and the
current world; each stage's own source-bound ledger validates its own plan.
Parent sources must stay within the same fortress and restore epoch and cannot
run backward in tick or cursor sequence. Gaps, cycles, branches, duplicate
identities, changed requests, withdrawn or restore-abandoned pursuits, restored
dispatch handles, missing sources, failed or cancelled stages and unknown physical work
refuse before the fork. No source is fetched from disk or reconstructed.
The verified recovery path below permits historical stages from accepted earlier
epochs; it does not relax these checks for live stages.

Selecting a continuation wrapper directly remains unsupported in
`complete_project`; select its ordinary root instead. Raw action/pause plans and
stand-alone resource acquisition requests also refuse. `current_stage` retains
its existing behavior for inspecting one exact original, including a committed
continuation, without expanding its lineage.

Each original keeps its initial `steps`, `predicted_actions_complete` and
`predicted_stage_truth`. `retained_stages` lists actual committed descendants in
parent order, including their real plan/action identities, original source
anchors, independent reserve samples, unchanged obligation deadlines and
predicted outcomes. Live rows state `custody: "already_committed_original"`.
Recovered rows state `custody: "recovered_verified_original"` and carry historical
step evidence without action handles. Every retained row states
`recommitted_by_forecast: false`; none reports a new observation.

Additional `future_stages` contain clearly labeled model plan/action
identities, stage names, required capabilities/risk,
checkpoint burden, stage deadlines/cadence and current action/work
outcomes. Stage indices start at two for the first retained descendant, and
hypothetical indices continue after the last committed stage.
`predicted_committed_actions_complete` covers the root and its retained
descendants; `predicted_project_actions_complete` includes those actions and
modeled future work. `predicted_original_physical_quiescent` preserves initial
work separately.
Overall completion requires all selected full goals, every required exact
reserve endpoint, Verified initial, retained and future actions, and quiet physical work
at the same shared frontier.

There are at most three hypothetical successors per root. They share the
original request's wall deadline, common simulation horizon, action/record
allowance, canonical source bytes, retained plan-archive bytes, predicate-work
meter and final lossless output reservation. Original action obligations are
never renewed. A future stage has its own compiler-derived obligations because
it represents a separate future review; those obligations cannot extend the
query's horizon or replace an original reserve endpoint.

Each future stage checks Observe, Query, Plan, every scoped effect grant and
checkpoint authority at its predicted tick **before** consuming cumulative
stage/action allowances or preparing on the private adapter. Missing/expired
authority, unavailable evidence, insufficient resources or exhausted bounds
refuse the complete prediction without publishing partial success. Zero horizon
performs no stage preparation. Pause remains authoritative and no clock grant
or unpause is invented.

The model assumes the agent reviews each eligible next stage without additional
delay. `future_stage_plans_are_executable` is false. Actual progress still needs
an explicit wait, fresh current evidence and a separate ordinary `continue_goal`
review/commit. The query changes no live pending plan, original receipt, goal
history, reserve sample, Task, journal or actual game tick.

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

A successful current-stage response has schema `dfmcp.lab-original-forecast/1`;
live complete-project responses use `/2`, and complete-project responses with
recovered originals use `/3`. All contain a
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
room or output project. A room acquisition stage can have a separate material
predicate and report `needs_continuation` when that stage is proved and quiet
but the full room goal is still False. A production acquisition stage retains
the original output-stock seal: when that predicate is False, the stable
current-stage outcome is `goal_unmet` even if every harvest action is Verified.
The default `current_stage` query does not compile, review or commit another
stage; `complete_project` models successors under the explicit contract above.

The group stops when all members complete, any member needs attention, the
horizon is reached, or the predicted world remains paused. Failed proof does not
silently stop its physical effects; those effects remain visible in the
original's physical-work result.

## Exact future reserves

Different originals can have different absolute reserve ticks. The model
inspects every such frontier exactly. A retained original endpoint remains
immutable, including False or Unknown. If an earlier endpoint was never retained,
a later stock count cannot reconstruct it.

A root and its retained continuations each keep their own endpoint custody,
even though their unchanged request has the same absolute reserve tick. A
retained sample from one pursuit cannot fill a missing sample in another.
Every admitted pursuit lacking a sample is evaluated at that exact shared
frontier before any hypothetical successor changes the world. Once the tick
has passed, a missing child or root sample yields `reserve_unobserved` and
prevents complete-project success or invented repair stages.

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
expiry; complete-project observations also respect modeled Query expiry.

The default `current_stage` scope accepts only complete live original receipts
owned by this session. Pending plans, foreign originals, withdrawn or
restore-abandoned pursuits and missing original custody refuse before the model
fork. Complete-project forecasting has the following bounded recovery path.

### Forecast completed stages after durable reopen

A reopened durable fortress can forecast the next stages of an exact ordinary
production, provisioned-room or connected-room root whose **every original step
is already Verified and whose original physical work is provably quiet**.
For example, a resource-aware BED goal whose wood harvest finished before
restart can forecast subsequent furniture production alongside other selected
projects, using the same finite current world.

Select the exact original digest and `forecast_scope: "complete_project"`.
The accepted recovery lineage, unchanged source and reconstructed plan, full
anchored step-receipt map and current physical effect identities pass the
existing recovered-original verifier. A dispatched step subsequently verified
by its real recovered monitor is eligible. The forecast never creates such
verification itself. Active, unknown, failed, cancelled and never-dispatched
original steps refuse; continue observing or explicitly review the appropriate
recovery action first.

The same root selection discovers up to eight committed stages across a
verified recovered prefix and any subsequent live continuations. Every recovered
stage must have complete, already Verified and physically quiet historical step
custody, an exact sealed-source witness, and the same accepted recovery frontier.
A live continuation must belong to the current session and have a sealed source
at or after that frontier in the current epoch. Recovered stages cannot follow
live stages. Across accepted earlier epochs, parent source ticks and epochs
cannot run backward; within one epoch, cursor sequences also cannot run backward
and an equal cursor must retain the exact anchor. A present live receipt remains
authoritative even if malformed, so it cannot fall back to archived custody.

For example, a completed harvest followed by a committed production stage can
be reopened and forecast as one original project. A newly reviewed production
continuation after reopening retains its current live jobs and deadlines while
the predecessor remains historical. After another reopen, that continuation is
eligible as historical work only once its real recovered monitor has verified
every dispatched step and the original physical work is quiet. A completed
recovered chain whose stock is later consumed can predict additional separately
reviewed stages without recommitting any historical stage.

Ordinary durable recovery already loads and verifies the original sealed
snapshot. It retains a compact forecast witness binding that complete source's
anchor, qualified stock-ledger identity and generation, full canonical byte and
record counts, and the ordinary query inspector's structural byte/work bounds.
Each recovered descendant retains its own witness. Forecast admission charges
the distinct complete source union alongside current, original and reserve
sources. It does not read the store, hydrate history or reconstruct the old world
during the query. Unavailable forecast-specific qualification does not prevent
ordinary goal recovery; it only refuses this optional forecast.

Recovered rows state `selection_origin: "durable_recovery"`,
`owns_original_dispatch: false` and `dispatch_handles_restored: false`.
Their original steps retain exact step IDs, historical proof and receipt
anchors and original obligation deadlines. `action_id` is `null`, with
`action_state_origin: "verified_recovered_history"`; no invented handle or
fresh stability observation stands in for history. The model checks original
physical quiet again at every predicted frontier. These historical steps count
toward original action completion and the same cumulative action allowance as
live and hypothetical steps. `forecast_work.verified_recovered_original_steps`
counts the recovered root and descendants;
`forecast_work.verified_recovered_continuation_steps` identifies the descendant
portion. `retained_continuation_actions` includes historical descendant steps
even though they have no live action IDs.

New predicted successor plans still require current scoped effect and
checkpoint authority. They exist only in the discarded model and carry the
existing separate-review requirement. Original deadlines, receipts, stability,
goal history and accepted recovery lineage remain unchanged.

An already retained reserve endpoint must have its exact snapshot in the
session's bounded history before forecasting. Immediately after restart, an
explicit selected wait may be needed to load that previously published
endpoint. Forecasting refuses an unavailable snapshot and does not replace its
first True, False or Unknown sample with current stock. A missed past endpoint
still blocks complete-project success. Zero horizon performs no successor
preparation, including for a recovered root.

One request shares its wall deadline and negotiated source, action, tick and
output budgets. The complete selected and competing action union must fit the
action budget. Current and distinct retained reserve sources, authenticated
recovered sealed-source witnesses, original archives,
semantic requests and owned commit receipts participate in admission. Every
derived source is checked again against the complete retained-custody allowance.
Hard ceilings include:

| Domain | Bound |
| --- | ---: |
| Original request bytes | 2,048 |
| Selected originals | 8 |
| Already committed stages (complete-project scope) | 8 per selected root, including that root |
| Original steps | Existing 64-step custody ceiling per original |
| Tracked action identities | 1,024 and current negotiated allowances |
| Hypothetical successors (complete-project scope) | 3 per selected root |
| Private polling rounds | 4,096 |
| Action polls | 65,536 |
| Conservatively charged goal-predicate nodes | 4,000,000 |
| Canonical source and retained custody | Current negotiated bytes, capped at 16 MiB |

Complete-project evaluation charges an additional conservative 130 semantic
predicate nodes per original against the shared 4,000,000-node ceiling,
including its full-goal and remaining-reserve inspection. Repeated source
inspections, deferred stage reviews and simulated work are reported. Canonical snapshot
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
allowance. The default forecast's next-step link identifies those originals.
Complete-project advice identifies the latest already committed stage of each
selected root, so waiting can progress an active continuation. It also lists
each root or descendant with reserve endpoint custody. Waiting for a tip does
not retain its predecessors' samples: at the original absolute tick, explicitly
capture every required pursuit with zero-tick selected waits, in groups of at
most eight, before advancing time again. No advice implies an automatic wait
or authorizes the predicted horizon.

## Implementation and verification

`lab_original_forecast.rs` handles closed intake, exact selected-original
custody, shared source accounting and read-only query projection.
`original_forecast_model.rs` drives the real cloned `MemoryAdapter`;
`Objective::forecast_truth_at` preserves original semantic room/output and
stock-ledger identity during pure private evaluation.
`original_project_lineage.rs` validates the bounded exact retained chain.
`objective_forecast_recovery.rs` retains forecast-specific sealed-source custody
during ordinary recovery; `recovered_original_forecast.rs` carries complete
verified historical steps without creating adapter handles.
`original_project_forecast.rs` preserves already committed descendants and
reuses the existing complete-project compilers for explicitly hypothetical
successor stages, separating their receipts from live originals.

The model regression file exercises real finite worker and wood contention,
retained stability at a deadline, unselected ready deferred pause, pause/resume
rounds, zero horizon, authority, reserve invalidation by consumption, retained
false/unknown/missed endpoints and live-adapter isolation. The facade regression
file covers original/alias custody, pending review and history preservation,
source/output bounds, semantic project truth and nonpublication of predicted
reserve endpoints. Retained-chain regressions cover an active child without
recommit, complete action-union limits, independent historical endpoints, and
real furniture allocation that consumes a previously produced bed and requires
further private harvest/production stages. Lineage regressions exercise missing
parents, cycles, branches, owner and restore-epoch mismatches, changed requests,
lost handles and the committed-stage bound. Recovered-model regressions cover
residual projects without old handles, cumulative action limits, immutable
negative/missing reserve evidence, current effect authority and reset refusal.
Recovered-chain regression sources cover complete archived chains, active live
tips after recovery, shared action budgets, independent root and child reserve
samples, rejected cross-reset custody, and further stages after real furniture
allocation. Durable chain sources cover repeated reopen and compaction, mixed
epochs, malformed descendant live-custody precedence, and refusal of unfinished
recovered descendants until real monitoring verifies their work.
Durable facade regression sources cover actual reopen and compaction, source
and output admission, unfinished work, malformed live-custody precedence,
replacement ledgers and preservation of pending review, world and history.
Execution results and environment limitations are recorded
in the implementation status; source presence alone is not a test result.

Machine-readable contract:
[`architecture/laboratory_original_forecast_v1.json`](../architecture/laboratory_original_forecast_v1.json).
Related semantics: [foreground progress](LAB_FOREGROUND_PROGRESS.md),
[selected plan sets](LAB_SELECTED_PLAN_SET_WAIT.md) and
[independent plan comparisons](LAB_PLAN_COMPARISON.md).

