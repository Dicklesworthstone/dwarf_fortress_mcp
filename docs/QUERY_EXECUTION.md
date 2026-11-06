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
