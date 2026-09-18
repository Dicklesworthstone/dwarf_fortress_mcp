# Multi-stage production chains through MCP

The spatial/1.8 `fortress.query` dispatcher now exposes the observed-stock chain
compiler as `production_chain`. This connects the existing joint-quota and
conservative inventory engines to the actual agent-facing runtime; it does not
introduce another planner, a native recipe catalog or a game-effect path.

## Declare a model

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "production_chain",
    "quantity_unit": "stack_units",
    "resources": [
      {"key": "raw", "item_types": ["item_type_3"]},
      {"key": "component", "item_types": ["item_type_998"]},
      {"key": "product", "item_types": ["item_type_999"]}
    ],
    "quotas": [
      {"resource": "product", "minimum_stock": 4},
      {"resource": "raw", "minimum_stock": 4}
    ],
    "recipes": [
      {"output": "component", "output_batch_size": 2,
       "inputs": [{"resource": "raw", "units": 2}],
       "job_token": "DeclaredComponent",
       "workshop": {"kind": "workshop", "type_key": "DeclaredWorkshop"}},
      {"output": "product", "output_batch_size": 1,
       "inputs": [{"resource": "component", "units": 1}],
       "job_token": "DeclaredProduct",
       "workshop": {"kind": "furnace", "type_key": "DeclaredFurnace"}}
    ],
    "section": "all",
    "limit": 8
  }
}
```

These are illustrative model keys, not certified DF recipes or item mappings.
Use the exact item-type keys and raw selectors from the captured inventory and
explicitly supply the transformations to investigate. Resource domains must be
pairwise disjoint, including wildcard intersections over possible future items.
Every input, output and quota must name a resource. The adapter normalizes duplicate
minima and duplicate ingredients, but rejects duplicate outputs and resource keys.

Quotas describe minimum *final* stock, not gross production. With seven raw units,
no components/products and the model above, producing four products consumes four
raw units, leaving only three. The result reports a one-unit modeled raw deficit;
it does not credit the same stock to both production and the four-unit reserve.

## Inspect the complete result

`section` selects `all`, `resources`, `recipes`, `steps` or `shortages`.
All-section rows are ordered as resources by key, normalized recipes by output,
steps in dependency order, then shortages by resource. Step dependencies identify
earlier step indices and their outputs, independent of page boundaries.

Resource rows separate observed eligible stock and generation/revision-bearing item
examples from hypothetical consumption, production, reserve and surplus balances.
An unused resource has no modeled balance. Recipe rows expose the normalized
assumptions, including job/workshop labels. Shortage rows report every modeled raw
shortfall. Empty shortage rows do not assert that native production is ready.

The complete summary on every page retains counts and disjoint exclusion categories.
`model_feasible` means the declared model can meet the quotas using conservative
stock and modeled production. `observed_quotas_met` asks whether the initial observed
stock already meets every quota, without counting future production. These are
separate: a feasible production plan can still have no observed finished products.

All results preserve the exact enclosing source identity. They create no prepared
plan, work order, reservation or mutation and prove neither native recipe semantics,
workshop/labor/path eligibility, game-job completion nor a global material shortage.
See `OBSERVED_PRODUCTION_CHAINS.md` for the underlying model and supply policy.

## Bounds and lifecycle

The closed schema is `schemas/mcp_production_chain_v1.json`: 32 resources, 32 recipes,
64 quotas, 32 ingredients per recipe, bounded strings and u32 quantities. The full
request additionally has a 4,096-node, depth-16 and 128-KiB shape allowance. Runtime
UTF-8 byte bounds, reference checks, disjointness and checked arithmetic remain
mandatory even when the JSON Schema accepts a request.

`max_work` is shared across the adapter's inventory and core planner, at most ten
million units. Parsing, analysis and rendering share the caller's cooperative
wall-time allowance. The existing spatial wrapper reserves the complete Agent Turn
and active-watch metadata before whole-row pagination. A complete summary plus one
row must fit or the request fails; empty result summaries are bounded too.

`pc1` continuations bind session, full anchor, source-bound normalized model,
section, query policy and work allowance. Page width may change, but changed
captures or model semantics require restarting. Equivalent normalized declarations
retain model identity. An optional `expected_anchor` rejects stale requests.

This initial dispatcher increment supports current coherent spatial/1.8 sessions.
It does not widen archive whitelists. A fenced live source is refused. Querying
neither acquires another native capture nor registers, evaluates or checkpoints a
watch. Existing current-authority and optional observation-journal custody checks
remain in the wrapper. No top-level MCP tool, dependency, bridge wire format,
production runner or compatibility entry is added.

## Evidence

Eleven Rust tests are added: eight query/serialization/pagination tests and three
actual spatial-handler scenarios with an injected coherent source. They cover joint
reserves, two-stage dependencies, explicit shortages, canonical models, stale
continuations, malformed/overlapping models, current authority and budgets,
unchanged active-watch evidence and source fencing. They have not been compiled
or executed here: `rustc`, Cargo and rustfmt are unavailable.

`python3 scripts/test_production_chain_contract.py` was executed successfully:
26 accepted and 47 rejected JSON Schema cases. This exercises the checked-in
request schema, not the Rust compiler, planner, MCP runtime, native DFHack or
live-game qualification. No full-repository or production qualification is claimed.
