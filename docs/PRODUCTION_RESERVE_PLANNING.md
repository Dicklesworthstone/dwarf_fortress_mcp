# Plan production that survives consumption

The deterministic laboratory compiles original DRINK and FOOD quotas into
ordinary sealed work orders, with optional explicitly authorized labor and
workshop setup. New `fortress.plan` requests use the fixed
`consumption_aware_v1` compiler. The existing `blueprint` JSON string accepts:

```json
{
  "template": "production",
  "quotas": [
    {"item": "DRINK", "minimum": 60},
    {"item": "FOOD", "minimum": 65}
  ]
}
```

No new tool or background production loop is introduced. Planning returns a
candidate; the caller reviews its digest, actions, requirements and forecast,
then explicitly commits it. Existing capability and checkpoint rules still
apply to every actual action. See `LAB_SEMANTIC_ACTIONS.md` for the action loop
and `PRODUCTION_GOAL_CONTINUATION.md` for later pursuit of an unmet retained goal.

## Preserve the original goal while budgeting the work

A batch finishing does not establish that enough stock remains. Population
consumes supplies while construction, queued work and production run. The new
compiler reads source-qualified living population and the actual metabolism
phase, then budgets every original quota through the complete sealed obligation
horizon. It includes a quota already satisfied at the source when that quota
would otherwise fall below its original minimum during the plan.

The compiler iterates between the production schedule and its consumption
reserve until both are consistent. Each planning target is at least the
original minimum plus the projected consumption through that schedule. Adding
work can extend the horizon and require more reserve, so a single calculation
is insufficient. Targets increase monotonically, and at most 64 rounds execute.

Original terminal predicates keep the user's exact minima. Additional reserves
are planning quantities; they do not rewrite the requested goal or count
unproduced goods as observed stock. For example, at starter-fortress tick 1100,
40 observed drinks and a minimum 60 formerly produced four brewing batches. The
meal at tick 1201 left 53 drinks when those batches finished. The new compiler
budgets seven drinks for consumption, requests six batches, and the actual
reference timeline finishes at 63 drinks. The original predicate is still
`DRINK >= 60`.

Both new ordinary quota requests and requests with explicit setup options use
joint staffing selection. If brewing and cooking must share their only eligible
worker, one order depends on the other's verified completion. This lets each
order retain a usable deadline after known waiting time. Read-only selection
never enables labor or builds a workshop without the existing explicit setup
permissions and sites.

Existing physical queue inspection still refuses overlapping same-output
producers, including work whose original handle has retired. Other queued
service, earned partial progress, setup deadlines and shared-worker dependencies
contribute to the complete horizon. Observation cadence still matters: deferred
work starts only when a foreground observation verifies its prerequisites.

## Read the analysis and forecast

`production.requirements` retains `minimum_stock` and adds
`planning_stock_target`, `consumption_allowance`, and
`predicted_consumption_through_horizon`. The `production.consumption` object
identifies the fixed compiler, source hash/tick, observed population and
metabolism phase, reference consumption intervals, complete horizon, iteration
count and assumptions. Its epistemic state is `predicted`, and
`completion_guaranteed` is false.

The forecast executes the sealed plan on a discarded laboratory fork. Its
`predicted_complete` requires all three at the same predicted frontier:

- every action proof is Verified;
- the exact original terminal predicate is True;
- the original actions' physical work is quiescent.

`predicted_actions_complete`, `predicted_goal_truth` and
`predicted_physical_quiescent` expose these separately. Unknown goal truth stays
Unknown. An early verified action does not hide continuing physical work.
`predicted_completion_tick` exists only for whole-plan completion;
`predicted_actions_completion_tick` retains the separate action-proof timing.
The forecast remains bounded by the sealed horizon and 400 time slices, and a
source, advance, poll or work-inspection refusal makes it unavailable. Predicted
snapshots never become canonical evidence or dispatch authority.

## Exact saved-plan behavior

Compiler selection is retained with the original source. New production intake
adds `"planner":"consumption_aware_v1"` to canonical source JSON. Historical
production sources without that field retain their previous compiler, canonical
bytes, summary, action program and prepared-plan seal when reconstructed at their
original snapshot. Parsing an archive does not choose today's compiler.

For modern production, a domain-separated digest of the entire canonical request
is included in the actual prepared-plan summary. This seals compiler generation,
all original quotas, permissions and site choices, even when some setup fields
are unused at the current anchor. Custom summaries must leave space for that
seal within the existing 256-byte bound.

New explicit continuations select `dfmcp.production-continuation/2`, retaining
the complete original request unchanged while choosing the consumption-aware
compiler for the new pursuit. Historical `/1` continuations retain their exact
compiler, bytes and digest domain. Ordinary archived reconstruction and exact
pending-candidate retry never upgrade a source silently. A newly requested
continuation can use the new generation even when its parent is historical.

## Bounds and limits

The model uses registered laboratory recipe yields and consumption intervals.
It assumes observed workers and workshops remain available, population does not
change, no new competing work appears, and the agent observes dependencies and
completion within the reviewed deadlines. Pausing delays progress. Scheduled
changes, interference, casualties and inadequate observation cadence can
invalidate the prediction; fresh goal evidence determines actual completion.

Unknown, unqualified or future-dated population/metabolism facts refuse reserve
planning. Existing workload and setup admission remains conservative. Checked
quantities, the 65,536-entity bound, the 64-round limit and the one-game-year
obligation bound prevent unbounded planning. Refusal means this bounded model
could not establish the requested plan; it does not prove every possible
production strategy infeasible. If every original quota already holds and
there is no production work, the compiler preserves the existing no-work
result instead of inventing a maintenance horizon.

This functionality is the executable reference laboratory. It does not claim
DFHack recipe behavior, native mutation admission or compatibility evidence.
