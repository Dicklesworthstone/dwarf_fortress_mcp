# Plan production that survives consumption

The deterministic laboratory compiles original DRINK, FOOD, BED, DOOR, TABLE
and CHAIR quotas into
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

## Produce furniture from a shared supply of logs

Furniture quotas use the same `fortress.plan` production request and sealed
plan → commit → wait → prove workflow. A request can mix all six supported
outputs. For example:

```json
{
  "template": "production",
  "quotas": [
    {"item": "BED", "minimum": 4},
    {"item": "DOOR", "minimum": 4},
    {"item": "TABLE", "minimum": 1},
    {"item": "CHAIR", "minimum": 1}
  ]
}
```

The closed reference carpentry recipes each produce one item and consume one
log from `stock.material.WOOD`. Output counts are explicit qualified facts at
`stock.item.BED`, `stock.item.DOOR`, `stock.item.TABLE` and `stock.item.CHAIR`.
An absent or unavailable count is unknown; it is not a zero starting stock.
All outputs share the same log supply, so the example needs ten logs when all
four output counts start at zero. Already held output is credited once. The
planner refuses a material deficit before producing a candidate program.

The starter fortress now has a completed `workshop:Carpenters`, an enabled
carpenter, twenty observed logs and explicit zero furniture counts. Each
reference completion creates a source-order-linked item as well as increasing
the corresponding stock count. The original terminal predicate requires the
requested stocks, independently of the individual work-order counters.
Furniture has zero metabolism consumption allowance; DRINK/FOOD in the same
request retain their consumption reserves through the complete shared horizon.

Current finite workshop and worker capacity still apply. All four furniture
recipes share one selected `CARPENTRY` worker and workshop capacity slot.
Joint selection compares the three workshop/labor families across all six
outputs. It maximizes distinct eligible workers across those slots, then
minimizes labor-enable cost, military preference and canonical IDs. Adding more
furniture output types does not enable more carpenters. The compiler selects
one canonical workshop per family and serializes jobs sharing that workshop or a selected worker through
verified-completion dependencies. This is conservative even when extra
carpentry workshops could permit more parallelism. Dependency horizons account
for that serialization instead of expiring later orders while they wait.

When setup is necessary, the existing explicit `prerequisites` object may name
`workshop:Carpenters` in addition to Still and Kitchen, with at most one site
per family and three sites total. Its footprint, support and safety halo must
be observed. A shared workshop is built only once, and a selected citizen's
same disabled labor is enabled only once. Every generated step retains its own
ordinary capability requirement; missing or unknown labor does not grant
permission to configure it.

Existing marked carpentry work contributes both remaining service and a
conservative debit of one log per unfinished unit. This debit is reported in
`production.materials.WOOD` as observed stock, the amount reserved in the model
for older work, and the amount left for the new candidate. It is not a native
reservation or future stock. An active producer for an output requested by the
new plan refuses overlapping quota work; the caller must first observe,
reconcile or explicitly stop the original work. New furniture planning also
refuses legacy carpentry without the qualified `carpentry/1` marker. Historical
food/drink-only requests preserve their old treatment of unmarked abstract
carpentry and reproduce the original program.

The bounded model does not harvest logs, infer missing inventory, establish
native recipe semantics, build rooms from those items automatically, or admit
a live mutation profile. Concurrent interference can invalidate the prediction;
actual qualified stock and item evidence determine completion.

## Fund food and drink reserves with finite ingredients

The opt-in `starter_fortress_provisions` scenario adds the explicit
`plant-provisions/1` model over the existing shared workforce and workshop
materials. Brewing and cooking each consume one shared PLANT per completed
five-unit batch. Earlier scenarios keep their original food recipes.

Every consumption-reserve iteration budgets the ingredients for the complete
planning stock targets, after retaining pending demand from older physical
orders. New workshop setup can therefore increase both WOOD requirements and
the PLANT needed to sustain the longer horizon. `production.materials.PLANT`
separates observed supply, model-only pending demand and the supply available
for the new candidate. A raw PLANT row has zero direct metabolism allowance;
it still pays for every added production batch.

See [LAB_FINITE_PROVISIONS.md](LAB_FINITE_PROVISIONS.md) for scenario selection,
current allocation history, forecast semantics and interrupted-goal recovery.
