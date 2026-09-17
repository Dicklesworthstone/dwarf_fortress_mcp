# Joint production portfolios

`production_portfolio` is a read-only structured query in the spatial/1.8 server.
It chooses a set of declared tasks that can receive **all requested workers and
all requested material inputs together**, using one coherent citizen/operations/
terrain capture. No task consumes modeled capacity unless all its inputs fit.

This is implemented, unadmitted development source. Rust compilation and runtime
execution have not been established for this increment. It does not add a native
method, game effect, dependency, production runner or admission permission.

## Why a joint model is needed

Separate maximum-flow calls maximize individual worker slots or stack units, not
complete production tasks. A worker allocation can favor one task while the
material allocation favors another. Intersecting those two partial results can
miss a third task that both domains could support with different assignments.

The new planner retains the **complete candidate supply model**, including stocks
not used in the full-demand partial allocation. It selects a single task set,
then requires both resource domains to satisfy that same set completely. Workers
have capacity one across the set, and each observed stack's units are shared
across every selected task and material input without double-counting.

The two resource universes are otherwise independent in this model. There is no
worker-to-item carrying constraint, task dependency, workshop-capacity constraint,
future output credit, duration or resource reservation. Those omissions are why
this is a production allocation analysis, not native job readiness or a schedule.

## Request

Call the existing `fortress.query` tool:

```json
{
  "session_id": "<live spatial or archive session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "production_portfolio",
      "origin": [0, 0, 5],
      "quantity_unit": "stack_units",
      "tasks": [
        {
          "key": "furniture-a",
          "priority": 3,
          "workers": 1,
          "skill_key": "CARPENTRY",
          "materials": [{"key": "wood", "units": 2, "item_types": ["WOOD"]}]
        },
        {
          "key": "furniture-b",
          "priority": 2,
          "workers": 1,
          "skill_key": "CARPENTRY",
          "materials": [{"key": "wood", "units": 1, "item_types": ["WOOD"]}]
        }
      ],
      "limit": 8
    }
  }
}
```

The coordinates and keys are illustrative. Use skill and item type keys actually
observed in the session. The example's demands are user declarations, not an
inferred Dwarf Fortress recipe. The origin must be a visible candidate tile in
the captured region; an occupied workshop, wall, hidden cell or liquid is not
made traversable by calling it a delivery point. All tasks share this origin.

A task has a unique ASCII key of at most 32 bytes, a positive priority (default 1),
a worker count, one skill key, and one to four **required** material inputs.
`min_effective_skill` defaults to 1; `preserve_social` and `adults_only` default to
true. Material inputs have task-local unique keys, positive stack-unit quantities,
1..8 interchangeable item types, and optional subtype/material restrictions. A
material index requires a material type. Identical local input keys in different
tasks are permitted and receive deterministic task-scoped internal demand keys.

Tasks and material inputs are normalized into key order. Equivalent input order,
explicit defaults, and permitted duplicate type alternatives do not change the
normalized model. Raw cardinality/string limits are checked before normalization.

## Objective and evidence

The exact finite search orders task sets by:

1. highest sum of declared priority values;
2. highest number of complete tasks;
3. lexicographically smallest sorted selected-task-key list.

Priority is additive utility, not a strict priority tier. Two priority-2 tasks
can beat one priority-3 task. With all priorities equal to one, the objective is
the greatest number of complete tasks, not the greatest number of assigned units.
No global skill-quality, travel-distance or completion-time optimum is claimed.

For each ranked task set, the existing integral allocator checks workers and
materials. The first set with both domains fully satisfied is selected. Every
higher-ranked rejected set retains a checked Hall-deficient demand subset in at
least one resource domain. If no nonempty set fits, all resources remain
unassigned and the empty selected set is reported explicitly.

The response contains selected task keys/mask, selected priority, per-task worker
counts and exclusions, supply classifications, total assigned workers/stack
units, source/model digests, and the exact objective. `selected_set_model_feasible`
is true even for the empty set; inspect `selected_tasks` and
`all_tasks_supported` rather than interpreting it as a successful game outcome.

Rows are emitted in this order: worker assignments, material assignments, then
rejected higher-ranked combinations. Worker and material rows carry generation-
checked entity references and exact-anchor candidate-route requests. Rejection
rows identify the combination, resource domain, deficient demand keys, required
units, eligible units and deficit. They are not independently additive shortages
or claims that excluded resources are globally absent.

The exclusion digest binds the normalized model, selected mask and complete
ordered rejection evidence. `truncated=true` means more assignment/proof rows
remain, not that an approximate optimum was selected. Follow every continuation
to retrieve the complete row evidence. Lower-ranked unselected tasks are not
thereby proved individually impossible.

## Bounds, publication and history

The request permits 1..8 tasks, at most 128 total declared worker slots, and at
most 32 material inputs. It examines at most 255 nonempty task sets. Existing
supply, route, graph, arithmetic and allocation limits still apply. The complete
model is validated before subsets are tried, so malformed losing tasks cannot
hide behind selection. Shared work/time exhaustion returns an error, never a
partially searched optimum. Deadline checks are cooperative around existing
bounded synchronous analysis; they are not hard preemption guarantees.

Pages contain 1..128 whole rows. `pp1` continuations bind the session, full
observation anchor, normalized model and work allowance. Page width and output
allowance may change, but not the model, source capture or `max_work`. Full Agent
Turn and active-watch metadata are reserved before filling pages; insufficient
space produces an explicit refusal rather than empty-progress pagination.

Current queries reuse the production runtime's authority, source-health, journal-
custody and response boundaries. They acquire no native capture, register no
baseline, sample no watch and write no journal. The same operation is permitted
inside `historical_query` and in archive-only sessions. Archived worker/material
routes are pinned to the exact record and digest; they cannot silently select
newer terrain. Current session state and current watch evidence are preserved.
All archive freshness and mutation refusals remain in force.

## Validation status

Eighteen new Rust functions are registered: eight pure selector tests, four
coherent adapter integration tests and six actual spatial-handler/private-file
scenarios. The selector suite includes 4,096 small worker/material eligibility
graphs checked against an independent exhaustive assignment oracle. Additional
coverage includes priority ties, multi-input tasks, scarce shared stacks,
conservative exclusions, source identity, unchanged watches, response limits,
stale continuations, historical routes and offline recovery. Existing archive
schema tests are updated to retain all prior reads and add the new variant.

**None of these Rust tests has been compiled or executed in the editing
environment.** Rust, Cargo and rustfmt are unavailable. Source/diff review is not
a compilation or runtime pass. Reproducible commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-adapter workforce_analysis::portfolio::selection::tests
cargo test --locked -p dfmcp-adapter --test production_portfolio_tests
cargo test --locked -p dfmcp-mcp production::portfolio_tests -- --test-threads=1
cargo test --locked -p dfmcp-mcp archive_schema_advertises -- --test-threads=1
python3 scripts/test_production_portfolio_contract.py
```

The Python request-schema checker passed 104 cases (27 accepted, 77 rejected)
against the exact committed schema bytes. It checks request structure only, not
cross-field aggregate limits, duplicate keys, source identity, route eligibility,
allocation correctness, Rust serialization, MCP execution or archive custody.
No native/live-game, filesystem-crash, full-repository or admission qualification
is established by this increment.
