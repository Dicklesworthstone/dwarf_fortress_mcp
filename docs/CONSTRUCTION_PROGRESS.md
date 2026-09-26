# Furniture construction progress and explicit watches

The existing spatial/1.6 and citizen/spatial/1.8 `fortress.query` handlers now
accept `construction_progress`. It uses the already-published coherent operations
component, not a second native read or an independently timed building/job/item
join. The adapter analyzer is also available for sealed operations-only states.
No top-level MCP tool, native protocol, game mutation or production admission is
added. Rust/MCP compilation and execution remain unverified in this environment.

## Inspect furniture, not just placement acknowledgements

In an existing spatial session, submit this envelope as the normal `query`
argument. IDs and the deadline below are illustrative: select native IDs and
canonical generations from that session's observations, and choose a future tick
inside its negotiated horizon. Omit `monitor` for diagnosis without a proposal.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "construction_progress",
    "targets": [
      {
        "building_native_id": 10,
        "expected_generation": 1,
        "expected_type": "Bed",
        "item_native_id": 20
      }
    ],
    "monitor": {
      "key_prefix": "bedroom",
      "deadline_tick": 100801200,
      "poll_interval_ticks": 1,
      "stable_observations": 2
    },
    "limit": 4
  }
}
```

`expected_anchor` may be added at envelope level to require one exact published
anchor. Every response carries the enclosing canonical anchor and source digest;
spatial/citizen entity generations are never replaced by a fresh operations-only
projection. Current Query authority, cancellation, scan and work limits apply.

Targets are explicit native building IDs. Optional `expected_generation` and
`expected_type` reject mismatched observed identities; the only supported expected
types are `Bed`, `Chair` and `Table`. Optional `item_native_id` requires that exact
item's installation condition. Duplicate building IDs or one item selected for
multiple buildings are refused, not deduplicated or silently reassigned.

Each row includes the canonical building handle, current bounds and construction
stage, complete construction/removal/other-job counts, suspended-construction and
worker-present counts, up to eight job examples, and optional exact-item links.
A worker ID in a job example is a native observation, not a citizen-generation
handle. Timers and flags are reported as observations, not predicted finish times
or explanations of why work is delayed.

The statuses distinguish:

| Status | Interpretation at this capture |
|---|---|
| `missing` | The selected building is not in the published roster. |
| `identity_mismatch` | A supplied generation or type expectation disagrees. |
| `unsupported` | Type or maximum-stage policy is not supported. |
| `removal_pending` | At least one observed `DestroyBuilding` job holds this building. |
| `suspended` | Construction is present and all its observed jobs are suspended. |
| `no_construction_job` | The stage is incomplete without an observed construction job. |
| `pending` | Construction is still present or the stage is incomplete. |
| `item_unverified` | The requested exact item's installed condition is missing or false. |
| `satisfied_at_observation` | All declared stage/job/item conditions hold at this capture. |

Job disappearance alone **never** establishes the satisfied condition. All jobs
are counted even after the eight-example limit; a late removal job cannot be
hidden by truncation. The summary covers all selected targets on every page,
including targets that are not on that page.

## The exact condition

Policy `dfmcp.furniture-construction-condition/1` accepts Bed, Chair and Table with
a maximum build stage in 1..32. It requires stage equality and no observed
`ConstructBuilding` or `DestroyBuilding` job held by the selected building.

When an exact item is requested, it additionally requires matching item kind
(BED/CHAIR/TABLE), that building as its holder, no container, `in_building=true`,
`in_job=false`, `removed=false`, `on_ground=false`, `in_inventory=false`, and no
attachment to any observed job. Multiple attachment roles to one job count once.
Other item flags are reported but do not establish usability or supply eligibility.

This is a condition on selected observed entities. It does **not** authenticate a
build/1.19 placement receipt, the original selected item/footprint, a shared native
incarnation between plugins, current terrain, room assignment, reachability,
building usability, mining/building causality, safety or native effect completion.
A receipt's native ID may be used as a selector, but coincident IDs are not proof
of continuity. Existing effect recovery and unresolved-work records remain intact.

## Register and sample deliberately

An eligible row's `monitoring.watch_request` is a complete envelope for the
existing `fortress.query` watch interface. Submit it explicitly in the returned
`session_id`. The proposal itself creates no handle, work obligation, reservation
or native preparation. Missing/unsupported building identities or a missing exact
item produce `monitoring.available=false` with a reason, not a weakened predicate.

The generated key is `<key_prefix>.<building_native_id>`. The prefix is 1..32 ASCII
letters/digits/underscore/dot/hyphen. Deadline, cadence and sample count are checked
against the current session; even an immediately matching goal must have enough
remaining time for the requested cadence and number of samples.

The watch pins the observed building type, maximum stage and canonical building
and item generations. It uses the existing observed-field, entity-count and
one-hop relationship predicates. A changed maximum stage never silently changes
an existing goal. A generation mismatch invalidates the shared watch. Missing
facts remain unknown at their leaves; a known false conjunct may still make the
whole condition false, but neither case provides a successful sample.

An observed removal job is an explicit failure condition. Otherwise normal
`poll_watch`, `await_watch` and shared `await_watches` semantics apply. Repeated
polls of one anchor do not add samples. Tick cadence, skipped-observation resets,
fixed deadlines, cancellation and terminal immutability are inherited from the
existing watch engine. No new timer, background polling, unpause or retry exists.
When the existing paired spatial/1.8 observation/watch journals are configured,
registration and recovery use their existing custody and persistence rules.

Normal retention limits still apply, including eight watches per session. A
32-target diagnosis does not create capacity for 32 simultaneous watches. Choose
which proposals to register and explicitly release terminal watches as needed.
Key conflicts, stale proposal anchors and output-budget failures are not bypassed.

## Bounds and pagination

The closed query schema is `schemas/mcp_construction_progress_v1.json`. Targets
are limited to 32; page width is 1..32 (default 4); analysis work is 1..1,000,000.
Complete scans remain subject to the session's entity and cooperative wall-time
budgets. Native roster limits are unchanged. No absence or completeness claim is
made about unobserved game history.

Continuations bind session, full anchor, source digest, sorted target selection,
policy, effective monitor options and analysis-work bound. Input target order and
page width may change without changing the selection identity. New observations
or changed targets/options require restarting pagination. Each proposal stays
intact in its row; a complete row plus summary must fit the existing reserved
MCP/Agent Turn output budget, or the query refuses. No partial watch is returned.
Large monitor rows may require a larger negotiated output budget.

## Validation scope

Ten adapter and ten shared MCP regression functions are added. They exercise
actual adapter publication and shared query/condition/watch handlers when run,
including lifecycle/stability, all 512 item flag words, item/container/job links,
removal, missing/recycled identities, authority, failure publication, complete
32-target pagination and schema discovery. The shared tests are included in both
spatial runtime modules. **They have not been compiled or executed here.**

Executed independently:

```sh
python3 scripts/check_construction_progress_reference.py
```

The reference validates 29 accepted and 50 rejected JSON shapes and 4,114 cases
against the checked-in predicate fixture. The fixture has 28 condition/predicate
nodes, depth four and 2,808 compact JSON bytes; those bytes are the predicates
only, not a complete response measurement. A Rust regression separately requires
actual generated predicates to equal the same fixture before evaluating them
through the real engine. Python reference success cannot establish that Rust
compiles or that MCP/native execution works. Exact scope and hashes are recorded
in `docs/evidence/construction-progress-reference.json`.

No real DFHack, live-fortress, physical power-loss or full-workspace qualification
is claimed. Broader construction, obligation and bridge beads remain open.
