# Retained dependencies in laboratory production orders

The laboratory production blueprint accepts the explicit order compiler policy
`orders: "dependency_preserving_v1"`. It lowers the complete production analysis
into ordinary requested actions while preserving material gates and predecessor
dependencies. The policy also gives every generated order a bounded name tied to
the exact canonical request, observed source anchor and normalized model. A later
pursuit therefore cannot confuse a retained earlier order with its own supplier.

Pass this object as the existing `fortress.plan` `blueprint` JSON string:

```json
{
  "template": "production",
  "orders": "dependency_preserving_v1",
  "quotas": [
    {"item": "BED", "minimum": 3},
    {"item": "CHAIR", "minimum": 2}
  ]
}
```

The normal Plan, review, Commit, Wait and current-goal proof workflow applies.
The optional policy composes with `prerequisites`, complete-project staffing and
`reserve_until_tick`. It does not supply any action capability, reserve workers
or materials, add a tool, or authorize native DFHack actions.

## What is retained

The joint compiler already records which producer steps supply other recipes.
The new core `ProductionPlan::into_requested_actions` keeps these as relative
`RequestedAction.depends_on` indices and as `CompletedOrder` conditions naming
the exact predecessor. A shell that inserts setup must remap the indices before
sealing. The laboratory consumer performs that remapping and also adds exact
completion conditions for orders serialized behind a shared selected worker or
workshop. Labor and workshop setup retain their normal action dependencies; they
are not represented as invented work orders.

Input gates require one batch's coefficients. A three-batch recipe consuming two
units per batch uses a material minimum of two, even though the original order
needs six units in total. Requiring six before every batch would prevent the last
two batches from finishing after the first batch consumed its inputs. Whole-plan
feasibility still budgets the complete input demand, including older physical
work and any setup or consumption reserve. The gate itself consumes nothing;
only the registered reference recipe performs the existing input consumption.

Every original stock minimum remains part of the terminal goal. Neither a named
predecessor's completion nor the consumer's completed batch count replaces the
current stock proof. The existing qualified source, physical-quiescence and
original-effect identity checks remain necessary.

## Explicit material bindings

A model resource key is not inherently a material field. The pure proposal API
requires an explicit map from every used input key to its material token.
`MaterialAvailable` reads `stock.material.<token>`; item stock fields have a
different interpretation. Missing bindings and two distinct model resources
mapped to one material token are refused. Aliasing the model's independently
budgeted resources would otherwise permit double counting.

The actual laboratory integration uses only its closed `WOOD -> WOOD` and
`PLANT -> PLANT` bindings. It cannot import arbitrary recipe labels or material
mappings from a blueprint. Carpentry retains one WOOD per completed item.
Food and drink use PLANT only when the qualified `plant-provisions/1` model is
present. Earlier laboratory worlds keep their established food recipes.

The core method can describe other caller-declared models, but its proposals do
not establish native recipe semantics, field meanings, source eligibility,
workshop readiness or effect authority. A consumer must independently admit
those bindings before execution.

## Names, replay and continuation

Order names include a versioned SHA-256 identity and the canonical step index.
The identity covers the caller's source namespace, complete normalized resource
balances, recipe outputs and yields, job and workshop labels, input quantities,
the material bindings actually used, and dependency indices. Distinct steps
using the same job token have distinct names. Reordering equivalent quotas or
ingredients, repeating an equivalent minimum, or supplying an unused valid
binding does not change the resulting proposal bytes.

The laboratory namespace covers the complete canonical production request and
the fortress ID, observation epoch and sequence, tick and source state hash.
Compiling an archived request at its original snapshot reproduces its names,
conditions, action identities and seal. A reviewed replay or continuation at a
new source frontier receives new names; the old plan and order history remain
unchanged. The operator cannot provide this namespace through the blueprint.

The optional `orders` policy is retained in the original canonical request and
therefore in the production source seal and the existing flat continuation
envelopes. Durable reconstruction uses that retained policy. A new pursuit after
a verified recovery uses its new frontier without upgrading historical sources.
The normal continuation eligibility checks still require current authority,
False original goal truth and proven quiescence of the earlier lineage.

Unmarked requests keep their existing compiler, canonical source bytes, order
names and conditions. There is no implicit migration. A saved request specifying
the new order policy must also carry its selected production planner generation;
current intake supplies that existing generation before saving.

## Inspection, bounds and failure

The production analysis exposes `order_compiler` with its policy, exact source
anchor and namespace, explicit input-binding interpretation and dependency
semantics. `reserves_materials` and `native_authority` are false. Inspect the
ordinary sealed steps for the concrete names and conditions.

The existing source, action and output budgets still apply. Core proposals are
bounded by the joint planner's reachable model and at most 64 conditions per
order. If preserving all input and predecessor gates exceeds that limit, the
entire lowering fails; it never emits a partially protected action list.
Malformed, control-containing, missing or aliased material bindings also refuse.
Insufficient or unqualified current inventory still refuses actual laboratory
planning before effects.

Tests exercise the pure proposal compiler and the actual laboratory compiler,
StaticPlanner and MemoryAdapter: shared suppliers, exact predecessor gates,
per-batch inputs, stable names, binding refusal, unchanged legacy bytes, complete
multi-output production, setup dependency remapping, retained prior orders,
canonical source replay and continuation after verified recovery. These are
reference-simulator checks. See `IMPLEMENTATION_STATUS.md` for the exact executed
scope and the independent full-workspace and native qualification status.
