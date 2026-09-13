# Production inspection and declared inventory allocation

The operations/1.3 development runtime adds two structured `fortress.query`
variants: `production_diagnosis` and `inventory_plan`. They use the existing
coherent jobs/buildings/items observation. They do not refresh the bridge, reserve
inventory, create an executable plan, or grant mutation authority.

Both require whole-projection Query authority, an unfenced source, and the exact
current context anchor. Entity-focused diagnostics do not widen a narrower grant.
The original sixteen query variants and eleven tool names remain unchanged.
Only the operations runtime advertises these two additional variants through
`mode="schema"`; other runtime schemas are not extended.

## Joined production inspection

The convenience `mode="production"` is equivalent to this value for the `query`
argument, with `session_id` supplied separately:

```json
{
  "schema": "dfmcp.query/1",
  "query": {"kind": "production_diagnosis", "limit": 4}
}
```

The result joins each job with its same-observation building holder, actual
attached items, and container ancestry. It reports suspension, lack of an assigned
worker, holder construction stage, native filter indices without indexed
attachments, flagged attached items or containers, shared job-item references,
and zero-sized attached stacks. Multiple attachment roles for the same item do
not multiply the distinct-item count or shared-item diagnosis.

A job or holder focus requires a canonical handle and its expected generation.
For the checked-in operations fixture, this selects job 9:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "production_diagnosis",
    "job": {"entity_id": "9", "generation": 1},
    "include_clear_jobs": true,
    "limit": 2
  }
}
```

Real callers must obtain IDs and generations from the current query result, not
copy fixture handles. `holder` accepts the same handle shape for a building.
Supplying both focus fields takes their intersection. Changed generations refuse
with conflict rather than inspecting a replacement entity.

Findings are **observed conditions, not proven causes**. An unfinished holder can
be normal for its own construction job. A shared attachment is not necessarily a
conflicting reservation. An unassigned job is not necessarily stalled. A native
filter with no indexed attachment is not proof that suitable materials are absent.
Rows explicitly retain `blocker_proven=false` and `job_ready_proven=false`.

By default, jobs with no listed finding are omitted from detail rows, but remain
in `summary.jobs_considered`. They are not labeled ready. `include_clear_jobs`
includes them without inventing a favorable assessment.

Inspection order is deterministic: removed, rotten, forbidden, suspended,
under-construction holder, then native job ID. This is not a probability, risk
score, execution priority, or timing estimate. At most eight affected-item handles
are included per row, with an explicit example-truncation flag and the total count.
A structured traversal template supports further inspection of the actual edges.

## Declared stack-unit allocation

The inventory query answers a narrower, useful question: can this observed supply
subset jointly satisfy **these explicitly declared interchangeable stack-unit
requests**, without counting one stack twice?

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "inventory_plan",
    "quantity_unit": "stack_units",
    "demands": [
      {"key": "bars-only", "units": 4, "item_types": ["BAR"]},
      {"key": "flexible-input", "units": 4, "item_types": ["BAR", "BOULDER"]}
    ],
    "limit": 4
  }
}
```

Each demand has a unique ASCII key, positive exact u64 unit count, and one to
eight exact item-type keys. Optional `subtype`, `material_type`, and
`material_index` filters use the observed raw integer identifiers. An explicit
material index requires an explicit material type. There is no material-name
lookup or inferred DF job requirement. Inspect actual item fields before choosing
selectors; the example declares a model, not a native recipe.

The fixed `conservative-unattached-stack-units/1` policy excludes zero-sized
stacks and any item whose own or container-ancestor state is forbidden, removed,
rotten, trader-held, marked for dumping, in a job, in unit inventory, or in a
building. Actual job attachments and explicit building-holder references also
exclude supply, even when the corresponding raw flag is unset. Each excluded
item receives one primary policy reason, so exclusion-category counts are disjoint.
Unmatched otherwise-admitted items are counted separately.

Passing this policy **does not establish accessible or usable inventory**. It
also deliberately excludes some objects that may be usable for a particular game
operation. Therefore a model shortage is a shortage in this selected subset, not
proof that the whole fortress lacks a material.

Equal-eligibility supplies are grouped, solved as an integral capacitated flow,
then expanded into deterministic assignments to actual item handles. Residual
rerouting avoids the failures of greedy demand-by-demand consumption. Assignment
rows are ordered by normalized demand key, then canonical item ID. The objective
is maximum total allocated units, not a user-priority or game-time optimization.
Demand order and duplicate/reordered item-type selectors normalize identically.

The certificate checks that allocated flow equals cut capacity. When the model
is deficient, it additionally identifies a subset of demands whose combined
required units exceed all eligible supply for that subset. For example, two
four-unit demands sharing only five admissible units have a joint deficit of
three; two separate five-unit availability answers cannot be added together.
A certificate applies to the declared selectors and supply policy only.

Every response includes `reservation_created=false`, `commit_compatible=false`,
and `mutation_authority=false`. No prepared-plan ID, prepare receipt, dispatch
capability, or reservation is created. Full native requirement filters, dimensions,
quality constraints, path access, labor eligibility and successful completion
remain outside this implementation.

## Paging, identity, and budgets

Both queries paginate complete detail rows inside the byte budget left after the
required Agent Turn and active watches are reserved. Summaries and certificates
refer to the whole computed result, even when assignment/diagnosis rows span pages.
An oversized summary or single row refuses explicitly; it never emits an empty
non-progressing page or removes required warnings.

Treat continuations as opaque. They bind the exact session, fortress, epoch,
sequence, game tick, state hash, source digest, analysis policy and normalized
query. Resume before refreshing the observation. Page width, output budget and
work ceiling may change; selectors and source state may not. The digest detects
misuse and edits, but is not authentication. Every page rechecks Query authority.
Pages are computed again from the immutable current source, not a retained cache.

The page limit is 1..128 (default 8). Material demands are bounded to 32, supplies
to 32,768, distinct eligibility groups to 1,024, directed residual arcs to 65,536,
and expanded assignments to 65,536. The explicit `max_work` ceiling is at most
10,000,000. Exhaustion returns no partial allocation or misleading shortage.
Container ancestry is iterative and memoized once per request, avoiding repeated
walks for every demand or job attachment.

Work units count explicit algorithm operations. They are not a measured wall-time
or every-comparison guarantee; existing canonical hash validation, source encoding,
ordered-container operations, sorting and JSON rendering have their own bounded
input sizes. No acquisition roster limits are raised by these queries.

## Evidence

Twenty-two Rust tests are registered: seven standalone allocator tests, eight
operations-model tests, and seven actual MCP-handler scenarios. They include
exhaustive small-model enumeration, residual rerouting, integral expansion,
shortage certificates, attachment deduplication, ancestry exclusions, exact raw
selectors, generation/scope/authority checks, source failure, cursor isolation,
schema isolation, and an 8,192-byte pagination path retaining an active watch.

These Rust tests have **not been compiled or executed in this editing environment**,
which has no Rust compiler, Cargo or rustfmt. No Clippy, stdio, native DFHack,
live-game, full repository qualification, or production admission is claimed.
The std-only allocator can separately be tested with:

```bash
rustc --edition=2024 --test crates/dfmcp-world/src/inventory_allocation.rs -o /tmp/dfmcp-allocation-tests
/tmp/dfmcp-allocation-tests
```

An independent Python algorithm-design check passed 1,568 exhaustive cases and
1,000 seeded cases, comparing against an independent allocation enumeration and
checking flow/cut/deficiency equalities. This was **not execution of the Rust**.
The schema extension passed 90 Python JSON Schema checks (38 accepted, 52 rejected),
assembled with the existing canonical entity-ID definition. The three JSON
examples above also validated against that extension assembly.

The native wire, producers, runtime admission, dependency pins, and migration bead
are unchanged. Raw observation acquisition, large-fortress paging, native material
requirements, map access, durable supervision, and live effects remain separate work.
