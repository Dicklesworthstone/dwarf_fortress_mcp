# Duration-aware dependency scheduling

`dfmcp_world::graph_query::schedule_dependencies` computes a deterministic
conditional schedule from an immutable, already-authorized world projection.
This implements the timed DAG analysis described in
`FORTRESS_GRAPH_ALGORITHMS.md` section 6.11. The existing
`analyze_dependencies` output, including its unweighted dependent-first chain,
is unchanged.

## Inputs and selection

`DependencyScheduleQuery` names explicit edge kinds, a `duration_field`, and up
to 64 target entity IDs. Edges mean **dependent → prerequisite**. Nonempty
targets select their complete transitive prerequisite closure. An unrelated
fortress, citizen or item does not need a fabricated duration. An empty target
list selects every vertex. A selected dependency is never silently dropped to
make scheduling possible.

Every selected entity must have a consistently Known, nonnegative `u64` or
`i64` duration. Zero is valid. Missing, stale, omitted, redacted, contradictory,
future, negative and noninteger values refuse the entire schedule. Accepted
provenance is either the registered laboratory fact contract or a bounded
nonempty DFHack source field with a nonzero source digest. Arbitrary derived
labels, agent assertions and replay facts cannot supply duration values.
These checks establish the source shape, not native compatibility or admission;
the caller remains responsible for supplying an authentic authorized projection.

The duration field's units remain explicit model units. Choosing a field does
not convert its values into Dwarf Fortress ticks. The model assumes fixed
durations, finish-to-start dependencies, and unlimited capacity for independent
work. It does not account for shared workers, resource contention, travel,
interruptions, uncertainty or unobserved prerequisites. Its result is a
prediction, never an observed completion or mutation precondition.

## Results

Each `ScheduledDependency` retains the entity's generation and revision and
reports duration, earliest start/finish, latest start/finish, and total slack.
Offsets begin at zero. Latest offsets are measured against the complete
selected schedule's makespan, including disconnected selected branches.

`dependency_order` is prerequisite-first, with ascending stable ID among ready
vertices. `critical_chain` is prerequisite-first and ends at a terminal vertex
of the selected closure. Equal-duration choices use stable identity. The chain
can include zero-duration dependencies. All critical alternatives remain
visible through zero total slack; the chain identifies one canonical choice.

`parallel_fronts` groups vertices by their earliest start offset. These are
start-time groups, not unordered dispatch batches: zero-duration prerequisites
and their successors can share an offset. Follow `dependency_order` within a
group. Parallel edges impose one precedence relation and do not multiply a
task's duration.

A cycle within the selected closure returns `conflict`; callers can use the
existing dependency analysis to inspect its SCCs and affected dependents.
Unrelated cycles outside a nonempty target closure do not block that schedule.
Integer overflow, source corruption, budget exhaustion and cancellation return
errors without a partial schedule.

## Bounds and witnesses

The algorithm uses iterative prerequisite traversal and forward/backward DAG
passes with linear auxiliary state. It does not enumerate paths or recurse with
graph depth. Ordered ready sets and adjacency make ties deterministic. Work,
vertices, edges and simultaneous frontier are bounded by `GraphBudget`.
`schedule_dependencies_with_check` additionally invokes a caller-owned check
before/after construction, throughout bounded analysis and before publication.
Complete source admission and snapshot hashing are still the caller's cost.

The witness binds the full source anchor, authorization-scope digest, selected
edge projection, canonical targets, duration field and source facts, input
budgets, scheduling policy, decisions and result. A digest grants no authority.
Authorization filtering must precede the supplied projection. A selected
closure is complete only within that projection; it proves neither complete
fortress history nor absence of unobserved relations.

## Existing-tool query

The default laboratory runtime exposes this model through the existing
`fortress_query` structured mode. Pass this object as the JSON string in `mode`:

```json
{
  "mode": "structured",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "schedule",
      "edge_kinds": ["requires"],
      "duration_field": "duration",
      "targets": [{"entity_id": "4", "generation": 1}]
    }
  }
}
```

Use an actual published integer field and inspected entity handles; this example
does not imply that every native or starter-world entity has a `duration` fact.
Omit `targets`, or supply an empty array, to schedule every observed vertex.
Explicit targets must be distinct and their generations must match the selected
source. The closed request accepts no caller durations, capacities, actions or
execution directives. Duplicate schedule and target fields in the original
laboratory request bytes are rejected before execution.

The response includes each exact duration fact and its provenance, entity
generation/revision, scheduling offsets/slack, makespan, critical chain,
dependency order, start-time groups and source/scope/decision witnesses.
`model.epistemic_state` is `predicted`; `model.time_unit` is
`selected_field_units`. The model creates no actions, clock advancement or
retained monitor. Cycles can be inspected using `kind: "dependencies"`.

The laboratory wrapper also accepts an exact retained `at` state hash. The
source anchor identifies that historical model; the current anchor identifies
the session. All repeated authorization uses current grants and the actual
current tick, even when a restore makes the historical source tick larger.
Source identity never renews expired grants or supplies current mutation truth.

Complete source records, unrelated fields and canonical bytes are admitted
before graph work. The final schedule is published whole or refused; it is not
paged or silently truncated. Output reservations for Agent Turns or active work
do not reduce the separately admitted source allowance. The caller can select
fewer goals or explicitly negotiate more output when the complete closure does
not fit. The eleven tools and legacy dependency request/response remain intact.

The generic semantic executor and its schema-extension helper also expose the
stateless operator. Live profiles retain their own discovery and history
allowlists; this increment does not claim universal live-profile discovery,
historical live support or native duration coverage.

## Verification scope

`crates/dfmcp-world/tests/graph_scheduling.rs` compares small weighted DAGs and
arbitrary cyclic graphs with independent path/closure oracles and exercises
source qualification, zero-duration ties, multi-edges, exact numeric bounds,
complete prerequisite selection, cancellation, frontier/work limits and deep
iterative graphs. Execution evidence and its exact scope are recorded in
`IMPLEMENTATION_STATUS.md`; these tests do not establish native-game admission.

The focused MCP executor tests cover duration versus hop-count decisions,
canonical target selection, generation fences, unavailable evidence, current
authority, exact source anchors, complete source/output admission, separate
output reservation, restored-clock history, closed grammar and schema
composition. Public laboratory and publisher regression sources additionally
cover facade reachability, historical generation/current grants, duplicate raw
keys and preserved source budgets. Their executed scope is recorded separately
from the std-only core suite and from full transport qualification.

Owning bead: `df-franken-storage-mvcc-54h.4.1`.
