# Relationship-scoped foreground monitoring

The additive `related` row predicate selects one-hop neighbors of an explicitly
named, generation-fenced root in the selected observed projection. It composes
with existing `field`, `all`, `any`, and `not` predicates in `entity_count` and
`item_quantity` conditions, stateless quantity inspection, and condition inspection.
The enclosing watch, batch, historical query, and durable-definition machinery is
reused; no bridge call, top-level tool, timer, mutation, or authority is added.

## Agent workflows

Obtain canonical IDs and generations from the current query result. The IDs below
are examples, not native IDs or stable addresses for a particular fortress.
Pass these envelopes in the existing `fortress.query` JSON argument, with the
session supplied separately. Existing `expected_anchor` fencing remains available.

Watch the number of suspended jobs currently held by workshop `10`, generation 1:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "watch",
    "key": "workshop-suspended-jobs",
    "condition": {
      "op": "entity_count", "scope": "observed_projection", "kind": "job",
      "predicate": {
        "op": "all", "args": [
          {"op": "related", "entity_id": "10", "generation": 1,
           "relation": "contained_in", "direction": "incoming"},
          {"op": "field", "field": "suspended", "comparison": "eq",
           "value": {"type": "bool", "value": true}}
        ]
      },
      "comparison": "eq", "value": 0
    },
    "deadline_tick": 10000,
    "poll_interval_ticks": 10,
    "stable_observations": 2
  }
}
```

Choose a future game-tick deadline within the session grant. This proves only the
selected predicate at the required discrete observations. A missing job is not
proof of successful work, and no suspended jobs does not imply an unblocked workshop.

Inspect raw stack units actually attached to job `11`, generation 1:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "item_quantity", "scope": "observed_projection",
    "quantity_unit": "stack_units",
    "predicate": {"op": "related", "entity_id": "11", "generation": 1,
                  "relation": "uses", "direction": "outgoing"}
  }
}
```

`incoming` is relative to the root: it selects rows whose edge points **to** the
root. `outgoing` selects rows reached by edges **from** the root. Thus incoming
`contained_in` selects a workshop's jobs or a container's immediate contents;
outgoing `uses` selects a job's attached items; outgoing `performs` selects a
citizen's jobs; incoming `performs` selects a job's observed citizen worker.
The latter requires the citizen-inclusive spatial/1.8 projection for actual worker
edges. An older projection lacking those edges does not establish world absence.

Each entity contributes once even when multiple native attachment roles connect
it to the root. Stack units are not interchangeable materials, food portions,
usable inventory, fulfilled requirements, or reserved supply. Containment is not
recursive; a bag inside a barrel does not add the bag's contents to the barrel's
one-hop measurement.

## Identity, uncertainty, and evidence

All root references in the row predicate are bound before scanning any population,
including roots inside a decisive `any`/`all` branch. A missing, malformed, or
unresolved root makes the aggregate condition unknown even for zero rows. A
changed positive generation marks the enclosing watch invalidated, including when
another outer condition is decisive. Stateless condition inspection reports
`invalidated_reference`; quantity inspection retains the reference failure and
returns zero as a conservative lower bound with no established upper bound.

Membership remains dynamic across fresh observations; binding the root does not
freeze its neighbors. A root may have an empty observed neighborhood, but that is
not a complete-world absence certificate. Fresh, nonzero, consistent native
DFHack provenance is required for each supporting edge. Missing, stale, derived,
redacted, or inconsistent evidence gives unknown membership; negation does not
turn it into proof. A proven parallel edge may establish existential membership
when another parallel edge is unestablished. No hidden backing field is emitted.

`relationship_selection` reports each root, requested/current generation, relation,
direction, binding status, and indexed endpoint count. It explicitly records
one-hop scope, entity deduplication, and the absence of complete-world guarantees.
Requests without relationship predicates retain their prior evidence shape.
Terrain predicates share the syntax but do not treat graph membership as certified
tile evidence: a `related` terrain leaf remains unknown, with root fences retained.

Root binding and edge scans share the existing one-million-unit foreground work
budget and cooperative deadline with all other conditions and batch watches.
All predicates still share the 64-node/depth-8 validation bound. A measurement can
index at most 65,536 distinct selector/endpoint pairs. Exhaustion returns an error,
not a partial count. Complete rendering still precedes retained watch publication
and any configured durable checkpoint. Older binaries reject the new predicate
rather than silently interpreting it as another operation; no archive migration
or backward readability of new definitions by old binaries is promised.

## Implementation status and evidence

Twelve Rust tests are registered in `query_watch_relationship_tests.rs`, covering
workshop filters, attachment deduplication, quantity inspection, root-relative
worker/container joins, native provenance/negation, canonical edge-kind aliases,
empty populations, generation invalidation through retained watch handlers,
dangling endpoints, shared budget exhaustion, output refusal, and strict parsing.
They have **not been compiled or executed in this environment**; Rust, Cargo, and
rustfmt are unavailable. No Rust/Clippy/stdio or whole-repository qualification is
claimed, and the separate open stdio lifecycle bead is not resolved here.

`python scripts/test_relationship_watch_schema.py` passed 44 isolated JSON Schema
component cases: 7 accepted and 37 rejected. It includes a newline-suffixed ID
regression. This is not full composed-schema validation, Rust execution, MCP
execution, durable recovery execution, real DFHack compilation, or a live campaign.
The production runner map and empty compatibility registry are unchanged. The
feature is development source, not a newly admitted live capability.
