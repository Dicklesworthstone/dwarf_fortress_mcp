# Finite workshop materials in the laboratory

The explicit `wooden-workshops/1` model gives workshop setup a finite material
cost. A Still, Kitchen or Carpenters workshop requires one qualified WOOD log
from the same inventory used by carpentry. It composes with
`shared-workforce/1`, so constructing that workshop also requires an eligible
living constructor. These are reference-simulator rules, not native Dwarf
Fortress construction costs or compatibility claims.

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

## Scope

The cost is a closed one-log laboratory recipe, independent of native game
construction quantities. The model does not harvest wood, haul individual
logs, rank nearest materials, consume stone or metal, or construct unsupported
building families. It establishes finite resource allocation and conservation
for the workshop setup used by complete production and furnished-room plans.

See [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) for executed
evidence and its full-server, native and live qualification limits.
