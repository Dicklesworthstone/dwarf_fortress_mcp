# Structured queries in the default laboratory MCP runtime

The default `serve` runtime accepts stateless `dfmcp.query/1` requests through
the existing `fortress_query` tool. This exposes the same typed entity,
aggregate, lexical, spatial, relationship and numeric-ranking executors used by the read-only development
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
`expected_anchor`. These ten query kinds are available:

| Kind | Purpose |
| --- | --- |
| `entities` | Typed filters, explicit field projection, deterministic ordering and source-bound pages |
| `inspect` | Exact entity ID and generation, with explicit selected fields |
| `aggregate` | Counts, grouping and exact integer/fixed statistics, optionally selected by relationships and location |
| `search` | Bounded lexical retrieval over selected kinds, typed predicates, fields and labels |
| `traverse` | Bounded observed graph traversal and a witnessed path to an optional target |
| `dependencies` | Prerequisite order, cycle components, cycle blockers and an unweighted critical chain |
| `events` | Inclusive retained-event windows, kind/subject selection, typed fields and chronological pages |
| `spatial_entities` | Inclusive intersection or containment of explicitly named point/cuboid entity geometry |
| `related_entities` | Entities with witnessed one-hop relations to targets selected by kinds, IDs and typed facts |
| `ranked_entities` | Best bounded page under exact numeric orderings, with optional relationship and spatial selectors |

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

### Aggregate by recorded relationships and location

An aggregate also accepts optional `relation` and `spatial`, with the same closed
selector objects used by `ranked_entities`. Either can be used alone or both
together. This answers questions such as stock totals for a selected stockpile
inside a work area, or profession counts in a particular part of the fortress,
without downloading and joining all contributing records.

For a source that publishes item stacks, their coordinates and containment edges:

```json
{
  "kind": "aggregate",
  "kinds": ["item"],
  "relation": {
    "edge_kinds": ["contained_in"],
    "target_kinds": ["stockpile"],
    "target_ids": ["2001"]
  },
  "spatial": {
    "region": {"min": [0, 0, 10], "max": [9, 2, 10]},
    "geometry": {"kind": "point", "field": "position"},
    "relation": "within"
  },
  "group_by": {"kind": "field", "field": "type_key"},
  "metrics": [{"name": "quantity", "field": "stack_size", "numeric_type": "u64"}]
}
```

Use this as `request.query`; select field names, edge kinds and target IDs that
actually exist in the chosen source. A relationship target may be outside the
region or have no geometry. Each matching source entity contributes once, even
when several recorded edges select it. Source `where` runs first, then the
positive relationship, then source geometry, then grouping and exact metrics.
An unavailable metric does not remove an otherwise selected member.

`selection` reports source False/Unknown exclusions, relation matches,
`unknown_relation`, `no_observed_match`, missing and unavailable targets,
and spatial matches/outside/Unknown geometry separately. Relation counts precede
spatial selection; their edge counts are incidences, not inventory quantities.
Unknown membership never becomes a zero contribution to a supposedly complete
total. `coverage.totals_scope` is `selected_known_members_only`, and
`selection_complete_within_projection` only means membership was decidable
within the supplied source. Metric sample exclusions remain explicit even when
membership is complete. Whole-fortress and relation-domain completeness, absence
proofs and mutation authority remain unestablished.

Complete-source admission includes targets, edges, unrelated fields, chunks and
events before hashing or allocating a relation index. Source/target predicates,
geometry, grouping and metrics share one work and wall budget. Groups publish
atomically; an excessive group count or complete-packet size returns
`budget_exceeded` without partial totals. Omitting both selectors preserves
the existing aggregate output shape. The complete contract is in
`QUERY_EXECUTION.md`.

## Filtered lexical search

Combine text matching with the same typed `where` selection. For example:

```json
{
  "kind": "search",
  "text": "Urist",
  "kinds": ["unit"],
  "where": {
    "op": "compare", "field": "alive", "comparison": "eq",
    "value": {"type": "bool", "value": true}
  },
  "limit": 4
}
```

Use this object as `request.query`. Only True predicate rows enter lexical
matching and ranking. `selection.matched_predicate` counts those rows before
text matching; top-level `matched` counts actual lexical hits. False and Unknown
exclusions remain separate, and negation does not turn unavailable facts into
matches. Filters participate in work admission and continuation identity.
Unfiltered requests keep their existing shape and ranking. See
`QUERY_EXECUTION.md` for text fields, matching rules and selection accounting.

## Ranking by numeric facts

Select the most relevant records without retrieving and sorting the population
in the caller. For example, if the source records a signed `stress` fact:

```json
{
  "kind": "ranked_entities",
  "kinds": ["unit"],
  "order_by": [
    {"field": "stress", "numeric_type": "i64", "direction": "descending", "max_age_ticks": 20}
  ],
  "fields": ["profession", "alive"],
  "limit": 4
}
```

Use this object as `request.query`. Supply one to three distinct field names.
Each key explicitly declares `i64`, `u64` or `fixed`, ascending or descending.
A fixed key also requires its exact `scale`; other numeric types cannot specify
one. Keys are compared in the requested sequence, then by ascending entity ID.
There is no float conversion, signed/unsigned coercion, scale conversion or
zero-filling of unavailable values.

Every ordering fact must be available and consistently Known, have the declared
type and scale, and have an observation tick no later than the source tick.
Optional `max_age_ticks` applies independently to each key. Without it, an older
Known fact may be ranked while retaining its original tick and epistemic class.
Missing, unavailable, future, over-age or mistyped keys exclude that entity and
appear in `selection.unknown_rank`, separately from typed-filter False and
Unknown exclusions. Reason counts name the first unusable key per entity.

Rows include the exact ranking facts and their provenance even when `fields`
does not request them. Advisory inferred or assumed facts keep those classes;
ranking never grants mutation authority or proves complete-world absence.
The bounded heap retains at most the requested page: four rows by default,
at most 128, with O(N log K) ordering work and O(K) retained candidates.
Complete-source admission and hashing still apply on every page.

The `rk1` continuation binds the session, full source anchor, filter, selected
kinds and fields, all ordering rules and the last returned ID and generation.
The page width may change. Pagination never skips a row that does not fit;
narrow the field projection or increase the output budget. Exact retained
sources and current authority follow the rules below.

Rank a relationship-selected population in the same request by adding `relation`
to `ranked_entities`. For example, to rank citizens assigned to squad `100`,
combine the stress ordering above with:

```json
{
  "relation": {
    "edge_kinds": ["assigned_to"],
    "target_kinds": ["squad"],
    "target_ids": ["100"]
  }
}
```

The selector has the same closed grammar as `related_entities`, including its
independent target `where`. Source filters run before relation selection and
numeric ranking. Missing endpoints or unavailable target facts remain Unknown;
a positive witness is required to enter the ranked population. Each returned
row keeps both its numeric facts and up to four exact relationship witnesses.
Relation exclusions and ranking exclusions are counted separately. The
complete source/target scan is bounded, with a temporary O(N) relation index
and the existing O(K) ranking heap. Every selector binds the continuation;
requests without a relation retain their previous identity and response shape.
See `QUERY_EXECUTION.md` for the complete composition and coverage contract.

Add optional `spatial` to the same ranked request to select locations before
ranking. It can be used with or without `relation`:

```json
{
  "spatial": {
    "region": {"min": [10, 20, 5], "max": [30, 40, 5]},
    "geometry": {"kind": "point", "field": "position"},
    "relation": "within"
  }
}
```

Source facts, positive relationships and geometry select one population before
the numeric top-K is chosen. The spatial selector applies to returned entities;
relationship targets may be elsewhere or lack geometry. Missing or unavailable
geometry is counted as Unknown, separately from known outside-region entities.
Required coordinate facts, ranking facts and relationship witnesses all survive
page fitting even when omitted from `fields`. Every supplied selector binds the
same source/session continuation. The complete combined example and staged
coverage counters are documented in `QUERY_EXECUTION.md`.

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

For entity, search, event, spatial, relationship or ranked pages, copy the returned `continuation` string into
`request.query.continuation`. Keep the query selection unchanged. Entity
continuations also bind selected fields, ordering, source anchor and session.
Event continuations bind the time window, kind and subject sets, selected
fields, order, complete source anchor and session. Spatial continuations also bind
the region, geometry field names, relation and typed filter. Relationship
continuations bind both endpoint selectors and filters, edge kinds, direction,
fields, ordering and the last returned entity generation. Ranked continuations
also bind key sequence, numeric types, scales, directions and age bounds. The requested page
width may change. After the current world advances, resume
by explicitly adding the original `at` hash while that source is retained,
or restart from the new world. Historical reads do not pin retention.

## Bounds and complete publication

The wrapper has a 64-KiB input ceiling; legacy query modes keep their existing
1-KiB ceiling. Executor-specific field, depth, scan, work, group, result and
continuation bounds still apply. The complete canonical source record count
and canonical byte size must fit the session's work budgets before execution.
A separate bounded structural walk checks source nesting, value frontiers and
work before canonical serialization. Its conservative estimate is distinct from
the exact negotiated canonical-byte allowance. A one-row page cannot bypass
source verification costs, including fields omitted from the response.

For structured `entities`, `spatial_entities`, `related_entities`,
`ranked_entities` and `events`, the complete laboratory wrapper and captured
Agent Turn participate while the page is chosen. `returned` can be smaller
than the requested `limit` without requiring the caller to retry smaller limits.
Entity pages render the requested `fields` and any required geometry, relation
witnesses or ranking facts; event pages retain their selected fields and event
metadata. Large unselected values do not consume the projected output allowance,
while complete-source work admission still includes them.
Rows form one ordered prefix and the continuation follows exactly that prefix.
Neither an oversized row nor a zero-row advancing page is silently accepted.
The final page can fit even when removing its continuation makes it smaller
than the preceding prefix. Empty results still need room for their full packet.

Presentation state is captured once outside the session lock and reused for
pure page-size checks. Before execution resumes, the current anchor, retained
source, grants and budget must still match. A refused query does not advance
the cached successful orientation or claim newly observed query changes. If
final output fitting or the publication deadline withdraws an otherwise valid
page, its refusal is rebuilt from the prior observation and bounded again using
the original query allowance, even when the first fit removed budget metadata.

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
the available kinds, exact historical/current anchors, capability refusal, unavailable
facts, continuation integrity, legacy grammar, source work and complete-packet
budgets. These source regressions do not establish live DFHack compatibility,
game effects, production admission or full workspace qualification.

`projected_entities_mcp_tests.rs` adds complete-turn pagination for entity,
spatial, relationship, ranked and event pages, retaining query catalogs and
pending game work. It also covers required-row refusal, failed-query observation
basis, historical authority and deferred-context checks. Pure family regressions
cover exact continuation replay, oversized required rows, smaller terminal
packets, empty results and publisher errors. Focused helper and renderer execution is recorded
separately from full public-facade execution in `IMPLEMENTATION_STATUS.md`.
