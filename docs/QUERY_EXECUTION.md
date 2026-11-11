# Bounded canonical query execution

`dfmcp_world::execute_query` and `execute_bounded_query` use the `query_page`
executor. The private evaluator remains the reference implementation of typed
predicates, deterministic ordering, row limits, and whole-row byte limits.

## Independent live source and response budgets

The shared live query publisher accepts the original authorized source context
and a separate byte allowance remaining after the enclosing Agent Turn. Entity,
spatial, related, ranked, aggregate, lexical and event executors retain their
source limits while active-watch metadata is reserved from the response.
This applies at the actual protocol-1.1, jobs, operations, map, spatial/1.6 and
spatial/1.8 handler boundaries, not only when an executor is called directly.
A compact count or page can inspect a larger admitted source without requesting
that source-sized output. The source and response each remain subject to their
own existing limits; passing a larger response allowance cannot expand source
bytes, record count, grants, cancellation or token authority.

Compound aggregates still refuse an incomplete total. Paged queries retain
whole rows and unchanged source/session/selection continuations. Retained
watch summaries remain identical and visible; stateless queries do not sample
them. Stateful watch and baseline operations keep their existing atomic
publication context, and the enclosing runtime validates the complete final
Agent Turn after the shared publisher accepts its own bounded result.

## Continuation contract

Treat returned continuations as opaque. A page is bound to the fortress,
observation epoch and sequence, game tick, canonical state hash, kind selectors,
predicate tree, and ordering. Changing the requested result set or resuming on a
different snapshot refuses with `ERR-STALE-ANCHOR`. The snapshot's stored hash is
verified before execution. Kind selectors are treated as a set; custom kind names
remain distinct from built-ins. Predicate tree order is intentionally part of
this version's query identity.

Page width and output-byte budget may change between pages. An oversized first
row returns `ERR-BUDGET-EXCEEDED` instead of emitting a non-progressing cursor.

The `q1` format replaces query use of the old offset-only `cont` format. Legacy
query continuations return `ERR-CURSOR-GAP`; restart the query. Delta continuation
encoding is unchanged.

This digest binding is **not authentication or a capability**. It detects edits
and unintended reuse, but does not prevent a caller from computing a fresh
commitment. The adapter must authorize every request and bind client-facing
continuations to session ownership. A continuation cannot grant read authority.

## Work and input limits

Before predicate execution, the evaluator refuses a conservative bound greater
than 4,000,000 entity-selector/predicate operations. This prevents individually
legal entity and predicate limits from multiplying into billions of evaluations.
It is an operation-count guard, not a measured wall-time guarantee. Canonical
snapshot hash verification remains a full snapshot operation.

Predicate shape/value limits remain in force. Query identity encoding additionally
bounds aggregate bytes to 256 KiB and nested kind names to 128 bytes. No predicate
is weakened, normalized to true, or partially evaluated to evade a budget refusal.

## Laboratory lexical retrieval

The laboratory's existing `fortress_query` mode
`{"mode":"search","text":"building","limit":5}` uses the bounded in-memory
`FrankenSearchEngine` lexical prototype. This is not an owned FrankenSearch
integration; roadmap bead `dfmcp-reality-wiring-proof-4bf.9` remains open.

Entity documents include their labels, canonical kind names, and only field
values accepted by `Fact::known_value()`. Legacy known values and consistent
explicit `Known` values remain searchable. Absent, unknown, unsupported, omitted,
redacted, stale and contradictory known-value records contribute neither field
names nor compatibility values to matches, relevance scores or snippets. A
label or kind can still match an entity whose fields are unavailable. Retained
event summaries and values keep their existing lexical representation.

Each snapshot indexing operation builds a complete bounded replacement before
publishing it. Reindexing after a fact becomes unavailable withdraws its old
terms. Equal scores retain canonical snapshot record order: ascending entity
identity, then ascending retained event identity. The scorer and existing count,
byte, token, query and result limits are unchanged.

The public laboratory route rechecks current Query authority and names the
source anchor in its normal Agent Turn. Search results are advisory retrieval
over the supplied projection. Neither an empty result nor a known value alone
proves complete-world absence, source authenticity or mutation eligibility.

## Filtered operational aggregates

The stateless `dfmcp.query/1` aggregate accepts the same typed `where` grammar as
entity queries. This permits a count or exact numeric total for a selected
population without downloading every record. For example:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "aggregate",
    "kinds": ["unit"],
    "where": {
      "op": "all",
      "args": [
        {
          "op": "compare", "field": "alive", "comparison": "eq",
          "value": {"type": "bool", "value": true}
        },
        {
          "op": "compare", "field": "labor.MINING", "comparison": "eq",
          "value": {"type": "bool", "value": true}
        }
      ]
    }
  }
}
```

Only rows whose complete predicate is True contribute to groups and metrics.
`selection` separately reports the kind-selected population, matches, known
False exclusions and Unknown exclusions. `complete_within_projection` means
every selected row's filter was decidable; it does not establish full fortress
coverage, source authority or a mutation precondition. Missing, unavailable,
redacted, stale and contradictory filter values remain Unknown under negation.
Boolean combinations use the same three-valued evaluator as entity selection.

Metric availability is a separate question: a matching row with an unavailable
numeric field remains a selected row and contributes to that metric's existing
excluded-sample counts. A metric with no eligible samples retains a null sum and
mean. Filtering runs before grouping, so excluded rows do not consume the group
cardinality allowance or reveal their group values.

The entire predicate is validated before scanning. A conservative work bound
charges predicate validation and evaluation together with grouping and metrics
against the existing 4,000,000-operation ceiling, including rows that might be
excluded. Results publish atomically under the caller's byte/token budget. The
query identity covers the filter, exact snapshot and session. Unfiltered
requests preserve their previous result fields and values.

The filter is available to existing structured live-read consumers. It performs
no bridge read, creates no watch and grants no effect authority.

### Combine aggregate membership with relationships and geometry

Optional `relation` and `spatial` objects on an aggregate use the same closed
grammar as compound ranking. These can be supplied independently. Source kinds
and three-valued `where` select candidates first; a requested relationship
requires at least one recorded positive edge to a selected target; a requested
spatial selector then checks the source entity's named point/cuboid geometry.
Only this final population reaches grouping and exact numeric metrics.

This permits grouped quantities within a stockpile, workshop relationship or
map area in one request. Targets have their own kinds, IDs and independent
three-valued `relation.where`; they need not lie inside the spatial region.
The aggregate retains its existing distinction between builtin entity kinds
and explicit `other:` names, and between builtin edge kinds and
`custom:` names. The shared relationship matcher exposes an opt-in exact-name
normalization for this consumer; existing `rq1` and `rk1` normalization and
cursor identities are unchanged.

A source contributes once regardless of parallel edges or multiple matching
targets. `outgoing`, `incoming` and `undirected` use the existing opposite-
endpoint rules, with undirected self-loops recorded once. One positive edge
suffices even if another endpoint is missing or has an Unknown target filter.
No positive match plus an undecidable relevant endpoint is Unknown; other
candidates are `no_observed_match`, never proof that unrecorded relations
are absent. Unknown geometry is distinct from known outside geometry.
All inclusive point/cuboid semantics are shared with spatial selection.

Compound results add these staged counters to `selection`:

| Counter | Population |
| --- | --- |
| `selected_entities`, `excluded_false`, `excluded_unknown`, `matched_source_filter` | Source kinds and source predicate, before relation or geometry |
| `relation_matched`, `unknown_relation`, `no_observed_match` | Source-predicate True candidates at the relationship stage |
| `matched_edge_incidences`, `missing_target_records`, `unknown_target_filters` | Oriented incidences after source filtering, including extra uncertain edges on positive matches |
| `selected_edges` | All source edges of the requested edge kinds before endpoint filtering |
| `spatial_matched`, `outside_spatial_relation`, `unknown_geometry` | Candidates reaching the geometry stage |
| `matched` | Final members contributing to groups |

The top-level `matched` and each group count are exact for selected known
members. Unavailable numeric samples remain in each metric's excluded counts;
they neither remove membership nor become zero. An empty metric still has a
null sum and mean. `coverage.totals_scope` is
`selected_known_members_only`. Selection completeness is false if any earlier
membership stage has Unknown exclusions, even when later predicates might
exclude such rows. It concerns only decidable membership in the projection,
not metric availability, complete-world coverage, edge-source authenticity,
terrain, accessibility or mutation authority. Aggregate evidence remains
inferred. `absence_proven` is always false.

Compound admission uses the existing complete-source structural preflight
before hashing: all entities, target records, edges, chunks, retained events,
unselected fields and unavailable backing representations count. The shared
100,000-record, 16-MiB, depth-64 and 4,000,000-work limits apply alongside the
negotiated budget. Source/target predicate validation and evaluation, relationship
incidences, geometry, group lookup and metrics are conservatively charged before
selection. The temporary relationship index is O(N); group state remains bounded
by 128 groups and 16 metrics. `source_cost` reports the complete structural
estimate and charged work. The same deadline spans admission, selection,
group construction and serialization; enclosing runtimes also enforce their
full publication budget.

There is no aggregate cursor or partial group publication. Exceeding group
cardinality, result bytes/tokens or the enclosing Agent Turn allowance refuses
the complete operation. The existing request/session/source digest binds all
supplied selectors. Historical laboratory reads still require the exact
retained source and current whole-fortress Query authority. Omitting both
selectors leaves the established aggregate shape, result values and identity
unchanged.

`aggregate_selection_query_tests.rs` covers independent graph-direction and
coordinate oracles, parallel edges, source/target/geometry Unknowns, separate
metric availability, exact namespaces, complete-source/work/output refusal and
schema parity. `lab_structured_query_tests.rs` adds actual public-facade
grouped selection, prepared-work retention, exact history/current authority,
atomic full-packet refusal and default-budget starter-world spatial counts.

## Exact numeric entity ranking

`ranked_entities` selects an ordered page from a canonical source by one to
three distinct numeric facts. It is shared by semantic-query runtimes and the
default laboratory structured-query wrapper. This fills the field-ordering
portion of the DfQL plan without adding a tool, changing an effect family, or
introducing a runtime dependency.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "ranked_entities",
    "kinds": ["unit"],
    "order_by": [
      {"field": "stress", "numeric_type": "i64", "direction": "descending", "max_age_ticks": 20},
      {"field": "age", "numeric_type": "u64", "direction": "ascending"}
    ],
    "fields": ["profession"],
    "limit": 4
  }
}
```

Field names refer to facts actually published by the selected source; this
example does not add stress or age to a native projection. The existing typed,
three-valued `where` predicate and kind selection run before ranking.

Each key declares `i64`, `u64` or `fixed` plus `ascending` or `descending`.
Fixed keys require a scale in 0..=u32::MAX and compare their signed units only
when that scale matches exactly. Non-fixed keys cannot supply a scale.
There is no numeric coercion, floating-point conversion or descending-order
negation; i64::MIN, i64::MAX and u64::MAX retain their exact ordering. Keys are
compared lexicographically in request order, with ascending entity ID as the
final tie-break. A field cannot occur twice.

Every key must pass `Fact::known_value()`, match its declared numeric type and
scale, and have an observation tick no later than the source tick. Optional
`max_age_ticks` is inclusive and applies independently to each key. Without
an age bound, earlier Known facts remain usable with their original timestamp
and epistemic class. Missing, unavailable, conflicting, future, over-age,
mistyped and differently scaled facts never become zero or a synthetic extreme.
The entire entity is excluded from ranking if any key is unusable.

`selection` separates kind-selected entities, filter False, filter Unknown,
and `unknown_rank`. `unknown_rank_reasons` counts the first unusable key per
entity, not every bad fact. Selection completeness concerns only the supplied
canonical projection and is independent of page truncation. Rows always include
the ranking facts, even when those fields were not explicitly projected.
Their presence, values, source digests, ticks and epistemic classes use the
shared fact encoder. Inferred and assumed values retain those classes.
Ranking is advisory and cannot certify a causal priority, absence or mutation
precondition.

A bounded binary heap keeps at most K candidate references for the requested
page, with O(N log K) ordering work and O(K) retained candidates. It does not
sort or clone the entire population or retain a hidden mutable index.
`source_cost.maximum_retained_candidates` records the actual peak.
K defaults to four and is bounded by 128 and the negotiated entity allowance.
Complete-source preflight still covers unselected entities, edges, chunks,
events, fields, nested values and provenance before hash verification.
The existing 100,000-record, 16-MiB, depth-64 and 4,000,000-work ceilings apply
alongside negotiated limits. Predicate, lookup, comparison, heap and final-page
sorting costs are charged before selection; wall time is checked throughout.

The `rk1` continuation binds the session ID, complete source anchor, canonical
kind set, typed predicate, normalized projected fields, key sequence and every
key's type, direction, scale and age bound. It carries the last returned
entity's ID and generation. Page width and output budget may change. The
executor re-derives that row's ordering values from the same verified immutable
source; a missing, ineligible, wrong-generation or final-position target
returns `cursor_gap`. Modified or differently bound tokens return
`stale_anchor`. Digest integrity is not authentication: current Query
authorization is repeated on every request.

Output admits complete rows with both projected and ranking facts. If the next
ranked row cannot fit, the prior page ends before it and the continuation
preserves that position; no smaller later row is substituted. An oversized
first row returns `budget_exceeded`. The semantic publisher reserves active-watch metadata against the output cap
without replacing the original source-read byte allowance. The enclosing
runtime still has to admit the complete Agent Turn. The laboratory supplies its
immutable complete-packet size check during row selection, so a large requested
page becomes a smaller resumable prefix without discarding ranking facts or
retained work. Historical laboratory pages explicitly select the
retained source hash while current grants and the current Agent Turn anchor
remain authoritative.

### Rank within an observed relationship

Add optional `relation` to `ranked_entities` to rank only entities that have a
recorded positive relationship to selected targets. It uses the same closed
selector as `related_entities`: edge kinds, direction, target kinds/IDs and an
independent typed target `where`. For example, select the highest-priority jobs
requiring an item whose recorded eligibility is known true:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "ranked_entities",
    "kinds": ["job"],
    "relation": {
      "edge_kinds": ["requires"],
      "target_kinds": ["item"],
      "where": {
        "op": "compare", "field": "eligible", "comparison": "eq",
        "value": {"type": "bool", "value": true}
      }
    },
    "order_by": [
      {"field": "priority", "numeric_type": "i64", "direction": "descending"}
    ],
    "fields": ["suspended"],
    "limit": 4
  }
}
```

These names must be present in the canonical projection. Source `where` runs
first; the positive relationship selector runs next; usable numeric ordering
facts determine the final ranked population. Each stage retains its own
exclusions. `selection.relation_matched` counts source-filter matches with a
positive witness before ranking. `unknown_relation` and `no_observed_match`
separate undecidable endpoints from the lack of a recorded match. Missing-target
and unknown-target counts describe oriented edge incidences after source
filtering, including uncertain additional edges on a positively selected row.
`unknown_rank` applies only to relation matches. One positive witness suffices
even when other endpoints are unknown.

Rows keep the existing `ranking` facts and add the same bounded `relations`
evidence as standalone relationship queries: at most four exact edge revisions
and target generations, with explicit omitted counts. Complete-packet page
fitting includes both forms of evidence. A required row that cannot fit is
refused rather than skipped. Ranking does not turn an unobserved relation into
negative evidence or authorize mutation.

Source and target predicates are each evaluated at most once per entity, and
the shared matcher scans edges once. Its temporary source/target index uses
O(N) records bounded by complete-source admission; the ranking heap still
retains at most K candidates. `source_cost` reports the relation index sizes
separately from `maximum_retained_candidates`. Relation work shares the same
prepaid work and wall allowances. Narrow targets, filters or pages cannot hide
source costs.

The normalized relation selector participates in `rk1` identity, including
its target predicate. Reordered or duplicate set members are equivalent, but
adding, removing or changing the relation invalidates an existing cursor. A
cursor position must itself satisfy the complete selection. Omitting `relation`
or passing null preserves the established ranked response and continuation
identity; the standalone `rq1` relationship contract is unchanged.

### Combine location, relationships and ranking

Optional `spatial` restricts the ranked source population by the same inclusive
point/cuboid rules as `spatial_entities`. It contains `region`, `geometry` and
optional `relation` (`intersects`, `within` or `contains`). Its nested
`spatial.relation` describes geometry; top-level `relation` continues to select
recorded graph edges. Either selector can be supplied alone or both together.
For example, find high-priority jobs in a workshop area that require a selected
item, without downloading and joining independent result sets:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "ranked_entities",
    "kinds": ["job"],
    "where": {
      "op": "compare", "field": "suspended", "comparison": "eq",
      "value": {"type": "bool", "value": false}
    },
    "relation": {
      "edge_kinds": ["requires"],
      "target_kinds": ["item"],
      "target_ids": ["100"]
    },
    "spatial": {
      "region": {"min": [10, 20, 5], "max": [30, 40, 5]},
      "geometry": {"kind": "point", "field": "position"},
      "relation": "within"
    },
    "order_by": [
      {"field": "priority", "numeric_type": "i64", "direction": "descending"}
    ],
    "fields": ["suspended"],
    "limit": 4
  }
}
```

The fields, item and relationships must already exist in the chosen projection.
Geometry applies to returned source entities only. An item outside the region,
or one without any coordinate facts, can still be a valid relation target.
The staged selection order is source kinds/`where`, optional positive relation,
optional spatial geometry, then usable ranking keys. `spatial_matched`,
`outside_spatial_relation` and `unknown_geometry` count the population reaching
that stage; `unknown_geometry_reasons` records the first unusable geometry fact.
`unknown_rank` counts only entities passing the earlier stages. Filtered-out
entities do not create downstream geometry or ranking failures, while each
earlier stage retains its own Unknown exclusions. These counts and the exact
matched population cover the entire source, including rows before a cursor.

Each row includes `geometry` with its bounds and the exact deciding coordinate
facts, alongside `ranking` and any `relations` witnesses. These required facts
remain present when `fields` omits them and count toward complete-packet fitting.
The geometry evaluator is shared with standalone spatial queries; it adds a
bounded coordinate check per candidate, no terrain scan and no additional
population index. Full-source preflight and conservative work charging happen
before any selector can shrink the result.

Region bounds, geometry kind/field names and normalized spatial relation are
bound into the ranked continuation. Its saved row must still satisfy every
selector on the same source. Null or absent `spatial` leaves existing ranked
identity and output unchanged, including relationship-only requests. Standalone
`sq1` cursors preserve their established identity. Unknown geometry never proves
outside occupancy, accessibility, collision safety or complete-world absence;
geometry and relationship evidence remain advisory.

## Spatial entity intersection and containment

The shared stateless `dfmcp.query/1` executor accepts `spatial_entities` through
the default laboratory structured wrapper and existing structured live-read
consumers. It selects entities using explicitly named coordinate facts in the
published source. For example, select units whose known positions fall within
one inclusive region:

~~~json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "spatial_entities",
    "kinds": ["unit"],
    "region": {"min": [10, 20, 5], "max": [30, 40, 5]},
    "geometry": {"kind": "point", "field": "position"},
    "relation": "within",
    "fields": ["profession", "alive"],
    "where": {
      "op": "compare", "field": "alive", "comparison": "eq",
      "value": {"type": "bool", "value": true}
    },
    "limit": 4
  }
}
~~~

For entities with a recorded footprint, use
`{"kind":"cuboid","min_field":"min","max_field":"max"}` as `geometry`. These are
explicit source field names; choose the names actually present in the canonical
projection. Both fields must contain typed coordinates, and they must have
distinct names. No missing location or footprint is guessed from a label,
relationship or another field.

All bounds are inclusive signed 32-bit coordinates in all three dimensions.
Reversed region bounds are rejected, and comparisons do not compute potentially
overflowing volumes or offsets. The relations are:

| Relation | Match |
| --- | --- |
| `intersects` (default) | Entity geometry and query region share at least one coordinate |
| `within` | Every coordinate of the entity geometry lies in the query region |
| `contains` | The entity geometry contains the entire query region |

A point is a cuboid whose minimum equals its maximum. Geometry is usable only
when each named fact has a consistent Known coordinate value, was not observed
in the source's future, and forms a valid cuboid. Missing, absent, omitted,
unsupported, redacted, stale, contradictory, mistyped, future or inverted
geometry is counted in `selection.unknown_geometry` with bounded reason counts.
It never becomes an outside-region finding. The same three-valued `where`
grammar used by entity queries runs on geometric hits; known False and Unknown
filter exclusions are counted separately.

Each row includes its exact entity ID, generation and revision, selected fields,
and the named geometry facts with presence, source digest, observation tick and
epistemic class. Derived or assumed coordinates retain those classes. Known
facts from earlier ticks retain their recorded freshness; selection does not
promote them to current authoritative observations. Geometry facts are included
even when `fields` is empty. Unavailable compatibility values are not exposed.

Ordering is ascending entity ID by default, or `id_descending`. The `sq1`
continuation commits to the complete source anchor, session, region, geometry
field names, relation, kind and projected-field sets, typed predicate, order,
and last returned entity ID and generation. Page width may change; all other
selection semantics must remain the same. A missing or final cursor target is
refused. Kind matching uses the canonical encoded name so equivalent source
encodings cannot produce different answers under the same hash. Cursor digests
are integrity commitments and cannot grant read authority.

There are at most 32 kind selectors, 32 projected fields and 128 rows per page
(default four). Complete-source preflight charges entities, edges, chunks,
events, field names, provenance and all nested values, including unselected or
unavailable data. The source must fit the session record budget and ceilings of
100,000 records, depth 64 and 4,000,000 work units. Its byte ceiling is the smaller
of the negotiated `max_bytes` allowance and 16 MiB, enforced before hashing or
allocating a nested-value scan frontier. The conservative source estimate includes
structural overhead and can exceed the compact canonical encoding size; narrowing
the region, projected fields or page width does not waive complete-source admission.
Predicate, selector and projection work are included before scanning, and the wall
deadline is checked throughout. Source identity and hash are verified before
results are returned. The laboratory wrapper also applies its canonical source-byte
and full-packet budget admission.

Whole rows fit both the byte and token budgets. A lower-bound projection pass
refuses oversized fields before building a large typed JSON tree. If a later
row cannot fit, the published cursor remains at the last returned row; the next
request cannot silently skip the large row. An oversized first row returns
`budget_exceeded`. The laboratory includes its complete Agent Turn and retained
work catalogs while choosing the page. The enclosing runtime still enforces the
final publication deadline and budget.

Coverage names `named_known_geometry_in_canonical_projection`.
`selection_complete_within_projection` means the selected geometry and relevant
filters were all decidable; `truncated` and `continuation` separately describe
page delivery. Neither field establishes complete fortress occupancy. Terrain,
accessibility, collision safety, source authenticity and mutation eligibility
are not inferred, and `absence_proven` remains false even for an empty result.
Historical laboratory reads retain the exact source while rechecking authority
at the current tick.

`spatial_entity_query_tests.rs` contains inclusive coordinate-oracle, provenance,
Unknown, canonical kind-alias, source/session/selector binding, whole-row budget
and authority regressions. `lab_structured_query_tests.rs` includes a public
facade regression across current publication, retained-source pagination and
grant expiry. These tests define the component contract; they are not native
DFHack admission evidence.

## Selecting entities by observed relationships

The shared stateless `dfmcp.query/1` executor accepts `related_entities` through
the default laboratory structured wrapper and existing structured live-read
consumers. This selects entities by a positive relationship to another recorded
entity, without first downloading the whole graph. For example, find suspended
jobs that require an item whose recorded `eligible` fact is true:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "related_entities",
    "kinds": ["job"],
    "where": {
      "op": "compare", "field": "suspended", "comparison": "eq",
      "value": {"type": "bool", "value": true}
    },
    "relation": {
      "edge_kinds": ["requires"],
      "direction": "outgoing",
      "target_kinds": ["item"],
      "where": {
        "op": "compare", "field": "eligible", "comparison": "eq",
        "value": {"type": "bool", "value": true}
      }
    },
    "fields": ["suspended"],
    "limit": 4
  }
}
```

The field and kind names must exist in the supplied canonical projection; the
query does not produce missing relations or facts. `where` outside `relation`
filters the returned entity. `relation.where` filters the opposite endpoint.
Both use the existing typed, three-valued field grammar. `target_ids` can
additionally restrict that endpoint to at most 64 canonical positive decimal
u64 IDs. Empty source-kind, target-kind or target-ID sets impose no restriction.
At least one explicit edge kind is required, with at most 32 edge kinds. Kind
selectors compare canonical names, including reserved-name aliases.

`outgoing` follows source → target and is the default. `incoming` follows
target → source; `undirected` accepts either direction. Each selected entity
appears once even when several edges match. Distinct parallel edge records
count separately, and an undirected self-loop counts once. The result preserves
up to four witnesses per row in ascending edge-ID order, plus the exact matched
edge count and omitted witness count. Each witness names the edge's full
128-bit ID, revision, kind and direction, and the opposite endpoint's ID,
generation and revision. These identities resolve in this exact snapshot;
they do not establish historical causal attribution when an ID was reused.
Selected source fields retain their ordinary Fact presence and provenance.

A missing target record or an Unknown target filter is reported separately.
A witnessed positive match selects the source even if another edge has an
unknown target. Without a positive witness, an undecidable relevant endpoint
makes the relation Unknown. `no_observed_match` counts other source candidates;
it describes only the supplied projection. There is no `exists: false` selector
or negative relationship proof. Unavailable facts stay Unknown under field
negation. Aggregate missing-target and unknown-target counters count oriented
source/edge incidences; one undirected edge can contribute to both endpoints.

Coverage names `positive_relations_in_canonical_projection`.
`selection_complete_within_projection` means every kind-selected entity's
membership was decidable within those recorded facts. Pagination is described
separately by `truncated` and `continuation`. `relation_domain_complete`,
`absence_proven` and `eligible_as_mutation_evidence` remain false, and edge-source
authenticity remains unestablished. No result, including an empty one, proves
that an unrecorded dependency, assignment or containment relation is absent.

Ordering is ascending or descending entity ID; the default page has four rows
and the maximum is 128. An `rq1` continuation binds the session, complete source
anchor, source kinds and filter, edge kinds and direction, target kinds, IDs
and filter, projected fields, ordering and the last row's generation. Selector
sets are normalized; predicate-tree order remains part of identity. Page width
may change. A missing or final cursor position is refused. Every call still
checks current authority; a cursor supplies no capability or source retention.

Source admission uses the same bounded full-snapshot accounting as spatial
selection: all entities, edges, events and nested values must fit the source
record allowance, the smaller of the negotiated byte budget and 16 MiB, depth
64, and the 4,000,000-work-unit ceiling before hashing and selection. The byte
measure is conservative accounting for in-memory canonical data, not an exact
wire encoding. Narrow target filters cannot exempt unselected source records.
Source and target predicates are evaluated at most once per entity, then edges
are scanned once; no traversal is repeated for every candidate. Conservative
selector/predicate work is charged before evaluation, and the wall budget is
checked throughout scanning and publication.

Whole rows and their witnesses fit both byte and output-token budgets. If a
later row cannot fit, the cursor remains at the last returned row; an oversized
first row is refused. The laboratory fits the complete Agent Turn and retained
work catalogs during selection, without reducing the authorized source budget.
The enclosing runtime also checks the final complete packet.
The component regressions include an independent incidence oracle over all
512 directed graphs on three vertices in all three directions, parallel edges,
self-loops, full-width IDs, unavailable facts, missing endpoints, canonical
aliases, source/session/selector binding and whole-row/source-budget refusal.
The public laboratory regression resumes a retained source after the current
world advances and verifies current grant expiry.

## Bounded retained-event windows

The shared stateless `dfmcp.query/1` executor accepts `kind: "events"` on the
default laboratory's structured wrapper and existing structured live-read
consumers. The closed query accepts optional `source`, `from_tick`, `through_tick`,
`kinds`, `subjects`, `fields`, `order`, `limit` and `continuation`. Tick bounds
are inclusive and default to zero through the source snapshot's tick. Reversed
windows and future end ticks are refused. Canonical event kinds are exact name
sets; subject IDs are canonical positive decimal u64 strings. A subject filter
matches the ID recorded in the event, without inventing a current generation.

The query verifies its exact source and scans the complete retained projection
before producing deterministic `(tick, EventId)` order. `tick_descending`
reverses both components. Selected fields retain typed null, signed/unsigned
integers, fixed values, coordinates, entity IDs, bytes and nested values. A
requested field missing from the retained record has unknown presence. The
canonical event type lacks individual Fact provenance, so the result declares
that limitation and does not promote its values to authoritative observations.
The complete source anchor plus event ID identify each returned record. Text
remains tainted and no event query is mutation evidence.

An `ev1` continuation binds the session, every complete anchor component, the
time window, kind/subject/field sets, ordering and last returned `(tick, EventId)`.
Set ordering and duplicate selectors do not change identity. Page width may
change; every other semantic selection must remain the same. A cursor naming a
missing or final record cannot manufacture an empty successful continuation.
Like other query cursors, its digest is integrity binding, not read authority.

There are at most 32 kind selectors, 64 subjects, 32 selected fields and 128 rows
per page (default four). Source scanning is bounded by the session record budget,
100,000 retained events, 16 MiB of estimated event data and 4,000,000 value visits;
nested values are bounded to depth 64 before projection. The complete source
fits the session's record budget and the canonical snapshot byte ceiling, and
its hash is verified. The laboratory wrapper separately charges canonical
source bytes against its original session work budget; live response reservation
does not relabel an already-published source as oversized input.
Excluded events still consume scan work. Whole rows are paged under both byte
and token budgets. An oversized first row is refused; an oversized later row
leaves the cursor at the last published row. The laboratory fits the full Agent
Turn and retained work catalogs while selecting event rows, including empty
results. The enclosing runtime still checks the final envelope and current
authority.

Coverage is explicitly `retained_event_projection`. The retained tick extrema
do not establish a continuous retained interval. `history_complete`,
`absence_proven` and `eligible_as_mutation_evidence` remain false. Sources without
canonical events return an empty retained projection with those same limits.
The explicit `starter_fortress_history` laboratory scenario populates canonical
events at atomic publication boundaries and supplies bounded validated
`coverage.producer` metadata. The model's recorded/evicted/omitted counts do not
upgrade source authenticity or mutation eligibility; see `LAB_EVENT_HISTORY.md`.
Native event capture is unchanged. Laboratory exact historical reads recheck
grants at the current tick before selecting a retained snapshot.

`event_query_tests.rs` exercises filters, both chronological orders, full-width
IDs, exact field types, complete-row paging, source/session/selector binding,
historical authority prerequisites, corruption and scan limits.
`lab_structured_query_tests.rs` drives the public facade across retained history
and grant expiry. `live_spatial_server_tests.rs` exercises the actual live query
handler over its published canonical fixture without a bridge refresh.

### Retained native announcements

`source: "announcements"` selects a read view of existing protocol-1.1
announcement entities. The default `source: "world_events"` and its historical
cursor domain are unchanged. The announcement view orders reports by their
original native dates, preserves typed Fact provenance and unavailable states,
and keeps raw text tainted. Subject identity cannot be inferred from text.
Its distinct cursor domain binds the complete source and selection without
inserting canonical events or changing projection hashes.

The protocol-1.1 runtime attaches exact retention bounds and gap status from
the same already-published combined capsule; filtering and response pagination
do not alter that native coverage. Other consumers cannot infer native bounds
from entity rows and return unknown retained-suffix completeness. History
completeness, absence and mutation eligibility remain false in both cases.
The live runtime fits complete rows with their Agent Turn and native metadata
under the response allowance, while retaining the original complete-source
work budget. See `ANNOUNCEMENT_EVENT_WINDOWS.md` for requests, field semantics,
identity, coverage and bounds.

## Qualified entity quantities

The typed `Predicate::EntityQuantity` counts matching entity records or sums a
named unsigned quantity field across one canonical entity-kind domain. It uses
one exact field-equality selector, a comparison operator and an unsigned target.
For example, `Item` with `type_key == "BED"`, quantity field `stack_size`, and
`Ge 1` asks whether the observed roster contains at least one BED stack unit.
Omitting the quantity field counts records, which is a different quantity.

Proof through `PredicateEvidence` requires independently granted **complete**
coverage of that kind. Every member needs a qualified known selector; every
matching member needs a qualified known `U64` quantity when summing. Unavailable,
contradictory, future or ineligible source facts, invalid quantity types and
checked-sum overflow yield `Unknown`. A qualifying subtotal never bypasses
missing evidence elsewhere in the domain. An empty complete domain sums to zero;
an empty incomplete projection does not establish zero.

Canonical kind names determine membership, including reserved-name aliases.
Combined aggregate work is bounded to 4,000,000 record/field visits before proof
evaluation. Query execution additionally charges repeated whole-domain scans
against its existing work limit. These are typed core predicates; a raw
`WorldSnapshot` query remains advisory and does not itself issue coverage or
source authority.

Predicate canonicalization, query-bound continuation identity and prepared-plan
archive encoding use appended tag 12. Existing tags and legacy archive bytes
remain unchanged. Cross-anchor MCP revalidation records a complete kind-domain
read: newly inserted entities, changed selectors, quantity changes and replaced
contributors invalidate it even if the resulting sum is equal. Its bounded
census refuses oversized worlds conservatively.

## Evidence

`query_page_tests.rs` covers all four ordering modes, repeated pages, changes in
page width and byte budgets, cross-query reuse, every anchor component, same-cursor
forks, malformed and edited tokens, legacy offsets, kind-name aliases, invalid
snapshot hashes, and aggregate input/work bounds. The public API truth-table suite
also resumes a returned bound cursor.

These additions are source implementation and regression tests, not live-bridge
admission, runtime qualification, or proof of a successful Rust test run.

The lexical regression additions in `dfmcp-world/tests/search_tests.rs`
exercise every unavailable/inconsistent presence, positive known values and
events, kind discovery, deterministic ties and withdrawal on reindexing.
`lexical_search_mcp_tests.rs` exercises the actual public laboratory
`fortress_query` path with a canonical fixture, current grants, Agent Turn
anchors and a later presence change. These are source regressions; Rust
execution and exact-source qualification remain pending for this increment.

## Filtered structured lexical search

The stateless structured `search` query accepts the same optional typed `where`
predicate as entity selection and aggregates. Agents can combine a name or text
search with recorded state without downloading and filtering every hit. For
example, search the published labels of living citizens:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "search",
    "text": "Urist",
    "kinds": ["unit"],
    "where": {
      "op": "compare", "field": "alive", "comparison": "eq",
      "value": {"type": "bool", "value": true}
    },
    "limit": 4
  }
}
```

Use this envelope as `request` inside the laboratory's `mode: "structured"`
wrapper, or as the existing structured live query. Only predicate True rows
enter text matching and ranking. False and Unknown are separate exclusions;
missing, omitted, redacted, stale, unsupported and contradictory facts do not
become matches through negation. Text in excluded rows cannot influence scores,
matching-field provenance, or text-availability counts.

Filtered results add `selection`: `selected_entities` counts the kind-selected
population, `matched_predicate` counts its True rows before text matching, and
`excluded_false` / `excluded_unknown` count the other outcomes. The top-level
`matched` still counts actual lexical hits. These counts cover the entire
projection on every page, including rows preceding a continuation. Selection
`complete_within_projection` means the filter was decidable for every selected
row; it does not prove complete fortress coverage or absence. Unfiltered queries
retain their previous response shape, ranking and query identity.

Predicate validation and evaluation share the existing 4,000,000-operation
allowance with term/field scans. The conservative charge precedes filtering,
kind exclusions and cursor selection; a false filter cannot make an otherwise
over-budget request admissible. The complete response, including selection
metadata and continuation, must fit the byte/token budget. Pages retain whole
rows in deterministic score/ID order. The existing `ls1` identity binds the
complete predicate, session and exact source, so adding, removing or changing
the filter invalidates an old continuation. Every page rechecks Query authority.

This is bounded lexical retrieval over published canonical facts. It adds no
native reads, effects, hidden index, semantic model or FrankenSearch dependency.

## Projected entity pages

Structured `entities` queries budget the requested field projection and its
complete result envelope. A large field omitted from `fields` no longer prevents
a small page from fitting, and the executor borrows canonical records instead of
cloning their unselected values. Whole-source record, structural byte, depth,
work and wall-time admission still apply before source hashing; selecting fewer
fields does not authorize a larger scan or conceal unavailable source data.

The page contains a complete ordered prefix. `returned` counts only published
rows, `matched` describes the selected source population, and the continuation
resumes immediately after the last published row. If the next complete row does
not fit, it is never skipped. A first row that cannot fit returns an explicit
budget error; narrowing `fields` or increasing output allowance can resume the
same request. Empty results also require the complete envelope to fit.

Existing `sp1`/`q1` cursor identities and all four ordering rules remain stable.
Page width may change while resuming the same exact source, session and selected
fields. Changed filters, ordering, source or session still invalidate the cursor.
The final page is considered even when removing its continuation makes it
smaller than the preceding prefix.

Callers can reserve response metadata independently of source admission and can
provide an immutable complete-packet size check while the page is selected. This
check cannot publish or advance session state. The enclosing MCP surface still
enforces its final Agent Turn budget; an incomplete structured row or a cursor
that advances past unpublished rows is never a successful response.

The same pure size check and bounded terminal-page lookahead also serve
`spatial_entities`, `related_entities`, `ranked_entities` and `events`.
Their own selectors, ordering, provenance, source budgets and cursor identities
are unchanged. Candidate rows are selected once under the original work
allowance; fitting does not repeatedly rescan the world with smaller limits.
Only an accepted complete prefix can be returned. A later oversized row is
never skipped, a rejected first row never creates an advancing empty page,
and an empty result still needs room for its complete packet. A candidate
without its terminal cursor may be examined as a size lower bound, but that
provisional candidate is never published as evidence or as a successful page.
Publisher errors abort the operation rather than being treated as a reason to
silently reduce its scope.
