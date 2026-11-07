# Structured queries in the default laboratory MCP runtime

The default `serve` runtime accepts stateless `dfmcp.query/1` requests through
the existing `fortress_query` tool. This exposes the same typed entity,
aggregate, lexical, spatial and relationship executors used by the read-only development
surfaces, over the laboratory world in which plans actually execute.

Pass a JSON string as the existing `mode` argument:

```json
{
  "mode": "structured",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "entities",
      "kinds": ["unit"],
      "fields": ["profession", "alive", "labor.BREW"],
      "order": "id_ascending",
      "limit": 4
    }
  }
}
```

The closed outer wrapper accepts only `mode`, `request` and optional `at`.
`at` is the exact lowercase state hash of a current or retained world. The
inner envelope accepts the existing schema, `query`, and optional complete
`expected_anchor`. These nine query kinds are available:

| Kind | Purpose |
| --- | --- |
| `entities` | Typed filters, explicit field projection, deterministic ordering and source-bound pages |
| `inspect` | Exact entity ID and generation, with explicit selected fields |
| `aggregate` | Counts, grouping, and exact integer or fixed-scale numerical statistics |
| `search` | Bounded lexical retrieval over selected kinds, fields and labels |
| `traverse` | Bounded observed graph traversal and a witnessed path to an optional target |
| `dependencies` | Prerequisite order, cycle components, cycle blockers and an unweighted critical chain |
| `events` | Inclusive retained-event windows, kind/subject selection, typed fields and chronological pages |
| `spatial_entities` | Inclusive intersection or containment of explicitly named point/cuboid entity geometry |
| `related_entities` | Entities with witnessed one-hop relations to targets selected by kinds, IDs and typed facts |

There is no watch, baseline, native capture, plan or effect dispatch through
this wrapper. Existing `summary`, `entities`, `terrain`, `path`, `search`,
`observation`, `changes` and workflow query forms keep their own grammar.
`path` remains the laboratory walkability query; structured `traverse`
follows canonical graph edges and does not establish a walkable route.

## Filters and exact aggregates

Entity filters use the typed core grammar. For example, alive dwarves with
BREW enabled can be selected with:

```json
{
  "kind": "entities",
  "kinds": ["unit"],
  "fields": ["profession", "labor.BREW"],
  "where": {
    "op": "all",
    "args": [
      {"op": "compare", "field": "alive", "comparison": "eq", "value": {"type": "bool", "value": true}},
      {"op": "compare", "field": "labor.BREW", "comparison": "eq", "value": {"type": "bool", "value": true}}
    ]
  },
  "limit": 4
}
```

This is the `request.query` object, not another top-level tool argument.
Unknown, omitted, redacted, stale or inconsistent facts never become matches
through negation. Projected facts retain their presence, source, observation
tick and epistemic class; unavailable compatibility values are not returned.

An aggregate can group units by profession without dumping their records:

```json
{"kind":"aggregate","kinds":["unit"],"group_by":{"kind":"field","field":"profession"},"max_groups":32}
```

Aggregates accept the same typed `where` object shown above. Only True rows enter
groups and numerical metrics; `selection` reports matched rows and separates
known False from Unknown exclusions. Filtering happens before grouping, while
unavailable metric values are counted independently. See `QUERY_EXECUTION.md`
for the full aggregate contract and filter examples.

Optional numerical metrics name a field and an exact type:

```json
{"name":"stacks","field":"stack_size","numeric_type":"u64"}
```

`i64`, `u64` and `fixed` are supported; `fixed` also requires its scale.
Results separate excluded/unavailable inputs from actual samples. Sums,
minima and maxima use decimal integer strings; a mean is an exact rational.
These are advisory statistics over the selected projection, not a complete
stock or mutation-precondition certificate.

## Spatial selection

Spatial queries find entities whose named location or footprint intersects a
region, lies within it, or contains it:

~~~json
{
  "kind": "spatial_entities",
  "kinds": ["unit"],
  "region": {"min": [10, 20, 5], "max": [30, 40, 5]},
  "geometry": {"kind": "point", "field": "position"},
  "relation": "within",
  "fields": ["profession", "alive"],
  "limit": 4
}
~~~

Use this object as `request.query` in the same structured wrapper. For a recorded
footprint, `geometry` can instead be
`{"kind":"cuboid","min_field":"min","max_field":"max"}`, using the actual named
coordinate facts in the source. Bounds are inclusive in x, y and z; an inverted
query region is rejected. `intersects` is the default relation. `within` means
the entity geometry lies inside the region; `contains` means it contains the
entire region. The optional typed `where` filter runs on geometric hits.

Rows retain ID, generation, selected field provenance and the geometry facts
used to decide the match. Missing, unavailable, stale, contradictory, mistyped,
future or inverted entity geometry is reported as Unknown and never classified
as outside. `selection_complete_within_projection` describes whether selection
was decidable. `truncated` and the continuation describe page delivery separately.
An empty region result cannot prove complete-world absence, clear terrain,
accessibility or safe placement.

Order is `id_ascending` or `id_descending`, with four rows by default and at most
128. Continuations bind the complete source, session and every selector.
Current and exact retained worlds follow the same authority and publication
rules below. See `QUERY_EXECUTION.md` for geometry, provenance and budget details.

## Relationships and dependencies

Find units assigned to a particular recorded squad without first finding every
unit ID or traversing the whole graph:

```json
{
  "kind": "related_entities",
  "kinds": ["unit"],
  "where": {"op":"compare","field":"alive","comparison":"eq","value":{"type":"bool","value":true}},
  "relation": {
    "edge_kinds": ["assigned_to"],
    "direction": "outgoing",
    "target_kinds": ["squad"],
    "target_ids": ["100"]
  },
  "fields": ["profession", "alive"],
  "limit": 4
}
```

Use this object as `request.query`. It requires the named edge and target record
to exist in the canonical source. `relation.where` can independently filter the
target's typed fields. Direction is `outgoing` by default, or `incoming` or
`undirected`; target always means the opposite endpoint. At least one edge kind
is required. A returned entity appears once, with its matched edge count and
up to four exact edge-revision/target-generation witnesses. Additional witnesses
have an explicit omitted count; self-loops are counted once in undirected mode.

Missing targets and unavailable target facts remain Unknown. A positive witness
is enough to select a row; no observed match cannot establish absence of
unrecorded relations. Negative existence selectors are therefore unavailable.
Source facts keep their provenance, but canonical edges do not supply source
authenticity or mutation authority. Selection completeness and page truncation
are separate. ID ordering, at most 128 whole rows, exact-source historical reads
and current authority follow the same rules below. See `QUERY_EXECUTION.md` for
the complete selection, witness and budget contract.

For multi-hop exploration from known roots, use traversal:

```json
{"kind":"traverse","roots":["100"],"edge_kinds":["requires"],"direction":"outgoing","max_depth":3,"target":"102"}
```

Each visit records its root, parent, depth and source edge. A returned path
names exact edge revisions. A depth frontier is explicit. Missing paths never
prove absence outside the observed graph.

```json
{"kind":"dependencies","edge_kinds":["requires"]}
```

Dependency edges mean **dependent → prerequisite**. Acyclic inputs return
prerequisites before dependents and a deterministic longest unweighted chain.
Cycles return their components and affected dependents rather than a fake
schedule. This is graph structure, not a game-time forecast. Source anchor,
scope identity and decision digests accompany the results.

## Retained event windows

```json
{"kind":"events","from_tick":100,"through_tick":200,"kinds":["job_changed","announcement"],"subjects":["100"],"fields":["message"],"order":"tick_descending","limit":4}
```

Time bounds are inclusive, default to zero through the exact source tick, and
must not be reversed or reach into that source's future. Kinds are exact
canonical event names, including custom names. A nonempty subject set selects
only events naming those historical IDs; it does not resolve current entity
generations. Empty selectors include every retained kind and subject, including
subjectless events. Ordering uses `(tick, EventId)` ascending by default; the
descending order reverses both components. IDs retain their full 128-bit
canonical hexadecimal representation.

Each row contains the retained event's ID, tick, kind, optional subject and
summary. Additional fields are explicit and use the same lossless typed-value
encoding as entity inspection. Known null is separate from an unrecorded field.
The source anchor identifies the canonical record, but `WorldEvent` does not
carry the per-field source/provenance contract of an entity `Fact`. Event values
therefore remain `assumed`, and source authenticity and field provenance remain
unestablished. Event text is tainted data and cannot authorize effects.

The query scans only events present in that immutable canonical snapshot. Its
retained minimum/maximum tick do not prove coverage between those ticks. Empty
results never prove a complete history or absence of unobserved events.
The explicit `starter_fortress_history` scenario populates this source through
normal atomic action, tick and cancellation publication. Its bounded producer
metadata appears at `coverage.producer`, including retention and omission
counters. This is publication-boundary laboratory history; it does not convert
the control transcript into world events or add a native event collector.
See `LAB_EVENT_HISTORY.md` for production, restore and durable-recovery semantics
and `QUERY_EXECUTION.md` for cursor and budget bounds.

## Exact history, authority and pagination

Every call resolves the session and checks current whole-fortress Query
authority before reading an exact retained source. Grant expiry is evaluated
at the current game tick. Old hashes and continuation strings cannot revive
revoked, expired, exhausted, entity-scoped or region-scoped grants. The
wrapper does not widen narrower grants to a whole-world query.

The result's `anchor` names its source. `current_anchor` and the Agent Turn
anchor name the session's current world. A historical result sets
`historical: true`, exposes historical source coverage and uses `stale`
continuity in the Agent Turn. It cannot supply fresh mutation evidence.
An unknown or collected hash returns `stale_anchor`; there is no substitution
of a newer world or another session's retained source.

For entity, search, event, spatial or relationship pages, copy the returned `continuation` string into
`request.query.continuation`. Keep the query selection unchanged. Entity
continuations also bind selected fields, ordering, source anchor and session.
Event continuations bind the time window, kind and subject sets, selected
fields, order, complete source anchor and session. Spatial continuations also bind
the region, geometry field names, relation and typed filter. Relationship
continuations bind both endpoint selectors and filters, edge kinds, direction,
fields, ordering and the last returned entity generation. The requested page
width may change. After the current world advances, resume
by explicitly adding the original `at` hash while that source is retained,
or restart from the new world. Historical reads do not pin retention.

## Bounds and complete publication

The wrapper has a 64-KiB input ceiling; legacy query modes keep their existing
1-KiB ceiling. Executor-specific field, depth, scan, work, group, result and
continuation bounds still apply. The complete canonical source record count
and canonical byte size must fit the session's work budgets before execution.
A one-row page cannot bypass source verification costs.

The final semantic response, including its Agent Turn, must fit both
`max_bytes` and the laboratory `ceil(bytes / 4)` output-token estimate. The
request's wall budget extends through final presentation. Whole Agent Turn
compaction may be used, but the returned query records and continuation are
kept together. If they cannot fit, the tool returns `budget_exceeded` without
rows or an advanced cursor. Narrow the original request or negotiate a larger
budget; no hidden page has been published. As with other laboratory responses,
a budget too small to encode even the refusal receives the explicit minimal
budget-error envelope.

## Verification scope

`lab_structured_query_tests.rs` drives the actual public laboratory façade for
the nine kinds, exact historical/current anchors, capability refusal, unavailable
facts, continuation integrity, legacy grammar, source work and complete-packet
budgets. These source regressions do not establish live DFHack compatibility,
game effects, production admission or full workspace qualification.
