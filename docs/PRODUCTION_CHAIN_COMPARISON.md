# Comparing production-chain alternatives at one observation

`production_chain_compare` is now an agent-facing subquery of the existing
spatial/1.8 `fortress.query` runtime. It compares 2..8 mutually exclusive recipe
models against one coherent conservative inventory and common minimum-final-stock
quotas. It uses the same compiler as `production_chain`; it does not combine the
models, dispatch work, create reservations or select an executable plan.

## One stock and quota universe

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "production_chain_compare",
    "quantity_unit": "stack_units",
    "resources": [
      {"key": "raw", "item_types": ["item_type_3"]},
      {"key": "product", "item_types": ["item_type_999"]}
    ],
    "quotas": [
      {"resource": "raw", "minimum_stock": 4},
      {"resource": "product", "minimum_stock": 4}
    ],
    "candidates": [
      {"key": "one-output", "recipes": [
        {"output": "product", "output_batch_size": 1,
         "inputs": [{"resource": "raw", "units": 1}],
         "job_token": "DeclaredProduct",
         "workshop": {"kind": "workshop", "type_key": "DeclaredWorkshop"}}
      ]},
      {"key": "two-output", "recipes": [
        {"output": "product", "output_batch_size": 2,
         "inputs": [{"resource": "raw", "units": 1}],
         "job_token": "DeclaredProduct",
         "workshop": {"kind": "workshop", "type_key": "DeclaredWorkshop"}}
      ]}
    ],
    "limit": 8
  }
}
```

The names and transformations above are illustrative assumptions, not DF recipe
facts. With seven observed raw units and zero products, the first alternative
requires four raw units for production and four to remain in reserve, so it has a
one-unit raw deficit. The second requires two raw units for production and can
retain the four-unit reserve. Feasibility remains conditional on declared yields,
inputs and the conservative stock policy; the observed game may not support either
recipe. Unmodeled labor, native ordering, time, capacity, routes and byproducts are
not silently supplied.

Every alternative has a unique 1..64-byte key and at most 32 recipes. Resource
definitions, final quotas, captured stock, source identity and current authority
are shared; per-candidate stock, quota, resource and budget overrides are refused.
Resource definitions retain the single-chain disjoint-selector requirement.

## Resource-by-resource deficit frontier

The comparison computes a complete deficit vector in the common sorted resource
order. A candidate is dominated only when another has no larger missing quantity
in any resource and has a strictly smaller missing quantity in at least one.
The nondominated candidates form `summary.deficit_frontier`. Equal vectors and
incomparable trade-offs both remain. All feasible candidates have zero deficits,
so they remain tied on this criterion even when their consumption or batch counts
differ. There is no arbitrary winner or scalar sum across unlike resources.

Each row reports the candidate key, exact source-bound model digest, feasibility,
frontier membership, all dominating candidate keys, per-resource missing/consumed/
planned quantities, modeled steps/batches and planner work units. Batch counts are
model structure, not native labor time or cost. `global_optimum_proven` and
`native_cost_proven` remain false. The frontier concerns only the supplied models
and this deficit criterion; it is not a search over every possible recipe.

Every page retains the full common stock and final quotas, exclusion counts,
feasible candidate keys and complete frontier. The models are alternatives, not
simultaneous consumers. To inspect a candidate's detailed dependency steps, submit
its recipes with the same resources and quotas to `production_chain`, using the
returned anchor as `expected_anchor` and `section="steps"`. The model digest is
identical because both operations use the same normalized compiler.

## All-or-error evaluation and bounded presentation

Candidates are evaluated in key order. Every candidate must pass validation and
complete before any comparison is returned. A malformed, cyclic, overflowing or
budget-exhausted candidate fails the whole request: a partial frontier is never
advertised as complete. The adapter must reproduce identical source, stock and
quota records for all candidates, or comparison fails with an invariant error.

The single `max_work` allowance covers all candidate planning plus deficit lookup
and frontier comparisons, not a fresh allowance per candidate. The hard ceiling
is ten million units. The existing foreground wall-time deadline is shared across
parsing, every candidate and rendering; it is cooperative rather than preemptive.
The request retains the shared 4,096-node, depth-16, 128-KiB shape bounds. All
runtime byte, model, arithmetic and selector checks remain in force.

Pages contain complete candidate rows and the complete comparison summary under
the negotiated byte/token budget, after reserving current Agent Turn and watch
metadata. A summary and one row must fit, or output is refused. `pc1` continuations
bind all candidate keys and normalized model digests, the common source/anchor,
session, comparison policy and shared work allowance. Changing even an off-page
candidate invalidates a cursor. Equivalent declaration order and page-width changes
do not. Single-chain and comparison cursors cannot substitute for one another.

Current Query authority, source fencing and optional observation-journal custody
are checked by the existing spatial production wrapper. No extra native capture,
watch registration, watch sample, checkpoint or game effect is performed. The
comparison is currently exposed in live spatial/1.8 sessions, not the fixed
archive-only or historical-query whitelists. No top-level tool, native protocol,
dependency, compatibility registry or production runner is widened.

## Validation evidence

Ten new Rust regression scenarios are registered: eight comparison tests and two
actual live-handler tests using an injected coherent source. They cover reserve
arithmetic, feasible/equal/incomparable alternatives, all 729 three-dimensional
small-vector dominance pairs, normalized identity, shared work exhaustion,
off-page cursor invalidation, malformed candidates, source/capability refusal,
complete pagination and unchanged existing watch evidence. These tests have not
been compiled or executed here: Rust, Cargo and rustfmt are unavailable.

`python3 scripts/test_production_chain_compare_contract.py` was executed against
the checked-in schema. It accepts 18 valid requests, rejects 40 malformed requests
and verifies seven shared-schema correspondences with `production_chain`. Its
independent integer-lower-box oracle passes 729 dominance pairs, 19,683 ordered
three-candidate frontier cases and five boundary checks. The original single-chain
schema test was rerun and passes 26 accepted / 47 rejected requests. Both scripts
also pass Python bytecode compilation.

These are JSON Schema and independent mathematical-reference checks, not execution
of Rust, the production planner, the MCP runtime, native DFHack or a live fortress.
No Rust, full-repository, native/live or admission qualification is claimed.
