# Joint worker/material production portfolios

## Implemented source

- Add `production_portfolio` to the existing spatial/1.8 `fortress.query`
  dispatcher and schema discovery, with live, exact-record historical and
  offline archive execution. No additional capture or top-level tool is added.
- Select complete declared tasks against the same worker AND material capacity
  model, instead of intersecting two potentially incompatible partial allocations.
  Each selected task gets every requested input; unselected tasks consume none.
- Support one to eight normalized tasks at one common candidate origin, one to
  four material inputs per task, and at most 128 total worker slots. Preserve
  observed skill/readiness, conservative inherited item/container exclusions,
  bounded terrain approaches and source/generation identity from one capture.
- Retain all validated spatial candidate supplies, including unallocated stocks,
  so an initial full-demand partial flow cannot strand a feasible task subset.
- Search task sets exactly by descending priority sum, descending complete-task
  count and ascending selected-key list. Reuse the existing integral max-flow/
  min-cut implementation for each resource domain. Positive priorities default to
  one, so the default objective maximizes complete tasks rather than filled units.
- Record a checked deficient-subset witness for every higher-ranked rejected
  combination. The selected set has complete allocations in both domains. Search,
  arithmetic or work/time exhaustion returns no partial or heuristic optimum.
- Return concrete worker and stack assignments, task summaries, model identity,
  source anchor, exclusion digest and paginated rejection evidence. Whole-row
  pp1 continuations bind session, full capture, normalized model and work bound.
  Reused workers/stacks cannot be double-counted across selected tasks.
- Preserve full Agent Turn/current-watch budget reservation, Query authority,
  source health and observation-journal custody checks. Analysis does not create
  reservations, assign labor, dispatch effects, change watches or write history.
- Historical worker/material route requests remain pinned to the exact record.
  Offline results retain explicit historical/current-freshness distinctions.
- Validate raw material selector cardinalities before deduplication, and every
  model input before subset search; malformed losing tasks are not ignored.

Usage and limitations: `docs/PRODUCTION_PORTFOLIOS.md`. A common origin is not an
observed workshop capacity proof. Recipes, task dependencies, future outputs,
worker-to-item carrying constraints, native labor eligibility, durations and
actual execution are outside this declared model. Lower-ranked unselected tasks
are not thereby proved individually infeasible. The empty set is explicitly
possible and must not be interpreted as successful production.

Native protocols, journal formats, dependencies, game-effect families, production
runners, compatibility registry and admission permissions are unchanged. The
formal unadmitted phase remains unchanged; this fragment records this increment's
implementation and evidence status.

## Validation

Eighteen new Rust test functions are registered: eight pure selector tests, four
coherent adapter integration tests and six actual MCP-handler/private-file tests.
The selector tests include 4,096 small eligibility graphs against an independent
exhaustive assignment oracle, plus priority ties, multiple required material
inputs, shared stacks, eight-task selection, malformed input and work/overflow
refusal. Adapter/handler tests cover conservative exclusions, coherent source
identity, unchanged watch/journal bytes, full-response pagination, stale cursors,
historical pinned routes, offline recovery and changed journal custody. Existing
archive tests retain all prior read variants and add the portfolio operation.

**No Rust compilation or test execution occurred here.** Rust, Cargo and rustfmt
are unavailable. The 4,096 oracle cases are registered Rust tests, not executed
evidence. No MCP runtime, native/live-game, filesystem-crash or full-repository
qualification is established.

The executed Python request-schema checker passed 104 cases: 27 accepted and 77
rejected. The tested schema Git blob is
`734045687ccd2318e0c1d563ac9978ab79ae46b3`; the tested script Git blob is
`cd81b09b6cff80afb584becce3da1c5b45432dbb`. Both match the committed bytes.
Script SHA-256: `d7d6ca3b0527dded73bbec057223435ebdd7f0b3819f1147d1aa5b50e8df6a33`.
Those checks cover request structure only, not aggregate limits, duplicate keys,
source state, allocation correctness, serialization, archive custody or execution
of any Rust path. Source/diff review and GitHub branch verification also occurred.
