# Finite food and drink inputs in the laboratory

The explicit `plant-provisions/1` model gives brewing and cooking finite
ingredients. Each completed `BREW_DRINK`, `PREPARE_MEAL` or `COOK_MEAL` batch
consumes one qualified PLANT unit and produces the existing five units of
drink or food. Both production families draw from the same inventory, while
their workers and completed workshops retain the existing shared-capacity
requirements. These quantities are reference-simulator calibration, not native
Dwarf Fortress recipes or compatibility evidence.

## Use the provisions scenario

Open the existing `fortress.open_session` tool with
`scenario: "starter_fortress_provisions"`. A production session with explicitly
authorized workshop setup can request:

```json
{
  "scenario": "starter_fortress_provisions",
  "paused": false,
  "requested_capabilities": [
    ["observe", "read_only"],
    ["query", "read_only"],
    ["plan", "read_only"],
    ["control_clock", "reversible"],
    ["construct", "guarded"],
    ["configure_labor", "reversible"],
    ["configure_production", "reversible"],
    ["checkpoint", "guarded"]
  ]
}
```

The scenario adds exactly the provisions marker and 20 observed PLANT units to
the existing economy starter. Shared workers, finite workshop WOOD and the
starter's other inventory remain in place. Earlier scenarios and saved-source
generations keep their original recipe interpretation. The ordinary production
blueprint works without a new tool or action grammar:

```json
{
  "template": "production",
  "quotas": [
    {"item": "DRINK", "minimum": 60},
    {"item": "FOOD", "minimum": 65}
  ]
}
```

Pass that object as the existing `fortress.plan` blueprint JSON string, then
use the normal reviewed plan, commit, wait and proof workflow. Optional
prerequisites can authorize existing labor/workshop setup; their separate
capabilities and finite WOOD costs still apply. See
[PRODUCTION_RESERVE_PLANNING.md](PRODUCTION_RESERVE_PLANNING.md).

## Explicit model and original order history

The canonical fortress-settings record selects
`laboratory_provisions_model = "plant-provisions/1"`. The model requires the
qualified `shared-workforce/1` declaration. Available ingredients are held in
`stock.material.PLANT` on one unambiguous original-generation stock ledger;
food and drink output counts must also be qualified unsigned quantities.

New food orders retain `production_model = "plant-provisions/1"`, their original
ledger identity and generation, and `provisions_consumed_plants`. The consumed
count must equal original batches minus remaining batches. Lifecycle, partial
service, action name, recipe, original amount and conditions remain explicit.
Current identity checks reject replacement generations, duplicate ledgers,
unqualified counters and malformed histories.

Worlds without the model preserve their previous order and action semantics.
An active old food order cannot silently acquire an ingredient-consumption
history; completed or cancelled historical orders can predate the model.
Unsupported or misplaced declarations and removal of the model while stamped
history remains refuse. Missing ingredient evidence is unknown, not zero or a
free source of future food.

## Production and cancellation

| Transition | Available PLANT | Output and retained history |
|---|---|---|
| Planning or forecasting | Source unchanged | No real reservation or production |
| Dispatch | Unchanged | Original finite order, zero consumed inputs |
| Partial batch service | Unchanged | Earned partial work retained |
| Completed batch | Decreases by one | Five output units, one consumed input, one fewer remaining batch |
| Cancellation | Unchanged | Stops future work; completed batches and input history remain |
| Repeated cancellation | Unchanged | No invented refund or repeated production |

Input availability gates work before worker selection. Each completion checks
it again after earlier completions at the same boundary, so two producers
cannot spend the last plant. A losing completion retains its previously earned
49 ticks, without retaining the rejected completion tick or banking later idle
time. A known shortage blocks work; a subsequently qualified replenishment can
resume the original partial batch. This model does not itself gather plants.

Conditions remain independent gates. `MaterialAvailable` does not consume its
threshold or reserve stock; the registered recipe consumes exactly one input
per completed batch. Item quotas and named predecessor conditions retain their
existing semantics. Caller-owned transaction shadows publish input consumption,
output and remaining-work changes atomically or publish none on refusal.

Canonical snapshots retain completed and cancelled consumption histories. A
restart does not refund consumed plants or make an old process-local handle
valid again. Unfinished batches have no held material to release.

## Planning and proof

Static action planning checks the combined plant demand of every new food/drink
batch plus every active original finite-food order's remaining batches. Old
physical work still counts when its request handle has retired. Cancelled and
completed orders contribute no future demand; their consumed input is already
absent from current stock and is not deducted again. This calculation is a
conservative feasibility check, not an inventory reservation or a guarantee
against later interference.

Each new food step seals the explicit model and original stock-ledger kind,
plus the full input quantity needed by that order. Mandatory completion
predicates require the exact original order and complete consumed-input
history. They are conjoined with caller-supplied postconditions and obligation
terminals; an explicit deadline remains unchanged. Dispatcher and laboratory
proof publication additionally validate original generations and the complete
ledger/history domain, which field predicates alone cannot express.

The shared workforce already seals the complete-workload replay guard. An
anchor change therefore requires reviewed replay before the shell can commit
the plan. Planning from a narrow geometry or output witness cannot silently
authorize changed ingredient demand.

## Consumption-aware ingredient budgets

The quota compiler selects its recipe catalog from the qualified snapshot.
Under this model, FOOD and DRINK each have a PLANT input, so their ingredient
demand is solved jointly. Before considering new production, the compiler
subtracts the complete active original-order demand. Possible future output
from those orders is never counted as current stock.

The consumption-reserve fixed point recalculates ingredients for every added
batch. More construction, queued work or shared-worker waiting can lengthen
the sealed horizon, require more food/drink reserve, and therefore require
more plants. A plan that funds its immediate stock deficit can still refuse
because it cannot fund its full reserve. The integration fixture with new
Still and Kitchen setup needs seven plants for its reviewed reserve, although
two would cover the initial food/drink deficits.

`production.materials.PLANT` reports the observed quantity, the conservative
debit for existing orders and the quantity left for new production. These
debits are explicitly model-only. The PLANT requirement is a raw recipe input
with zero direct metabolism allowance; its recipe consumption still includes
every reserve batch. Workshop/furniture WOOD and provisions PLANT remain
separate finite supplies in a mixed production program.

Original stock goals retain the requested minima. A stock goal may become
true before all reserve work finishes, or before a deferred order starts.
Current goal proof validates the provisions census and the identity/history
of any existing original order without confusing current stock truth with
action completion. Whole-plan completion still requires verified action
proofs, the true original goal and physical quiescence at the same frontier.

## Current reports and forecast results

With current Observe authority, session opening exposes `provisions`, and
observation exposes `world.provisions`. The orientation briefing copies only
the current response's report. It includes the exact source anchor and ledger,
available plants, consumed history, active remaining demand and bounded order
rows with an explicit omitted count. Remaining demand is not reserved stock
and must not be added to availability.

Per-action `work_state.provisions` looks up the exact original order from the
full qualified census before presentation bounds. Its row includes current
input-shortage information. Missing or unqualified evidence is displayed as
unknown; it cannot become a zero or a retained current quantity. Ingredient
shortages remain separate from worker/workshop capacity blockers.

Low-stock alerts screen recommendations that need new production against the
qualified ingredient supply after existing orders' remaining demand. When
necessary ingredients are short, the alert names that blocker and suppresses
the proposed production or labor remedy. Missing evidence remains unknown,
without invented stock counts. This is only a necessary-input screen: the
production compiler still determines the full consumption-reserve budget.
Retained hunger or thirst flags do not invent an ingredient requirement when
the proposed stock target is already met.

Forecasts return `predicted_provisions` with a predicted anchor and discarded
simulation scope. They retain separate action, goal and quiescence outcomes.
The source inventory and order history remain unchanged by forecasting.

## Continue an interrupted reserve plan

If an order is cancelled after producing some batches, those outputs and
consumed-input records remain. Recovery preserves the exact remaining plant
stock and cancelled history while invalidating retired process-local action
handles. An explicit production continuation pursues the unchanged original
quotas from the current source; it does not restart the original batch amount
or refund completed food.

The integration tests exercise this cancellation, canonical reopening and
continuation path. They also cover external ingredient depletion after commit:
the original physical work stalls, and later qualified replenishment resumes
the retained partial batch. Recovery cannot reinterpret missing ingredients
as free production. Gathering and agriculture remain outside this model.

## Scope

The closed recipes do not model agriculture, gathering, barrels, hauling,
ingredient quality, crop species or native recipe costs. PLANT is not consumed
directly by metabolism; completed food and drink retain the existing population
consumption model. No new action kind, dependency, background loop, native
mutation profile or MCP tool is introduced.

See [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) for the exact
executed evidence and full-workspace, server, native and live limitations.
