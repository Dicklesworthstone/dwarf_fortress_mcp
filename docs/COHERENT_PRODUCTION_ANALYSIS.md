# Coherent production analysis

The spatial/1.8 development server now exposes the existing production-diagnosis and conservative
inventory-allocation engines through its actual `fortress.query` dispatcher. These operations were
previously tied to the older operations-only state. They now use the same complete citizen,
operations and terrain capture as workforce, map and ordinary entity queries.

This is source-present, unadmitted development functionality. The Rust changes and regression tests
have not been compiled or executed in the editing environment. No native protocol, dependency,
production-admission entry or game-effect authority is added.

## Inspect observed production conditions

The existing query tool has a convenience mode:

```json
{"session_id":"<spatial session>","mode":"production"}
```

The structured form supports focused diagnosis and pagination:

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "production_diagnosis",
      "include_clear_jobs": false,
      "limit": 8
    }
  }
}
```

Optional `job` and `holder` filters each contain exactly an `entity_id` string and a `generation`
integer. Use the canonical identity returned by the query, not the raw native job/building number.
Do not copy the additional returned `revision` field into these input objects. Wrong entity kinds,
missing entities and generation mismatches are refused. Supplying both filters selects their
intersection. Whole-projection Query authority is still required; a focus does not widen a scoped
grant into permission to scan the complete inventory.

The report joins each selected job to its observed holder, attached items and container ancestry.
Findings include suspension, unassigned workers, unfinished holders, filter indices without indexed
attachments, direct and inherited item flags, shared attachments, and zero-sized attached stacks.
The existing deterministic inspection order and analysis policy are retained.

These findings do not establish causes. A suspended job can be intentional; an unfinished holder
can be normal construction; a missing indexed attachment does not prove a material shortage.
`blocker_proven`, `job_ready_proven`, `reservation_created` and `commit_compatible` remain false.
No-findings results explicitly do not prove readiness.

Each diagnostic row includes two bounded structured drill-downs:

- `inspect_assignment` inspects the job's worker assignment, strict-citizen join and position using
  the complete capture anchor and expected job generation. In spatial/1.8 a retained strict-citizen
  worker has the existing canonical entity reference. An outside-roster worker remains unknown.
- `inspect_relationships` traverses the job's observed `uses` and `contained_in` relationships,
  including attached items and their container/holder chains, under a bounded depth. It now also
  binds the full anchor instead of silently following whichever snapshot happens to be current.

Pass the returned object as the query argument of `fortress.query`, with the current session ID.
Following a stale live link fails instead of inspecting a recycled entity or newer capture. Neither
link acquires a native observation or diagnoses why the job is blocked.

## Allocate declared material supply without double counting

`inventory_plan` performs the existing integral allocation over conservative observed stack units:

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "inventory_plan",
      "quantity_unit": "stack_units",
      "demands": [
        {"key":"first-project","units":4,"item_types":["WOOD"]},
        {"key":"second-project","units":4,"item_types":["WOOD"]}
      ],
      "limit": 8
    }
  }
}
```

The type key is illustrative; use exact keys from this capture's item queries. A request accepts
1..32 declared demands, optionally narrowing by subtype and raw material type/index. Each physical
stack contributes its capacity only once across all demands. The response includes full requested
and allocated totals, disjoint exclusion counts, allocations and an integral-flow/min-cut shortage
certificate. Independently computed per-demand maxima must not be added together.

Removed, forbidden, rotten, trader, attached, in-job, dump, inventory-held and building-held supply
is excluded under the existing policy, including relevant inherited container restrictions.
Zero-sized stacks do not become supply. This is not full native job-requirement matching, material
substitution semantics, a resource reservation, a labor assignment or a commit-compatible plan.

`inventory_plan` deliberately does not prove access. Use the existing `spatial_inventory_plan` when
the declared request also needs the restricted terrain-route model. A model shortage here is about
the declared interchangeable stack units and conservative supply subset, not an authoritative
explanation of a native job's requirements or eventual success.

## Historical and offline use

Both operations are allowed inside `historical_query` in a journal-backed live session and in an
archive-only session. Discover an exact record and digest with `history`, then wrap the desired
production query using the existing historical-query envelope. No new journal is required.

The engines borrow the operations component, canonical snapshot and source digest of the enclosing
state. They never rebuild an operations-only snapshot with fresh generation counters. This preserves
the full spatial anchor, source identity and generation history when an item, building or job has
disappeared and later reused its native ID.

Historical assignment and relationship drill-downs are rewritten as exact-record queries. Their
record/digest stays fixed even if the live world has advanced or a new archive session is reading a
later capture. The rewritten request must fit the space already reserved for its original anchor-bound
form. Current session authority is checked before historical replay; old observations do not revive
expired or exhausted grants. Current state is not replaced and current watches are not evaluated.

A failed live bridge blocks ordinary production reads but does not block healthy verified historical
reads. Offline `mode="production"` inspects the latest retained capture and labels its result
historical, without a bridge connection or current-freshness claim. Archive schema discovery now
contains twelve stateless variants and three history variants. The spatial/1.8 history cursor schema
uses its actual `sch1` prefix; the shared spatial/1.6 schema file is unchanged.

## Budgets and retained state

The complete analysis precedes whole-row pagination. `limit` is 1..128; `op1` continuations bind
session, policy, full anchor, source digest and normalized request. Page width and output allowance
may change, but another capture, focus, selector or demand model invalidates the token. A complete
row that cannot fit produces BudgetExceeded rather than a non-progressing continuation.

Live requests reserve their complete Agent Turn, compact operational attention and current watch
metadata before allowing rows to fill the remaining result budget. Optional observation-journal
custody and its agreement with the current anchor are checked before analysis and before returning.
No new native capture is acquired, no watch sample is added, and neither observation nor watch
history is changed by successful pure production analysis.

The adapter now checks a cooperative wall-time allowance during analysis, in addition to its existing
explicit work ceiling. Query serialization and live preflight/presentation are also checked. This
is not hard preemption: the integral allocator is work-bounded and checked for elapsed time after
it returns, and synchronous filesystem operations cannot be interrupted by these checks. Historical
replay keeps the existing separately bounded archive-read path. No global end-to-end hard deadline
or cross-file transaction is claimed.

## Validation status

Ten new Rust test functions are registered: four adapter integration tests and six actual MCP-handler/
schema tests. A shared coherent fixture exercises worker joins, holder stages, inherited container
flags, shared attachments, competing demands and empty/reappearing rosters without regressing native
allocation counters. Coverage includes identical analysis across sealed profiles with distinct source
identities, generation reuse, scoped-authority/anchor/work refusals, bounded pages, stale continuations,
unchanged durable watches, historical pinned links, failed-source history and read-only archive use.
Existing archive tests were retained and extended for the two additional read operations.

**None of these Rust tests has been compiled or executed here.** No Rust compiler, Cargo or rustfmt
is installed; container network access could not retrieve repository/toolchain bytes. Review of the
committed diffs does not establish a passing build, warning-denied Clippy, runtime correctness, native
DFHack behavior or full repository qualification.

```bash
cargo test --locked -p dfmcp-adapter --test production_spatial_analysis_tests
cargo test --locked -p dfmcp-mcp production::tests -- --test-threads=1
cargo test --locked -p dfmcp-mcp spatial_archive -- --test-threads=1
```

The operations-only callers still use the same policy and algorithms. Their regression suite and the
full exact-head qualification remain required alongside the new tests. No production admission or
additional mutation capability should be inferred from this source increment.
