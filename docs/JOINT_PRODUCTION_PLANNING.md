# Bounded joint production quota planning

`ProductionLogisticsCompiler::plan_quotas` analyzes simultaneous minimum final
stocks in a caller-declared, single-output recipe model. The existing
`compile_quota_work_orders` now delegates to this bounded solver. The joint
`compile_quotas_work_orders` returns action proposals only when the model has no
raw-resource deficits. This implements a remaining WP-PLN-02 compiler gap; it is
not native Manager integration or permission to perform game effects.

## Shared stock and final quotas

Two independently feasible plans may compete for the same wood, fuel, or bars.
The joint solver aggregates every consumer before crediting initial stock once
and rounding a supplier's batch count. A diamond-shaped dependency shares one
supplier batch and its surplus rather than manufacturing a batch per path.

Quotas mean minimum **final** stock. A charcoal quota of two plus another recipe
consuming one charcoal requires three units in total. Repeating a token's quota
uses the maximum requested minimum, not their sum. Duplicate recipe inputs are
summed with checked arithmetic. Results are independent of quota ordering,
recipe registration order and input ordering. Submitted duplicate input/goal
records still consume validation work.

The report exposes complete per-resource balances, every missing raw-resource
quantity, and hypothetical recipe steps with dependencies on earlier step
indices. Resource rows are token-sorted. Steps reverse the lexicographically
ordered consumer-first Kahn traversal, so suppliers precede consumers with a
fixed tie-break. Balances obey:

```text
initial stock + proposed production + missing units
  = final quota + proposed consumption + surplus
```

A deficit report may retain hypothetical downstream steps for diagnosis, but
`into_work_orders` refuses it rather than returning a partially feasible action
list. Inputs and inventory are never modified by analysis. Action proposals
retain the previous item-count threshold representation; the structural report
DAG is not a proof of native scheduling, readiness or eventual completion.

## Bounds and cycles

Planning is iterative. Each unfinished expansion round activates at least one
previously unused recipe, and complete aggregate demand is recomputed from the
original quotas instead of added to a previous round's totals. Limits apply to
the reachable model, not an unrelated catalog: 64 submitted quotas, 64 inputs per
recipe, 250 active recipes, at most 4,096 resources and 8,192 normalized edges.
Defaults allow 1,024 resources and 1,000,000 abstract work units; the fixed work
ceiling is 10,000,000. Graph visits, edge processing, validation and report
construction consume work. The limits are not hard wall-time preemption.

All model strings contain 1..256 UTF-8 bytes without NUL. Generated order names
reserve their prefix within that limit. Yields and input coefficients must be
positive, and quantity arithmetic must fit u32. Overflow or work/model-limit
exhaustion is an error, never a false shortage or a partial success.

Only recipes needed to cover a shortage introduce dependency edges. Initial stock
can discharge an otherwise cyclic catalog without executing a cycle. An active
cycle is refused as `Conflict`; catalytic or other cyclic bootstrap schedules
are not modeled even when such a schedule might exist. Unused malformed catalog
entries do not invalidate unrelated queries.

## Model and authority boundary

`without_recipes()` constructs a closed custom catalog. The existing default
catalog remains illustrative, including its existing quantities; it is not a
certified database of native recipes. Missing inventory keys mean zero only in
the caller's declared model. Unknown, omitted or redacted game observations must
not be converted to zero stock. Recipes assume additive interchangeable integer
units, one output, and no byproducts, reusable containers, skill, travel, workshop
capacity or timing. Model feasibility establishes none of those omitted facts.

These APIs neither observe a fortress nor issue RPCs, reserve resources, seal an
intent, create obligations, or authorize effects. Consumers must still supply
observed witnesses, current authority, native compatibility, appropriate
pre/postconditions, idempotency and ordinary plan/commit/observe/prove checks.
No native protocol, dependency, top-level MCP tool, compatibility admission or
production runner changed in this core increment.

## Validation

Thirteen new Rust integration test functions cover shared-stock conflicts,
final-stock preservation, shared supplier batches, complete shortages, input
normalization, active/unused cycles, a 250-step chain, independent budgets,
invalid models and checked arithmetic. One compares all 4,096 small DAG/yield/
stock configurations against independent exhaustive batch-vector enumeration.
The two existing single-quota tests remain. These Rust tests have **not been
compiled or executed** here: rustc, Cargo and rustfmt are unavailable.

The independent Python reference passed 4,105 cases, including 4,096 exhaustive
small-model oracle comparisons and ordering checks. Executed source SHA-256:
`46fbd31a44d5e874137c6186e19c2d93d4874fab2d42c0369c8809017ed122f5`.
It does not execute Rust, MCP, native DFHack or a live fortress, and establishes
no Rust or whole-repository qualification.

```bash
cargo test --locked -p dfmcp-intent --test production_planning
cargo test --locked -p dfmcp-intent logistics
python scripts/test_joint_production_reference.py
```
