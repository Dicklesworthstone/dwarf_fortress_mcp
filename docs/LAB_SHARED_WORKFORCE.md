# Shared laboratory workforce

`shared-workforce/1` makes excavation, construction and registered production
compete for a finite common set of living workers. It extends the reference
laboratory's causal timeline and existing semantic actions. It adds no native
mutation capability and makes no claim about Dwarf Fortress's actual job AI.

## Use the workforce scenario

Open the existing `fortress.open_session` tool with
`scenario: "starter_fortress_workforce"`. The original `starter_fortress`,
`besieged_fortress` and `empty` scenarios retain their previous semantics.
For a complete room workflow, request the ordinary observation, planning,
clock, designation, construction, production, labor and checkpoint capabilities:

```json
{
  "scenario": "starter_fortress_workforce",
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

The scenario supplies two miners, one constructor, a brewer, a carpenter and a
cook among the starter's seven living dwarves. All five labor columns are
explicit qualified booleans. Existing `connected_room`, `provisioned_room` and
production requests run through this same workforce; no replacement blueprint
grammar or additional tool is required. For a complete connected-room request,
see [LAB_CONNECTED_ROOM_ACCESS.md](LAB_CONNECTED_ROOM_ACCESS.md).

With current Observe authority, session opening returns a bounded `workforce`
report. `fortress.observe` includes it under `world.workforce`; the orientation
briefing copies only a report returned at the current frontier. Reports include
the model, source anchor, eligible workers, active activities, remaining
service, assignments, blockers and explicit omitted-row counts. They are
`certified_derived` within the laboratory evidence scope. They are omitted when
the caller lacks observation authority. An unavailable model reports Unknown
instead of displaying retained values as established current knowledge.

Action `work_state.workforce` reports the selected worker or current blocker for
that exact effect. It filters the complete selection before applying output
bounds, so later work cannot disappear behind the global briefing's row cap.
The existing `work_state` remains the authority for physical quiescence.
Forecasts expose the same selection on the isolated simulated world as
`predicted_workforce`, explicitly marked predicted and bound to its predicted
anchor; they never reserve workers in the source fortress.

If construction is blocked because no living unit has `CONSTRUCTION` enabled,
an ordinary reviewed and authorized `SetLabor` plan can restore that capacity.
Subsequent foreground waits resume the original physical work without granting
service for the time it spent unstaffed. Unknown facts do not authorize implicit
labor changes. If the original obligation has already expired, its historical
failure remains immutable; use the established drain/reconcile/continuation
workflow rather than treating restored capacity as a new success receipt.

## Explicit model identity

The canonical fortress-settings entity has a qualified
`laboratory_workforce_model = "shared-workforce/1"` fact. Each registered
activity created in that world stores
`laboratory_workforce_task_model = "shared-workforce/1"`. Both survive ordinary
snapshot serialization, restoration and recovery with the physical work.

Worlds without this model retain their original behavior and plan semantics.
Existing active legacy work cannot silently acquire staffed history: opting in
beside an unstamped active registered activity refuses. Completed starter
workshops may predate the model. Unsupported, misplaced, unknown or unqualified
model declarations refuse, as does removing the global marker while stamped
task history remains. The model is a scenario/state version, not an arbitrary
standing-order switch.

## Activities and capacity

| Physical work | Required qualified labor | Additional capacity |
|---|---|---|
| `DesignateDig` | `MINING` | One worker per active designation |
| `Build` and exact-item `PlaceFurniture` | `CONSTRUCTION` | One worker per active building |
| `BREW_DRINK` | `BREW` | One completed Still per concurrent order |
| `PREPARE_MEAL` / `COOK_MEAL` | `COOK` | One completed Kitchen per concurrent order |
| Registered wooden furniture recipes | `CARPENTRY` | One completed Carpenters workshop per concurrent order, plus the existing material/item requirements |

Every worker needs qualified `alive = true` and the exact `labor.<TOKEN> = true`
fact. Missing, unknown, false or unqualified facts provide no capacity. A worker
can serve at most one activity at a time, including when several labors are
enabled. Unregistered work-order recipes keep their existing abstract timer
semantics; the workforce model does not turn them into material production.

At each causal interval the engine computes a deterministic maximum-cardinality
matching. It prefers activities with earned partial service, then canonical
entity identity, while moving flexible workers to leave specialists useful.
Completed matching workshops cap the relevant production family. Conditions and
materials still gate production before worker assignment.

Only selected work earns service. Mining retains its existing ten ticks per
tile, construction its 500 ticks per building, and registered production its
50 ticks per unit. An unstaffed task cannot bank elapsed time. A workshop that
finishes during an interval becomes production capacity in the next interval;
it does not retroactively staff work. Cancellation releases capacity, and a
worker's death or changed labor flags affects subsequent selection. Existing
exact furniture allocation and cancellation custody continue to apply.

Inspection returns current assignments and blockers from the same selection
algorithm as advancement. These are derived laboratory scheduling decisions,
not persistent reservations or observed native worker assignments. Pending
physical work remains pending when its action proof has failed or timed out;
the existing cancellation/drain rules still govern cleanup.

## Planning against the common queue

`LaboratoryWorkforcePlanning` inspects all current registered activities,
including blocked work, and sums qualified remaining service with checked
arithmetic. Its default deadline floor includes that queue, all newly requested
temporal service, a conservative factor of two, ordinary observation-boundary
allowance and a 100-tick margin. The result is bounded to one game year and 64
new actions. Oversized or malformed inputs refuse instead of saturating into a
misleading feasible horizon.

`StaticPlanner` uses the floor only for omitted obligations, preserves longer
dependency horizons and retains explicit caller deadlines. Each temporal step
also seals the exact workforce model as a precondition. That guard binds the
model version; it does not establish worker availability or guarantee
completion. Staffing, material shortages, external interference and future
authorized observation can still determine whether a goal succeeds.

The workload inspection includes a complete domain and absence reads. A shell
must therefore obtain a reviewed source replay after an anchor change for a
plan marked by `plan_uses_shared_workforce`; the ordinary per-action witness
cannot silently certify that the queue and its sealed deadline remain valid.

The current production compiler uses this same complete horizon while solving
its food/drink consumption reserves. Queued construction can therefore raise
the planning stock target before the plan is sealed. The reserve calculation
and the actual sealed maximum obligation deadline must agree exactly. Actual
future output is never counted as observed inventory. An anchor change requires
an explicitly reviewed replay for workforce plans, including plain excavation
requests that would otherwise have a narrower action witness.

## Boundaries

Selection, inspection and advancement use the existing entity, work and event
budgets. Caller-owned shadows preserve atomic publication on refusal. No new
runtime, background job, dependency or MCP tool is introduced.

This model does not simulate movement, hauling, skill-dependent speed, worker
preferences, equipment, room ownership or native construction behavior.
General `Build` retains its previous material model; actual furniture placement
continues to require and hold its exact produced item. Live qualification and
admission remain separate evidence requirements.
