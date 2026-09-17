# Quality-aware workforce allocation

The standard-library-only `dfmcp_world::workforce_allocation` module supplies
bounded integer minimum-cost maximum flow for one-worker-per-citizen allocation.
It is integrated into the spatial/1.8 `workforce_plan` read query through
`dfmcp_adapter::workforce_analysis::quality`. This is source-present, unadmitted
analysis, not a qualified runtime or a native labor allocator.

## Opt-in query

Send this envelope through `fortress.query` in a spatial/1.8 development session,
using observed skill keys and candidate target tiles from its captured region:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "workforce_plan",
    "objective": "priority_skill_distance",
    "demands": [
      {"key":"carpentry","workers":2,"target":[0,0,5],"skill_key":"CARPENTRY","priority":10},
      {"key":"mining","workers":1,"target":[1,0,5],"skill_key":"MINING","priority":100}
    ],
    "limit": 16,
    "max_work": 1000000
  }
}
```

Coordinates and skill keys above are illustrative, not assertions about a live
fortress. Priority defaults to zero and is bounded to 0..1000. A non-null priority
requires this objective; the legacy objective refuses it rather than ignoring it.
Absent/null/explicit `max_filled_slots` retains the old allocator, model identity,
summary shape and `wp1` continuations. Candidate query `wc2` is unchanged.
Quality-aware requests use `wq1`: source anchor, session, normalized demands,
objective, priorities, policy and work allowance are bound to pagination. Changing
only page size, demand input order, or omitted priority to explicit zero preserves
the model. Changing the objective or priorities cannot reuse an old continuation.

## Model and objective

The adapter uses the existing coherent observation, authorization, observed skill
minima, job-availability policy and conservative terrain approach filters. Already-
filtered candidates bind worker ID and demand index to effective skill, nominal
skill and modeled travel steps. First maximize filled slots, then lexicographically
maximize summed priority, effective skill and nominal skill, then minimize summed
steps. A priority contributes once per filled slot, not once per fully staffed role.
This is not the all-or-nothing production-portfolio objective.

This is a global allocation: residual rerouting can move a flexible worker to leave
another role for its only specialist. One person cannot fill two simultaneous slots.
Checked four-component integer costs avoid floating point and big scalar weights.
Equal shortest-path labels retain the first parent in canonical demand-key/worker-
ID/arc order. No random iteration or hidden seed is used. Inputs must be canonical
and duplicate-free; malformed models fail.

A forward eligibility capacity is the total request, a proven finite upper bound
on possible flow. Worker-to-sink capacity is one. This preserves the Hall shortage
interpretation while permitting vector costs for each individual worker/role edge.

## Verification and limits

Every result is independently rechecked by rebuilding residual capacities from its
assignments. A residual-closed source cut must equal assigned headcount, and every
positive residual edge must have nonnegative lexicographic reduced cost under the
provided node potentials. Together these certify cardinality and quality only for
the declared model. A deficient-demand witness counts distinct eligible workers;
it does not establish a fortress-wide labor shortage or additive independent gaps.

The query returns achieved objective totals and a certificate digest binding the
model, assignments, costs, node potentials and source cut. Complete witnesses remain
available through the Rust allocation result and its `verify` function; they are not
expanded into every MCP page. Every page retains the complete allocation summary,
whole candidate/route rows and current-session active-work projection. A summary
or a single whole row that cannot fit is refused, never silently truncated.

Limits: 4096 distinct candidate workers, 16 demands, 128 total slots, 65536 candidate
edges and 10000000 logical work units. Successive shortest paths take at most 128
augmentations; storage is O(V+E), search is O(F E log V). Logical work counts cover
model visits, residual scans, node initialization/updates, augmentations and the
independent verification pass; ordered-container comparison costs are not individual
work units. The adapter shares one work allowance across candidate analysis,
canonicalization, allocation, verification and output conversion. It checks elapsed
wall time cooperatively, including after the pure solver, not via hard preemption.
Exhaustion or arithmetic/certificate disagreement returns no allocation.

No job is assigned, labor setting changed, reservation created, game time advanced
or fresh native capture requested. Modeled steps are not native navigation proof;
skill and job availability do not prove native labor eligibility, safety or terminal
production. Portfolio defaults, action families, dependency pins, bridge generations,
compatibility admission and production dispatch remain unchanged.

## Evidence

Eight standalone Rust solver tests are registered, including 4096 exhaustive small-
graph/priority comparisons, rerouting, exact objective precedence, empty and multi-
slot models, Hall deficiencies, deterministic budget boundaries and certificate
mutation tests. Six additional registered-handler tests cover opt-in skill/priority
selection, legacy preservation, route drill-down, reordered demands, byte-bounded
pagination with active watches, stale/session/objective/priority continuation
refusal, schema discovery, invalid input, cancellation and authority fencing.

They have not been compiled or run here: Rust/Cargo/rustfmt are unavailable.
`python scripts/test_workforce_quality_reference.py` passed 4608 exhaustive-oracle
comparisons, 4096 deterministic reruns and three certificate refusals. It executes
an independent Python mathematical reference, not the Rust implementation.
`python scripts/test_workforce_quality_schema.py` passed 362 JSON Schema cases;
it does not execute the Rust parser or MCP handler. No Rust/native/live/workspace
qualification or admission is implied.
