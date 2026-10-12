# Query observed staffing, then review an executable proposal

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The existing laboratory `fortress.query` accepts `mode="labor_allocation"` in its
JSON mode string. It derives the initial labor matrix from the selected canonical
world version; callers supply only selected citizens and allocation preferences.
It never writes state, acquires mutation authority or creates a pending plan.
No new top-level tool, native endpoint or compatibility admission is added.

## Example

Use `schemas/examples/labor_analysis_query_v1.json` as the query's mode string:

```json
{"mode":"labor_allocation","roster":[
  {"id":"1001","skills":{"MINING":20,"BREW":10},"stress_level":100},
  {"id":"1002","skills":{"BREW":9},"stress_level":100}],
 "demands":[
  {"labor":"MINING","skill":"MINING","workers":1,"minimum_skill":1,"priority":0},
  {"labor":"BREW","skill":"BREW","workers":1,"minimum_skill":1,"priority":1}]}
```

Skills, skill mappings and stress remain caller-declared preferences, NOT
observed native facts or calibrated native conversions. `assigned_labors` is
forbidden in the query input. The initial settings and liveness come from the
qualified source. Missing settings remain unknown; the query does not initialize
them. In the starter fixture, the two-citizen example first needs the explicit
reference-only setup in `schemas/examples/labor_execution_setup_lab.json`, through
ordinary authorized plan/commit/wait. No existing scenario's bytes are changed.
The existing brewer's known `labor.BREW` on citizen 1003 can be analyzed without
that setup (one BREW demand with one worker reports already configured).

## Four useful outcomes

| `readiness` | Meaning | Executable handoff |
| --- | --- | --- |
| `missing_evidence` | Named citizen identity/liveness or a managed labor bit is not established. `problems` names required fields; `model_feasible` remains null. | None. No partial roster is used. |
| `staffing_shortage` | The observed roster cannot fill the declared model. `staffing` reports each role's eligibility, assignments and deficit. | None. Partial coverage is not silently executed. |
| `already_configured` | Every managed setting is known and matches the allocator's current target; no change is needed. | None. No fake no-op effect is proposed. |
| `proposal_available` | The current model is feasible and the existing compiler accepted its complete guarded transition. | `proposed_workflow_json`, an ordinary `labor_allocation_v1` request string. |

Pass the exact `proposed_workflow_json` to `fortress.plan.actions`, review the
sealed steps, then explicitly commit the returned plan digest and wait for its
proofs. `proposed_workflow_digest` is SHA-256 of that string for comparison; it
is NOT a commit token. Query output never supplies Plan or ConfigureLabor grants.
Changed initial settings refuse through the existing workflow's guards. Reading
a proposal never cancels or replaces another pending plan.

Re-query after execution for current configuration truth. An old completed action
receipt is not reused as current staffing proof. The result has a separate exact
`source_anchor`, coverage, known enabled/disabled counts, missing-worker totals
and aggregate work. Historical reads retain the existing shell's historical and
current anchor distinction; a historical proposal is not a current-state claim.
Unmanaged labor coverage is positive-only, not a complete native labor census.
Configuration truth does not prove job assignment, output, evacuation or safety.

## Bounds and preservation

The existing 1 KiB query input limit remains. Core bounds are 64 citizens, 16
roles and 64 requested positions. Optional `max_work` is 1..1,000,000 and spans
source inspection, allocation AND the compiler's matching/lowering pass. All
passes use the shrinking remainder. Aggregate retained representation is capped
at 256 KiB, complete reply at 32 KiB and the handoff at the existing 16 KiB action
input limit. Problems show at most 16 entries with explicit total/truncation.
Budget exhaustion returns no partially executable proposal.

The handoff is one scalar JSON string. Output fitting may omit it whole under a
tighter negotiated output budget, but cannot shorten its internal roster into a
different executable subset. A missing string is not permission to reconstruct a
partial request from summary rows. Re-query with a suitable output allowance.
Legacy action arrays, the prior workflow decoders, civilian protection queries,
other query modes and canonical scenario generation retain their implementations.
The existing server authorizes Query and fits output; this projection adds no
ambient context or authority. The core embedding entry has explicit context and
negotiated read-budget checks; the laboratory wrapper uses fixed inner bounds.

## Validation

Ten new Rust tests exercise the real query decoder/compiler, current source
matrix, shortages versus no-ops, deterministic handoffs, exact aggregate work
bounds, strict inputs, atomic output fitting, facade query/plan/commit/wait,
stale input refusal and capability denial. With the preceding nine core tests,
19 Rust cases are present. They remain uncompiled/unexecuted in this environment,
which lacks Rust/Cargo and dependency caches. No Rust, native or live success is
claimed. `python3 scripts/test_labor_analysis_schema.py` passed both schema
metaschemas, the example, 56 malformed structural mutations and 16 fixture
handoffs against the unchanged existing workflow schema. These are structural
checks, not execution of the Rust observation, allocation or MCP implementations.
