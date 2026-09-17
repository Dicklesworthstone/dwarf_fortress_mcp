# Production portfolios across separate sites

The existing spatial/1.8 `production_portfolio` query now accepts an optional
`origin` on each task. Workers and materials must have candidate routes at that
task's own site. All tasks still share one global worker-capacity model and one
global finite-stack model; independently feasible local plans are not combined
as though they owned separate copies of the resources.

This is unadmitted development source. Rust compilation and execution have not
been established for this increment. Native protocols, observed field formats,
journal formats, dependencies and game-effect authority are unchanged.

## Request

Use the existing `fortress.query` tool. Coordinates and keys below are
illustrative; every origin must be an observed candidate tile in the session's
captured region, and skill/item keys must match the capture.

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
          "key": "west-work",
          "priority": 1,
          "workers": 1,
          "skill_key": "CARPENTRY",
          "materials": [{"key": "wood", "units": 1, "item_types": ["WOOD"]}]
        },
        {
          "key": "east-work",
          "origin": [4, 0, 5],
          "priority": 2,
          "workers": 1,
          "skill_key": "CARPENTRY",
          "materials": [{"key": "wood", "units": 1, "item_types": ["WOOD"]}]
        }
      ],
      "reserves": [{"key": "buffer", "units": 1, "item_types": ["WOOD"]}],
      "limit": 8
    }
  }
}
```

The top-level origin is the default for tasks without an override and the origin
for ALL reserve pools. Omitted or null task origins inherit that default. A task
origin cannot move reserves, widen the captured region, select a bridge method,
identify an occupied workshop as a walkable tile, or grant execution authority.
Even a task that will lose on priority must have a valid origin. Hidden, liquid,
occupied, excluded or outside-region task sites are refused under the existing
candidate-route policy rather than ignored during optimization.

## Joint spatial model

The workforce analyzer already supports target-specific candidate sets. This
path supplies each task's actual origin to that analyzer, retaining its observed
skill/readiness filters, generation checks and restricted occupied-endpoint rule.
Each citizen contributes exactly one worker slot across all selected tasks.

Material analysis reuses the existing conservative item/container policy at each
distinct origin. Complete candidate supply pools are retained, including stacks
not used by the local full-demand partial allocation. Each local eligibility
mask is restricted to the demands that actually belong to that site. Those masks
are then joined by stable item identity while checking capacity agreement:

- different sites can use different stacks in disconnected regions;
- one stack visible from multiple sites retains its original capacity, not their sum;
- stock at a disconnected site cannot satisfy another site's task or default-site reserve.

The existing exact task-subset selector consumes those global models. Selected
tasks receive all worker and material inputs; reserves remain hard, distinct,
unconsumed stock constraints. Priority and complete-task tie-breaking remain the
same. A reserve-only shortfall still excludes every task set, including empty
production. A rejection describes site-dependent modeled scarcity, not global
absence or proof of native inaccessibility.

## Responses and historical inspection

Multisite task summaries and assignment rows include their resolved `origin`.
Worker routes start at the task's site and end at its candidate approach tile.
Material and reserve routes start at the owning demand's site and end at the
observed ground-root location. Reported step counts come from that site's
analysis, never from the default site's distance to the same item.

`supply_model.candidate_stacks` counts distinct globally eligible stacks.
Multisite `supply_model.sites` adds per-site candidate counts and bounded
reachability evidence. Per-site counts overlap and are explicitly NOT additive;
the legacy item-classification/reachable-tile fields describe the default site.
Rejected-combination rows include origins aligned to the deficient demand keys.

Live, exact-record historical and offline archive paths use the same query.
Historical route drill-downs retain the original record number and digest as
well as the correct site. These reads do not replace current state, sample
watches, write either journal, or acquire a second native capture. The enclosing
runtime retains its authority, source-health, archive-custody and complete
Agent Turn/current-watch output checks.

Nondefault origins enter the normalized model digest under
`joint-complete-tasks-location-bound-shared-capacity/1`. Changing a task's site
invalidates its old continuation even when both sites are mutually reachable.
The `pp1` format remains. Omitted/null/explicit-default origins preserve the
common-origin request identity. The existing typed `plan` and
`plan_with_reserves` entry points remain wrappers; `plan_at_sites` accepts a
bounded map from task key to origin without changing `ProductionTask` literals.

## Bounds and nonclaims

One request still permits 1..8 tasks, at most 128 worker slots, at most eight
reserve pools, and 32 combined material demands. There are at most nine distinct
inventory analyses: the default plus eight task overrides. Repeated origins
reuse one site report. Site scans, resource matching, subset search and final
rendering remain bounded by the enclosing shared cooperative work/time limits.
The combined candidate pool cannot exceed the allocator's 32,768-stack bound.
A bound failure returns no partial plan or guessed optimum.

This first implementation retains separate bounded site reports and may repeat
material scans for different origins. It makes no speedup, benchmark or measured
peak-memory claim. It does not combine disjoint captures into a bigger map.

The result is not a native workshop-capacity proof, reservation, job assignment,
transport schedule, worker-to-item carrying assignment, task dependency plan,
or prediction of future outputs. An occupied workshop still needs an explicit
candidate working/delivery tile. No new live mutation family is enabled.

## Validation

Seventeen new Rust test functions are registered: two pool-merging tests, eight
coherent adapter tests and seven actual MCP-handler/private-file scenarios. One
adapter test enumerates 256 small site/resource/reserve cases against independent
capacity arithmetic. Coverage includes disconnected sites, common supply,
default-site reserve scarcity, exact route witnesses, normalized defaults,
pagination, changed locations, authority/output refusal, unchanged journal and
watch bytes, historical reads and offline schema discovery.

NONE of these Rust tests has been compiled or executed here. Rust, Cargo and
rustfmt are unavailable, and direct container DNS access to obtain a toolchain
or checkout failed. Focused commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-adapter --test production_sites_tests
cargo test --locked -p dfmcp-adapter 'portfolio::sites::tests'
cargo test --locked -p dfmcp-mcp 'production::site_tests' -- --test-threads=1
python scripts/test_production_sites_contract.py
```

The executed request-schema checker passed 60 cases (12 accepted, 48 rejected),
each directly and inside a query envelope: 120 checks. It also confirmed that
task-material and reserve schemas remain identical. Tested schema Git blob:
`6d39942f06a852ca734f2a43f4509a8a9b45a5f5`; tested script Git blob:
`b47b09dd7cb01dd2371856d85094329a2a42abca`. Both match committed bytes.
These checks establish request structure only, not Rust, route feasibility,
resource allocation, MCP execution, archive replay, native/live DFHack,
filesystem crash durability or production admission.
