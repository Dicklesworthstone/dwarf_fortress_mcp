# Finite food and drink inputs in the laboratory

The explicit `plant-provisions/1` model gives brewing and cooking finite
ingredients. Each completed `BREW_DRINK`, `PREPARE_MEAL` or `COOK_MEAL` batch
consumes one qualified PLANT unit and produces the existing five units of
drink or food. Both production families draw from the same inventory, while
their workers and completed workshops retain the existing shared-capacity
requirements. These quantities are reference-simulator calibration, not native
Dwarf Fortress recipes or compatibility evidence.

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

## Scope

The closed recipes do not model agriculture, gathering, barrels, hauling,
ingredient quality, crop species or native recipe costs. PLANT is not consumed
directly by metabolism; completed food and drink retain the existing population
consumption model. No new action kind, dependency, background loop, native
mutation profile or MCP tool is introduced.

See [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) for the exact
executed evidence and full-workspace, server, native and live limitations.
