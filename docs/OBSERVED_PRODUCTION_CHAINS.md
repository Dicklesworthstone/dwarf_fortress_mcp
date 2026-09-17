# Production chains backed by coherent observed stock

The adapter's `operations_analysis::production_chain::plan_production_chain`
connects the joint quota compiler to the same sealed observation, inventory
ancestry index and conservative supply policy used by `inventory_plan`.
It is read-only source-present development functionality, not native recipe
verification, a resource reservation or a commit-compatible plan.

## Evidence and assumptions

Callers declare 1..32 resource definitions. Each has a unique short key, exact
captured item-type keys, and optional subtype/raw material selectors. Every
quota, recipe output and recipe input must name one of these resources. A
resource is a modeled interchangeable stack-unit domain; recipes declare a
single output, positive batch yield, integer inputs, job token and workshop or
furnace type. At most 32 recipes are accepted, with one recipe per output.
Unknown recipe effects, byproducts, reusable containers, substitutions, labor,
travel, workshop capacity and timing are not inferred.

Resource definitions must be disjoint over possible item tuples, not just the
items currently captured. A wildcard WOOD resource and a second resource for
one particular wood material are therefore refused. Use separate non-overlapping
selectors instead. This prevents both direct double counting and aliases that
only become ambiguous after a future capture. Duplicate item-type selectors are
normalized, while duplicate resource keys and duplicate recipe outputs fail.

The complete captured item roster supplies initial stock. Zero-sized stacks and
items excluded by the existing direct/inherited container policy do not become
supply. Attached, forbidden, rotten, trader, in-job, dump, inventory-held and
building-held chains retain the existing exclusion behavior. Each resource
includes its eligible item count and up to eight canonical identity/generation/
revision examples from that same snapshot. Exclusion categories remain disjoint.
No separate operations-only snapshot or new generation map is constructed.

The report separates these observed-policy stock counts from hypothetical
production and consumption. A shortage is conditional on the declared recipe
model and conservative stock subset; it does not prove an actual native job
blocker or a whole-fortress material shortage. A feasible model does not prove
native readiness or successful execution.

## Identity, budgets and refusals

The model digest binds the full anchor, enclosing source digest, both policy
versions, normalized resource selectors, final-stock quotas and every declared
recipe, including job/workshop type and input coefficients. Equivalent input
ordering, duplicate minima and split duplicate recipe inputs normalize to the
same model identity. Source profile changes, generation history and different
recipe semantics do not share that identity.

The adapter checks Query authority over the whole projection, exact anchor,
valid canonical hash, scan allowance, cancellation, expiry and remaining grant
uses through the existing source/context checks. Model validation, inventory
indexing, matching, digest construction and core planning share one explicit
work budget (maximum 10,000,000). Elapsed wall time is checked cooperatively;
the core bounded computation is checked immediately on return, not preempted.
All observed resource totals and modeled quantities must fit u32 or the request
fails rather than clamping supply or inventing a deficit. Inputs, snapshots,
journals, watches, bridge state and authority are not modified.

This adapter API works over the sealed operations, spatial and coherent-citizen
views without collapsing their source identities. Runtime presentation must
still reject fenced live sources and enforce current authority before replaying
historical state. No native method, dependency or admission entry was added.

## Validation status

Seven Rust integration tests use the existing coherent wire fixture to cover
conservative stock accounting, final reserves, source/profile separation,
overlapping aliases over empty captures, undefined model references, canonical
model digests, item-generation reuse, current authority and budget refusals.
They have **not** been compiled or executed in this environment.

An independent Python selector-domain reference passed all **24,336** ordered
pairs of 156 selectors over 54 concrete item shapes. It verified intersection
against direct predicate enumeration, including 20,486 disjoint pairs with no
possible double-counted item. Executed source SHA-256:
`f3d2663e10a21e98fc4ec10a2787cd3bd6254c9e30c59155196f42780b5f4a67`.
This checks the mathematical selector rule, not the Rust implementation,
DFHack, MCP dispatch or whole-repository qualification.

```bash
cargo test --locked -p dfmcp-adapter --test production_chain_analysis_tests
cargo test --locked -p dfmcp-adapter --test production_spatial_analysis_tests
python scripts/test_production_resource_domains.py
```
