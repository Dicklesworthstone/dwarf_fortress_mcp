# Fund future production reserves from finite resources

The laboratory can gather missing WOOD or PLANT for a production request that
retains an absolute future reserve tick. Pass this object as the existing
`fortress.plan` **blueprint JSON string**:

```json
{
  "template": "production",
  "quotas": [
    {"item": "DRINK", "minimum": 40},
    {"item": "FOOD", "minimum": 60}
  ],
  "acquire_resources": {"assign_labor": false},
  "reserve_until_tick": 12001
}
```

Use `starter_fortress_resources` for the finite harvest and provisions model.
The request selects `resource_reserve_v1`. Its original stock minima, absolute
tick and independent labor/setup permissions remain fixed throughout the
workflow. A caller may explicitly name the same compiler; archived sources
must retain it. This is deterministic laboratory functionality, not native
gameplay or live compatibility evidence.

## Gather, observe, then review production

The compiler proposes one executable stage at its current observed anchor:

| Stage | Proposed actions | Next step |
| --- | --- | --- |
| `acquire_resources` | Finite harvest and explicitly permitted harvesting labor. | Commit, observe actual yield and physical quiescence, then request `continue_goal`. |
| `production` | Strictly funded work orders and explicitly permitted setup. | Commit and observe original action proof and output stocks. |

It never inserts an unfunded order or builds with forecast wood. Each stage
needs a newly reviewed plan and explicit commit. Once the original stage is
physically quiet, submit:

```json
{
  "template": "continue_goal",
  "plan_digest": "<retained original stage digest>"
}
```

The continuation retains the same `reserve_until_tick`; it cannot turn that
tick into a new duration. Replanning at tick 101 for a tick-12001 reserve
budgets the remaining 11,900 ticks. An expired original horizon refuses.
If the requested tick is so near that gathering finishes after it, continuation
also refuses; the original horizon cannot be renewed implicitly.

The example deliberately begins with currently satisfied FOOD/DRINK minima.
Those observed stocks do not fund the same minima through the future tick
under the reference consumption model. Initial planning may therefore propose
more work. When gathering finishes and instantaneous minima still hold, `/5`
can continue into production if the remaining explicit reserve still has a
source-qualified deficit. If the complete remaining reserve is already funded,
continuation proposes no new work.

## Budget the complete staged program

The compiler seeds targets with consumption through the requested horizon,
then calculates complete production demand, existing input debt, deduplicated
workshop costs, shared-worker dependencies and finite harvest service.
Consumption is budgeted through the later of the original reserve tick and
the modeled end of acquisition plus production. Targets increase monotonically
until demand and schedule agree, within 64 rounds.

The complete source is bounded and arithmetic checked. Both the explicit
horizon and combined work remain within one reference game year (403,200
ticks). Insufficient sources, unknown stock or population, unavailable workers,
unsafe sites and nonconvergence refuse the whole proposal. Forecast output is
never available inventory.

Harvesting labor permission remains `acquire_resources.assign_labor`.
Production and construction staffing retain the separate `prerequisites`
policy. Neither option grants a capability. Current Observe, Query, Plan and
each effect's ordinary scoped authority remain required.

Acquisition for already satisfied current output minima uses the distinct typed
`LaboratoryResourceProductionReserveUntil` admission. It permits finite harvest
and the relevant harvesting labor enables only. The older
`LaboratoryProductionReserveUntil` policy retains its production-only action
set. Normal source-generation, supply, checkpoint, risk, budget and mandatory
action-proof checks still apply.

## Read current truth and forecasts separately

`production.resource_workflow` uses `dfmcp.production-resource-workflow/2` and
reports the current stage, complete original request, exact allocations and
resource requirements. `production.consumption` reports the explicit reserve
tick, source tick and absolute stock-sizing horizon. Subtract the source tick
to obtain the remaining duration; the workflow retains both stage durations.
Sealed action obligations
remain based on the current stage's actual work; a distant reserve tick does
not extend a stuck harvest deadline.

The ordinary forecast executes only the current stage. A harvest forecast
does not run a later production plan implicitly. Its separate `reserve_forecast`
examines the requested endpoint on the discarded model world when evaluation
is available. Read availability and future truth separately from ordinary
action-completion fields.

Current semantic stock truth, exact action proof, historical first achievement
and remaining reserve demand are distinct facts. A True minimum is not a
certificate that supplies will last. A predicted True future endpoint does
not prove continuous maintenance throughout the interval. Population changes,
interference, casualties and delayed observation can invalidate predictions;
fresh evidence governs each next stage.

## Cancellation, recovery and shared sessions

Cancellation retains completed yield; the next stage sizes only the observed
residual. All earlier same-root pursuits must be physically quiet. Active or
unknown work blocks another stage, and commit repeats the checks so concurrent
reviewed candidates cannot duplicate an active pursuit.

Continuation `/5` is a flat, domain-separated source retaining the complete
request and immediate parent/root identities. Durable recovery reconstructs
it at the exact archived snapshot, retaining the original stock ledger ID and
generation. Recovery grants no authority and dispatches no old action.
Checkpoint restore abandons the old pursuits. A replacement ledger cannot
satisfy or continue the old objective.

Remaining reserve work stays discoverable through ordinary goal observations
and handoff even when present stock minima are True. These views use the same
source and authority gates as continuation intake. Unknown reserve evidence
cannot establish no-work or authorize discarding unfinished goal custody.

## Saved sources and publication

Historical `resource_aware_v1` still rejects `reserve_until_tick`. The older
consumption-reserve workflow retains false-goal-only continuation. Saved `/1`
through `/4` sources keep their compiler, canonical bytes and digest domain.
The new combination selects `/5`; changing only an old schema label cannot
import its semantics.

The existing eleven tools are unchanged. Source changes before commit require
a fresh review. Complete Agent Turn publication must fit before pending work
is replaced. Authority, source, no-work, deadline and output-budget refusals
preserve the prior review; complete action proof and lineage are not truncated.

See [finite-resource production](LAB_RESOURCE_PRODUCTION.md),
[production reserves](PRODUCTION_RESERVE_PLANNING.md), and
[goal continuation](PRODUCTION_GOAL_CONTINUATION.md). The machine contract is
[`laboratory_resource_production_reserve_v1.json`](../architecture/laboratory_resource_production_reserve_v1.json),
the request schema is
[`lab_resource_production.schema.json`](../schemas/lab_resource_production.schema.json),
and the owning bead is `df-lab-resource-harvest-h2m.4`. Executed validation and
remaining qualification limits are recorded in `IMPLEMENTATION_STATUS.md`.
