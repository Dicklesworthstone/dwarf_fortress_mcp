# Wooden furniture production in the laboratory

The laboratory can manufacture beds, doors, tables and chairs through the
ordinary `fortress.plan` → `fortress.commit` → `fortress.wait` workflow.
Production consumes finite wood, competes for citizens and workshops, and
creates inspectable inventory. The original stock goals remain part of the
sealed plan and its durable history.

This is reference-simulation behavior. It is not a DFHack qualification or a
change to the production protocol map, native methods, or compatibility registry.

## Recipe and inventory contract

| Quota token | Canonical job | Workshop | Labor | Input per unit | Output per unit |
|---|---|---|---|---|---|
| `BED` | `CONSTRUCT_BED` | `workshop:Carpenters` | `CARPENTRY` | one `WOOD` | one bed |
| `DOOR` | `CONSTRUCT_DOOR` | `workshop:Carpenters` | `CARPENTRY` | one `WOOD` | one door |
| `TABLE` | `CONSTRUCT_TABLE` | `workshop:Carpenters` | `CARPENTRY` | one `WOOD` | one table |
| `CHAIR` | `CONSTRUCT_THRONE` | `workshop:Carpenters` | `CARPENTRY` | one `WOOD` | one chair |

The calibration is 50 game ticks of eligible service per completed item.
Workshop and citizen capacity is shared with brewing and cooking. A citizen
enabled for several labors cannot simultaneously service several orders, and
one carpenter workshop cannot service several furniture orders at once.

The stock ledger uses `stock.material.WOOD` and
`stock.item.BED`, `stock.item.DOOR`, `stock.item.TABLE`, `stock.item.CHAIR`.
Each output also has an `Item` record with deterministic identity derived from
its original order and production ordinal. Advancing time in several pieces
produces the same item identities, stock quantities and physical work as the
same uninterrupted interval. Publication cursors, revisions and observation
timestamps still reflect the actual calls.
Metabolism consumes drink and food; furniture is not assigned a consumption
rate.

Only source-qualified, explicit counts establish stock. Missing or stale wood,
an absent output count, or unavailable labor does not mean zero or permission to
proceed. Work-order conditions, material checks, cancellation, and original
deadlines continue to apply. Existing abstract furniture orders without the
new `carpentry/1` production marker do not retroactively manufacture items.

## Plan a complete furnishing supply

Open an unpaused `starter_fortress` laboratory session with current Observe,
Query, Plan, ConfigureProduction and ControlClock grants. The starter includes
a carpenter, a completed carpenter workshop, 20 modeled wood units and explicit
zero counts for the four furniture outputs. For a custom world, inspect the
current stock, living citizens and complete workshops first.

Pass this JSON string as the `blueprint` argument to `fortress.plan`:

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

The plan sizes finite orders from the observed deficit. It considers all
requested outputs together, so those ten items require ten logs in total.
Outstanding marked carpentry orders retain their own conservative wood demand;
their future output is never counted as current inventory. A request that
cannot fit the remaining stock fails before any effect.

Review the plan's analysis, staffing, workshop dependencies, predicted outcome,
postconditions and exact digest. Commit that digest explicitly, then use a
bounded positive-tick `fortress.wait` to advance the unpaused world. Waits can
complete earlier orders and dispatch their dependent orders within one call.
Inspect canonical item rows with `fortress.query` using:

```json
{"mode": "entities", "kind": "item", "limit": 32}
```

The `blueprint` and query `mode` arguments are JSON strings in the laboratory
MCP tool interface. Tool responses retain their normal Agent Turn and authority
checks. A plan is a proposal; a forecast is a prediction; only later observed
stock can prove the original goal.

## Establish missing capacity explicitly

The existing `prerequisites` object can authorize planning a labor enable or
specify a workshop site:

```json
{
  "template": "production",
  "quotas": [{"item": "BED", "minimum": 4}],
  "prerequisites": {
    "assign_labor": true,
    "workshops": [
      {"building": "workshop:Carpenters", "location": [2, 1, 10]}
    ]
  }
}
```

This location is illustrative: the actual source must establish an unoccupied,
supported floor and the complete required hazard halo. Enabling a labor requires
an explicitly observed disabled value so its inverse is known. The resulting
setup actions need their own ConfigureLabor, Construct and applicable checkpoint
authority; the request itself grants none.

Several furniture outputs reuse one generated workshop or labor-enable action.
Joint staffing compares at most three workshop/labor families across the six
outputs: carpentry, brewing and cooking. It maximizes distinct eligible citizens
across those capacity slots, then minimizes labor enables, military exposure,
and canonical identity ties. All four serialized furniture outputs reuse the
same selected carpenter, so extra output types do not enable extra citizens.
A bounded shortlist keeps the exact search independent of fortress population.
Shared selected workers and workshops create ordinary completion dependencies.
The compiler selects one canonical workshop per recipe family, so it
conservatively serializes that family's outputs even if more completed
workshops could support additional parallelism.

## Persistence and limits

Manufactured item identity and stock live in the canonical world, so the normal
snapshot, checkpoint and durable-world machinery retains them. Saved original
requests preserve their complete quotas and compiler identity. Replaying a
forecast does not alter the real world; reopening a durable world does not
create another set of outputs.

This feature manufactures loose inventory. Existing laboratory furniture
construction remains an abstract construction model and does not select or
consume these item records. Native item allocation still requires independently
qualified native observations; a laboratory item cannot authorize native
placement. Room assignment, native hauling and real construction completion
are separate boundaries.

Ownership: `df-lab-conditional-production-c4p` and
`df-dfhack-bridge-plane-c-pic.4` track the broader production and execution work.
