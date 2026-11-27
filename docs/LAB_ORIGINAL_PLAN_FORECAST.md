# Joint forecasts of committed laboratory originals

The existing `fortress.query` tool can forecast one to eight **already committed,
original plans together**. The default scope selects current-session live
originals and exact recovered originals, including raw excavation, harvest and
work-order plans. Complete-project forecasting additionally accepts admitted
continuation chains, including an already-dispatched unfinished tip with an
authenticated observation monitor. This answers a practical question
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
`"current_stage"`; omitting the field preserves the original `/1` response for live-only selections.
Live complete-project responses use `dfmcp.lab-original-forecast/2`. A request
whose selected recovered steps are all historical uses
`dfmcp.lab-original-forecast/3`. If a selected recovered root or final committed
stage enters with unfinished observation work, the response uses
`dfmcp.lab-original-forecast/4`, including when its predicted outcome is complete.

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
Recovered stages entering with complete historical proof state
`custody: "recovered_verified_original"`; an unfinished recovered tip states
`custody: "recovered_observation_original"`. Both carry original step evidence
without action handles. Every retained row states
`recommitted_by_forecast: false`; predicted observations remain private.

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
this assumption explicitly. Both forecast scopes also clone every authenticated
unfinished recovered monitor held by this session, including unselected peers.
Those peers constrain the same cadence and consume shared observation, action,
source and predicate-work budgets. They receive no invented successors or
restored dispatch handles.

This also applies when a new live original is selected with the default
`current_stage` scope after durable reopening. Its verification schedule shares
the recovered peers' cadence and deadlines, just as real foreground waiting
does. Omitting those peers could predict an eligible stability sample too early.
An exact recovered original can itself be selected in `current_stage`.
Selected recovered steps count once in the action domain; their carried
monitors are excluded from the unselected-peer count. No project source or
production ledger is required for a raw action original.

An initial zero-time polling round can release already-ready original
dependencies. Subsequent slices follow the laboratory foreground wait policy:
a nominal 100-tick interval clipped to remaining horizon, each active original
obligation's next eligible observation and deadline, every unfinished carried
monitor's next eligible observation and deadline, and each selected exact
reserve endpoint. The next eligible tick comes from the retained monitor's
last cadence-eligible sample, not from the most recent read or query start.
A short earlier wait or another original's reserve endpoint therefore cannot
postpone the remaining stability observations. Carried observations
use the same pure transition and cadence ceiling as real foreground waiting.
The model also observes them after hypothetical commits at the same tick, so
new effects cannot escape current failure and identity checks. Repeated reads
at one tick add no stability samples.
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

A successful live-only current-stage response has schema
`dfmcp.lab-original-forecast/1`. Any current-stage selection containing a
recovered original uses `dfmcp.lab-original-forecast/5`, with
`forecast_scope: "current_stage"`; this includes entirely historical selections
and mixed live/recovered selections. Live complete-project responses use `/2`,
historical recovered-project responses use `/3`, and selected unfinished
recovered-project responses use `/4`.
All contain a
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
expiry; complete-project observations and selected recovered current-stage
observations also respect modeled Query expiry.

The default `current_stage` scope accepts complete live original receipts owned
by this session or exact recovered original custody. Pending plans, foreign
originals, withdrawn or restore-abandoned pursuits and missing original custody
refuse before the model fork. A present live receipt remains authoritative;
malformed live custody cannot fall back to recovery.

### Forecast original work after durable reopen

With the default `current_stage` scope, select the exact recovered original
digest. The query predicts only that already committed original: raw Mine,
harvest and work-order actions do not need to be wrapped in a production
blueprint. A committed continuation may also be inspected independently without
expanding its lineage. Every admitted step must be historical Verified and
currently physically quiet, or already dispatched with its exact active
recovered temporal monitor. Never-dispatched successors, failed or cancelled
custody, unavailable sources and unknown physical work remain refusals.

Ordinary durable recovery captures a compact, ledger-independent witness for
every exact original sealed source: its complete canonical record/byte counts,
structural byte/work bounds, original anchor, semantic digest and lossless
typed-plan object digest. Current-stage admission compares the entire typed
plan object, including variants that the older semantic encoding could alias.
Historical source costs are charged even when every selected step is complete
and no carried monitor remains. The query does not load the archive, hydrate
history or reconstruct old handles. Production/room goal semantics still enforce
their own original ledger requirements; complete-project admission additionally
retains its existing source and current-ledger validation.

Recovered current-stage responses use `/5`. Their original rows retain
`selection_origin: "durable_recovery"`, immutable proof anchors, null ActionIds
and separate private predicted proof. Source diagnostics include the archived
witnesses, recovered/Verified/unfinished step counts and fixed historical
structural charges. No future stage is prepared, and the next-step link still
names the exact selected originals for an explicit real wait.

With `forecast_scope: "complete_project"`, a reopened durable fortress can
forecast an exact ordinary production,
provisioned-room or connected-room root and its retained committed chain.
Every predecessor must already have complete Verified, physically quiet
original work. The final committed stage, or a root without descendants, may
still contain already-dispatched temporal work with its authentic recovered
observation monitor. An unfinished predecessor cannot be skipped to admit an
already-committed successor.

Select the exact ordinary root digest and `forecast_scope: "complete_project"`.
The existing recovered-original verifier checks the accepted lineage, unchanged
source and sealed plan, full anchored step-receipt map and current physical
effect identities. Each unfinished monitor must bind its deterministic original
proof ID, full normalized obligation and original registration tick. This
includes combined postconditions, terminal and failure predicates, deadline,
cadence and stability requirement. Its last observation must lie within the
accepted recovery frontier and the current source. A monitor from another
original fails even when both share the same recovery anchor.

The shared private model clones those existing monitors with their current
post-recovery stability progress. It does not reconstruct them from receipts,
reset a streak or deadline, restore an ActionId, recreate a job, or dispatch an
old effect. The same pure observation transition used by real waiting evaluates
the private world. It may predict verification or original-deadline failure;
neither result is published to the canonical owner. A predicted failure remains
separate from whether the physical work has stopped.

Admission rejects missing or interrupted monitors, never-dispatched work,
failed/cancelled/indeterminate custody and unknown physical work. Every
unfinished carried peer is independently admitted through its full original
custody, including unselected peers needed to reproduce the real wait cadence.
Unsupported non-temporal or unauthenticated peer work refuses this optional
forecast rather than supplying a replacement monitor. Zero horizon reads the
admitted states without observing or dispatching work.

The root selection discovers up to eight committed stages across a recovered
prefix and any subsequent live continuations. Every recovered stage retains an
exact sealed-source witness and the same accepted recovery frontier. A live
continuation belongs to this session and has a sealed source at or after that
frontier in the current epoch. Recovered stages cannot follow live stages.
Across accepted earlier epochs, parent source ticks and epochs cannot run
backward; within one epoch, cursor sequences cannot run backward and an equal
cursor retains the exact anchor. A present live receipt remains authoritative
even if malformed and cannot fall back to archived custody.

For example, an active wood harvest may be reopened and forecast through
verification into future BED production. A completed harvest followed by a
separately committed production stage may be reopened while production is still
running; its recovered final stage can predict completion without recommitting
either original. If completed stock is later consumed, separately reviewed
future stages can be modeled under the unchanged full goal.

Ordinary durable recovery already loads and verifies the original sealed
snapshot. It retains a compact forecast witness binding that complete source's
anchor, qualified stock-ledger identity and generation, full canonical byte and
record counts, and ordinary query structural byte/work bounds. Each recovered
descendant retains its own witness. Forecast admission charges the distinct
complete source union alongside current, original, peer and reserve custody.
The query does not read the store, hydrate history or reconstruct the old world.
Unavailable forecast qualification does not prevent ordinary goal recovery; it
refuses this optional forecast.

All recovered rows state `selection_origin: "durable_recovery"`,
`owns_original_dispatch: false` and `dispatch_handles_restored: false`.
Their steps preserve exact step IDs, original receipt/proof anchors and
unchanged deadlines, and `action_id` remains `null`.

| Step at admission | Result evidence |
| --- | --- |
| Already Verified and physically quiet | `action_state_origin: "verified_recovered_history"`; immutable original proof, with physical quiet rechecked at each predicted frontier |
| Already dispatched with an active recovered monitor | `action_state_origin: "cloned_recovered_observation_monitor"`; private `predicted_state`, `predicted_proof_anchor`, stability and failure details, with the original `proof_anchor` unchanged |

For active rows, `predicted_proof_is_canonical` and
`dispatch_handle_restored` are false. A private predicted proof cannot become
an original archived proof merely because its deadline and predicate succeed.
The stage's admission custody label remains `recovered_observation_original`
even if all of its private predicted steps finish.

All selected recovered steps contribute to the cumulative original-action
allowance. `recovered_original_steps` counts the root and descendants;
`verified_recovered_original_steps` and
`verified_recovered_continuation_steps` count only their historical Verified
steps at admission. `retained_continuation_actions` includes every retained
descendant step. The source report separately exposes
`recovered_observation_steps`. The model reports `tracked_recovered_steps`,
`competing_recovered_steps`, `recovered_observation_polls` and
`recovered_predicate_node_visits_bound` for privately observed work. Recovered
polls share the polling ceiling, and their predicates share the existing
conservative node-work bound.

New predicted successor plans still require current scoped effect and
checkpoint authority. They exist only in the discarded model and carry the
existing separate-review requirement. Original receipts, canonical stability,
goal history and accepted recovery lineage remain unchanged.

An already retained reserve endpoint must have its exact snapshot in bounded
session history before forecasting. Immediately after restart, an explicit
selected wait may be needed to load that previously published endpoint.
Forecasting refuses an unavailable snapshot and cannot replace its first True,
False or Unknown sample with current stock. A missed past endpoint still blocks
complete-project success. Each root and child retains independent reserve
custody, including during prediction of unfinished recovered work.

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
| Live action and recovered-observation polls combined | 65,536 |
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
Complete-project advice normally selects the latest committed stage of each root.
The `next_step.reserve_capture` object identifies the earliest outstanding
endpoint at or after the actual source tick, the ticks remaining, the number
of missing samples at that endpoint and the first batch of wait selectors.
A caller choosing to advance time can use this distance to limit its request
while the source anchor is current. Past missing samples are counted separately.

At the exact endpoint, the returned wait arguments select up to eight unsampled
originals with `max_game_ticks: 0`, including predecessor stages. Execute that
wait and refresh the forecast. If more than eight samples are due, the next
forecast selects the next outstanding batch. A retained false or unknown sample
is already recorded and is never selected for recapture. The forecast itself
does not advance time, capture evidence or grant clock authority.

## Implementation and verification

`lab_original_forecast.rs` handles closed intake, exact selected-original
custody, shared source accounting and read-only query projection.
`original_forecast_model.rs` drives the real cloned `MemoryAdapter`;
`Objective::forecast_truth_at` preserves original semantic room/output and
stock-ledger identity during pure private evaluation.
`original_project_lineage.rs` validates the bounded exact retained chain.
`objective_forecast_recovery.rs` retains forecast-specific sealed-source custody
during ordinary recovery, including ledger-independent raw-original witnesses;
`recovered_original_forecast.rs` separates immutable
historical census from private predicted monitor results. The shared pure
`recovered_step_progress.rs` implements real and predicted recovered observation
and cadence without creating adapter handles.
`original_project_forecast.rs` preserves already committed descendants and
reuses the existing complete-project compilers for explicitly hypothetical
successor stages, separating their receipts from live originals.

The model regression file exercises real finite worker and wood contention,
retained stability at a deadline, interrupted waits and off-cadence reserve
frontiers, unselected ready deferred pause, pause/resume
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
epochs, malformed descendant live-custody precedence, and unfinished recovered
tips whose predicted completion matches real selected waiting. Active-recovery
regressions cover exact monitor binding, retained eligible stability, original
deadlines, same-tick observations, peer cadence, independent reserve custody,
modeled authority expiry and canonical-world isolation.
Durable facade regression sources cover actual reopen and compaction, source
and output admission, unfinished work, malformed live-custody precedence,
replacement ledgers and preservation of pending review, world and history.
Execution results and environment limitations are recorded
in the implementation status; source presence alone is not a test result.

New current-stage regression sources cover a raw Mine source with terrain and
no stock ledger, durable reopening and comparison with real selected waiting,
mixed live/recovered finite orders, retained cadence/stability, original deadline
failure, current and modeled-expiry authority, exact typed aliases, missing
historical witnesses, complete source/action/output limits and memory/file read
purity. They are authored sources; compilation and execution remain unavailable
in the disconnected implementation environment.

Machine-readable contract:
[`architecture/laboratory_original_forecast_v1.json`](../architecture/laboratory_original_forecast_v1.json).
Related semantics: [foreground progress](LAB_FOREGROUND_PROGRESS.md),
[selected plan sets](LAB_SELECTED_PLAN_SET_WAIT.md) and
[independent plan comparisons](LAB_PLAN_COMPARISON.md).

