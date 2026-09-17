# Production portfolios with protected stock

The spatial/1.8 `production_portfolio` query now accepts optional `reserves`.
These are hard, distinct stock pools in the same allocation model as the selected
production tasks. Task priority cannot spend them. This extends the existing
query, not `fortress.plan`, and creates no game orders or native reservations.

This is implemented development source, not qualified production functionality.
The new Rust tests have not been compiled or executed in the editing environment.

## Request

Use actual item and skill keys from the capture and an observed candidate origin.
The following coordinates, type and quantities are illustrative:

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "production_portfolio",
      "origin": [0, 0, 5],
      "quantity_unit": "stack_units",
      "tasks": [
        {
          "key": "furniture",
          "priority": 10,
          "workers": 1,
          "skill_key": "CARPENTRY",
          "materials": [
            {"key": "wood", "units": 20, "item_types": ["WOOD"]}
          ]
        }
      ],
      "reserves": [
        {"key": "emergency-wood", "units": 50, "item_types": ["WOOD"]}
      ],
      "limit": 8
    }
  }
}
```

With 60 eligible units, the reserve can be supported but this 20-unit task cannot.
With 70 eligible units and a modeled worker, both can be supported. Increasing
priority never changes that resource constraint.

Omitting `reserves`, specifying null, or specifying an empty array preserves the
original no-reserve model and its identity. Nonempty reserves have their own
policy identity and become part of the normalized request/model digest.

## Pools use distinct units

Each reserve declares `key`, positive `units`, and one to eight interchangeable
`item_types`, with the same optional subtype/material selectors as a task input.
Up to eight reserves are accepted. Task inputs plus reserves must total at most
32 material demands; the existing eight-task and 128-worker-slot limits remain.
Raw selector counts are checked before sorting/deduplication. Reserve keys must
be unique, and reserve/task input keys occupy separate namespaces.

The same quantity cannot support two reserve pools or both a pool and a task.
For example, a 20-unit any-bar pool and a 10-unit steel-bar pool require 30
DISTINCT units. This is intentionally not the semantics of two overlapping count
predicates where ten steel bars could satisfy both thresholds. Use item-quantity
inspection/watches for such measurements; declare reserve pools only when their
distinct-stock interpretation is intended.

No particular stacks are greedily removed before planning. Each candidate task
set is solved jointly with all reserves, allowing the existing residual-flow
algorithm to move reserve support onto another compatible stack. This avoids
stranding a task that needs a scarcer material. Units within a stack remain
interchangeable under the existing stack-unit model; physical split operations
and carrying capacity are not proved.

## Results distinguish consumption, support and infeasibility

`assigned_stack_units` counts only inputs assigned to selected production tasks.
`reserve_constraints.protected_stack_units` counts separately supported reserve
units when the complete constraints are feasible. `reserve_assignment` rows name
the pool, item generation, ground root and exact-anchor route request, and state
`consumed_by_selected_tasks=false` and `reservation_created=false`.

A feasible empty task set may still contain reserve-support rows. This is not
successful production. Reserves consume no workers, earn no task score and are
never silently dropped from that empty set.

When the reserves alone are infeasible, the query reports:

```json
{
  "selected_tasks": 0,
  "selected_set_model_feasible": false,
  "assigned_workers": 0,
  "assigned_stack_units": 0,
  "optimization": {"status": "infeasible_hard_reserves"}
}
```

The single `reserve_shortfall` row contains a checked deficient-subset witness
naming the reserve keys, required units, distinct eligible units and deficit.
Because these demands are mandatory in every candidate, that witness excludes
all task sets, including the empty one. The runtime withholds partial assignment
rows. Per-pool `supported_units` may describe partial diagnostic flow, but
`protected_in_model=false` and total `protected_stack_units=0` prevent it from
being presented as a completed reserve allocation. Query success only means that
the analysis returned; it does not mean the resource constraints are satisfied.

For feasible reserves, the existing task objective and higher-ranked exclusion
witnesses remain. Material cuts can name both task inputs and reserve demands.
One shortage is not an independently additive diagnosis of global resource need.

## Live, historical and offline behavior

The query uses the same coherent capture, eligibility rules and observed terrain
model as the existing workforce and spatial-inventory paths. Forbidden, attached,
held, rotten, inaccessible-in-model or otherwise excluded items do not become
reserve support. Failure to support a pool therefore does not prove global stock
absence or an actual game shortage.

The existing live, exact-record historical and offline archive dispatchers all
use this implementation. Every reserve route drill-down in a historical result
stays pinned to the selected record and digest. Historical support does not
establish current stock. No capture, watch sample, journal write, lease, item lock,
standing order or other game effect is created by this analysis.

Complete response metadata, current watch summaries and reserve summaries are
reserved before whole-row pagination. `pp1` continuations retain their existing
shape. Changing a normalized reserve key, amount or selector changes model
identity and invalidates a continuation. Equivalent ordering, duplicate type
spellings within the raw bound, and omitted versus explicit defaults retain the
same identity. Existing no-reserve cursors are not deliberately invalidated.

## Validation status

Eighteen new Rust test functions are registered: seven public selector tests,
five coherent adapter tests and six actual MCP-handler/private-file scenarios.
They cover joint rerouting, priority refusal, overlapping pools, infeasible and
feasible empty sets, legacy behavior, normalization, malformed inputs, budgets,
current authority, unchanged journals/watches, pagination and historical/offline
route binding. One Rust test enumerates 8,192 small worker/material graphs against
an independent exhaustive assignment oracle. Those oracle cases are UNRUN.

Rust, Cargo and rustfmt are unavailable in the editing environment. No Rust build,
MCP execution, native/live campaign, filesystem-crash qualification or production
admission is established. Source/diff review and GitHub branch verification are
not substitutes for execution.

The executed Python schema checker passes 68 cases (14 accepted, 54 rejected),
validated both directly and inside a query envelope: 136 validation checks. It
also checks that task-input and reserve selector schemas are identical. This is
request-structure evidence only; it does not exercise Rust, allocation, aggregate
limits, duplicate-key refusal, custody, pagination or archive replay.

```bash
python3 scripts/test_production_reserve_contract.py
cargo test --locked -p dfmcp-adapter --test production_reserve_selection_tests
cargo test --locked -p dfmcp-adapter --test production_reserve_portfolio_tests
cargo test --locked -p dfmcp-mcp reserve_tests -- --test-threads=1
```

Native protocols, journal formats, dependencies, top-level tools and production
admission are unchanged. See `PRODUCTION_PORTFOLIOS.md` for the original joint
model and its remaining limits on native eligibility, workshop capacity, task
dependencies, future outputs and execution.
