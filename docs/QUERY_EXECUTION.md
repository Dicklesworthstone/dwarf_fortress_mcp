# Bounded canonical query execution

`dfmcp_world::execute_query` and `execute_bounded_query` use the `query_page`
executor. The private evaluator remains the reference implementation of typed
predicates, deterministic ordering, row limits, and whole-row byte limits.

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
`budget_exceeded`. The enclosing runtime remains responsible for the complete
Agent Turn publication budget.

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

## Bounded retained-event windows

The shared stateless `dfmcp.query/1` executor accepts `kind: "events"` on the
default laboratory's structured wrapper and existing structured live-read
consumers. The closed query accepts optional `from_tick`, `through_tick`,
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
leaves the cursor at the last published row. The enclosing runtime still checks
the full Agent Turn envelope and current authority.

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
