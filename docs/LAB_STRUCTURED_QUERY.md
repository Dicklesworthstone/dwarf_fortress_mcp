# Structured queries in the default laboratory MCP runtime

The default `serve` runtime accepts stateless `dfmcp.query/1` requests through
the existing `fortress_query` tool. This exposes the same typed entity,
aggregate, lexical and graph executors used by the read-only development
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
`expected_anchor`. These six query kinds are available:

| Kind | Purpose |
| --- | --- |
| `entities` | Typed filters, explicit field projection, deterministic ordering and source-bound pages |
| `inspect` | Exact entity ID and generation, with explicit selected fields |
| `aggregate` | Counts, grouping, and exact integer or fixed-scale numerical statistics |
| `search` | Bounded lexical retrieval over selected kinds, fields and labels |
| `traverse` | Bounded observed graph traversal and a witnessed path to an optional target |
| `dependencies` | Prerequisite order, cycle components, cycle blockers and an unweighted critical chain |

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

## Relationships and dependencies

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

For entity or search pages, copy the returned `continuation` string into
`request.query.continuation`. Keep the query selection unchanged. Entity
continuations also bind selected fields, ordering, source anchor and session.
The requested page width may change. After the current world advances, resume
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
the six kinds, exact historical/current anchors, capability refusal, unavailable
facts, continuation integrity, legacy grammar, source work and complete-packet
budgets. These source regressions do not establish live DFHack compatibility,
game effects, production admission or full workspace qualification.
