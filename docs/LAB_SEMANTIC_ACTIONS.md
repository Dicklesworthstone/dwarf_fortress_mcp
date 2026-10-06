# Semantic actions in the deterministic laboratory

The process-local laboratory (`dwarf-fortress-mcp`, `run_stdio`) executes every
semantic action family except extensions, through the same eleven tools. It is
a deterministic simulator for developing and evaluating agents and for testing
the plan → commit → wait → prove loop. **It is not evidence about Dwarf
Fortress or DFHack behaviour**; progress rates are a laboratory calibration.

## One definition of what an action means

`dfmcp_intent::effects` is the single reference model of each action:

| Action | Effect on canonical state | Default proof | Temporal |
|---|---|---|---|
| `pause` | fortress pause flag | `Paused(p)` | no |
| `designate_dig` | dig-designation entity; tiles converted over time in z/y/x order | `RegionTerrain{area, target tile}` | yes |
| `build` | building entity, `planned → under_construction → complete`; footprint must be observed floor | `construction_stage == complete` on the created entity | yes |
| `create_work_order` | work-order entity counting `amount_remaining` down | `amount_remaining == 0` | yes |
| `set_labor` | `labor.<LABOR>` on each unit | field equality per unit | no |
| `set_burrow_membership` | `burrow.<id>` on each unit | field equality per unit | no |
| `assign_squad` | `squad` on each unit | field equality per unit | no |
| `configure_stockpile` | `accepts`, optional bin/barrel/wheelbarrow limits | `accepts` equality | no |
| `set_standing_order` | `standing_order.<key>` on the fortress settings entity | field equality | no |

Entities an action creates get an identity derived only from the step's
idempotency key (`effects::created_entity_id`), so the sealed plan names them
before dispatch and a retry cannot create a duplicate.

The planner fills omitted postconditions, a bounded obligation for temporal
work (deadline scaled to the work, after every prerequisite's deadline, capped
at one game year), and an exact inverse compensation where the action fully
determines it (labor, burrow membership, pause). Explicit caller choices win;
everything is sealed into the plan digest.

## Agent workflow

```text
fortress_open_session(scenario="starter_fortress",
    requested_capabilities=[["observe","read_only"],["query","read_only"],
      ["plan","reversible"],["control_clock","reversible"],["checkpoint","guarded"],
      ["designate","guarded"],["construct","guarded"],
      ["configure_labor","reversible"],["configure_production","reversible"]],
    max_game_ticks=2000, paused=false)
fortress_query(mode='{"mode":"entities","kind":"unit"}')
fortress_query(mode='{"mode":"terrain","min":[0,0,10],"max":[9,5,10]}')
fortress_plan(actions='[
  {"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},
  {"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},
  {"action":{"kind":"create_work_order","name":"brew","job_token":"BREW_DRINK","amount":2},"depends_on":[1]},
  {"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":true}}]')
fortress_commit(plan_digest=...)
fortress_wait(max_game_ticks=100)   # repeat until every plan_actions entry is Verified
```

`fortress_plan` returns each step's capability, risk, created entity, sealed
postconditions and obligation (terminal, deadline). Steps whose dependencies are
not yet verified stay `Prepared` and are dispatched by a later `fortress_wait`.
`fortress_wait` lets time pass only while the fortress is unpaused and within
the session's game-tick budget; a paused fortress reports `blocked`.

Commit authority is the plan's own capability set: a session that did not
negotiate `designate` cannot commit an excavation, and an idempotent replay
re-checks that authority. Effect capabilities are never granted by default.

The `starter_fortress` scenario is a 48x48 rock level at z=10 with a carved
10x3 hall at (0..9, 0..2), seven dwarves (entity IDs 1001–1007), a stockpile
(2001), a burrow (3001) and a squad (4001).

## In-process dispatcher

`dfmcp_adapter::dispatcher::MutationDispatcher` executes the same reference
semantics for embedding and tests. Temporal steps are recorded as
`AppliedAwaitingVerification`; `reconcile(plan, snapshot, ctx)` proves or fails
them against whatever later observation the caller supplies (it never
simulates time) and then dispatches newly unblocked steps.
