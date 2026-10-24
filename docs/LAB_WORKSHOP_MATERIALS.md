# Finite workshop materials in the laboratory

The explicit `wooden-workshops/1` model gives workshop setup a finite material
cost. A Still, Kitchen or Carpenters workshop requires one qualified WOOD log
from the same inventory used by carpentry. It composes with
`shared-workforce/1`, so constructing that workshop also requires an eligible
living constructor. These are reference-simulator rules, not native Dwarf
Fortress construction costs or compatibility claims.

## Use the economy scenario

Open the existing `fortress.open_session` tool with
`scenario: "starter_fortress_economy"`. For a complete connected-room or
production workflow, request the usual scoped capabilities:

```json
{
  "scenario": "starter_fortress_economy",
  "paused": false,
  "requested_capabilities": [
    ["observe", "read_only"],
    ["query", "read_only"],
    ["plan", "read_only"],
    ["control_clock", "reversible"],
    ["designate", "guarded"],
    ["construct", "guarded"],
    ["configure_labor", "reversible"],
    ["configure_production", "reversible"],
    ["checkpoint", "guarded"]
  ]
}
```

The scenario composes the exact shared-workforce starter with one explicit
material-model declaration. It retains the starter's completed workshops and
inventory; existing workshops can be reused without charging a new input.
The `starter_fortress_workforce`, `starter_fortress`, `besieged_fortress` and
`empty` scenarios retain their prior declarations and behavior. Ordinary
production, `provisioned_room` and `connected_room` requests use this model
without a new MCP tool or blueprint grammar. See
[LAB_CONNECTED_ROOM_ACCESS.md](LAB_CONNECTED_ROOM_ACCESS.md) for the connected
room request and [LAB_SHARED_WORKFORCE.md](LAB_SHARED_WORKFORCE.md) for workers.

## Model and material lifecycle

The canonical fortress-settings record selects
`laboratory_construction_material_model = "wooden-workshops/1"`.
The model is explicit in canonical state; existing worlds without the marker
retain their original Build behavior. New model versions cannot be inferred
from labels, inventory quantities or old construction progress.

| Construction phase | Free WOOD | Material custody |
|---|---|---|
| Planning and forecast | Unchanged in the source world | No real allocation |
| Successful dispatch | Decreases by one | One log held for the exact building |
| Work in progress | No further debit | The same held allocation remains |
| Completion | No further debit | Held log becomes consumed |
| Unfinished cancellation | Increases by one | Allocation becomes released |
| Repeated cancel or cancel of complete work | Unchanged | No second refund |

Held logs are already absent from available stock, so other construction and
carpentry cannot spend them. Completion and cancellation preserve the original
allocation record, including its stock ledger, generation, quantity and model.
Canonical snapshots retain these records through reconstruction and recovery.
Cancellation does not undo completed construction or refund consumed material.

The current model supports exactly `workshop:Still`, `workshop:Kitchen` and
`workshop:Carpenters`, with observed supported footprints. Required material
tokens may be empty or exactly `WOOD`; forbidden tokens must not exclude WOOD.
`prefer_nearest=true` and nonzero `reserve_count` refuse because this aggregate
log inventory cannot establish those spatial or reservation semantics. Other
generic Build families refuse in a material-model world. Existing exact-item
`PlaceFurniture` remains the route for installing produced furniture.

Unavailable quantities, duplicate or replaced stock ledgers, malformed
allocations, unsupported markers, identity collisions and arithmetic overflow
refuse. Active old construction cannot silently acquire a material history.
Removing the global model while stamped task history survives also refuses.
The normal caller-owned transaction shadow prevents partial publication.

## Planning the complete resource demand

Planning must account for three demands against the same available supply:

1. New workshop setup.
2. New furniture production.
3. Remaining material demand from existing carpentry orders.

A new Carpenters workshop and two new beds therefore need three available
logs. Two logs cannot fund that complete program. Reusing an established
completed workshop adds no new setup cost. Existing held construction logs are
already debited from free stock and are not subtracted a second time.

The static action planner checks joint demand before producing a candidate.
Each supported Build seals the model and its per-action availability guard,
then requires the exact consumed allocation in its postconditions and
completion obligation. An explicit caller deadline stays exact. A custom
predicate cannot turn unconsumed material into completed workshop proof.

The material model requires the shared-workforce model, so temporal plans
already carry the complete-workload replay guard. Changed anchors require a
freshly reviewed plan; historical inventory and a narrow geometry witness do
not authorize a different material allocation.

The production compiler first determines the actual deduplicated workshop
setup, including reuse of completed workshops. It subtracts those new inputs
and pending carpentry demand before admitting furniture quotas. The same
accounting applies to food/drink requests that need a new Still or Kitchen.
Analysis reports `construction_materials.new_workshop_wood` and the WOOD
quantities reserved for existing orders, new workshops and new production.
These are planning calculations; only authorized dispatch allocates inventory.

## Current reports and forecasts

With current Observe authority, session opening returns
`construction_materials`; observations expose it under
`world.construction_materials`, and the orientation briefing forwards that
current report. The report binds the exact source anchor and stock ledger and
separates available, held, consumed and released quantities. Allocation rows
are bounded with an explicit omitted count. Per-action
`work_state.construction_materials` selects the original building from the full
qualified census before rendering, so a global row limit cannot hide that
action's allocation. Unavailable model or inventory evidence is reported as
unknown rather than a current quantity. Reports grant no mutation authority.

`consumed_wood` and `released_wood` describe retained workshop-allocation
history. They are not extra available stock, and they do not include furniture
recipe consumption. Releasing and later reusing the same log can therefore
leave both a historical released row and a later consumed row.

Forecasts expose `predicted_construction_materials` on the discarded simulation
with a predicted anchor and explicit predicted evidence. They do not allocate
real logs or certify future completion. Current goal proof independently
checks completed construction custody; replacement generations or a changed
allocation state cannot pass solely because ordinary field predicates do.

## Interrupted projects

An unfinished workshop cancellation releases its input and retains the exact
released receipt. Repeating cancellation cannot return another log. After
canonical recovery, the receipt remains visible, while retired process-local
action handles remain retired. A freshly reviewed room continuation can
reuse that original site only after the released allocation has been
validated, and its new Build must acquire available material again. A
completed workshop retains consumed material and is reused as capacity.

The focused connected-room test exercises a two-bedroom project with a missing
carpenter's workshop: two logs refuse before dispatch; three logs complete the
new workshop and two beds. Forecast and execution agree on material totals and
completion time at the same cadence. The cancellation/recovery test refunds
the unfinished workshop, reopens the canonical world and completes that same
room goal from the recovered inventory and original site.

## Scope

The cost is a closed one-log laboratory recipe, independent of native game
construction quantities. The model does not harvest wood, haul individual
logs, rank nearest materials, consume stone or metal, or construct unsupported
building families. It establishes finite resource allocation and conservation
for the workshop setup used by complete production and furnished-room plans.

See [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) for executed
evidence and its full-server, native and live qualification limits.
