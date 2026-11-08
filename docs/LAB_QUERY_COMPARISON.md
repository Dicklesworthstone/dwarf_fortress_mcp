# Exact selected endpoint changes in the laboratory

The default laboratory `fortress_query` tool can compare an entity selection
at two exact session-owned worlds. An agent can inspect which selected jobs,
units, buildings or items changed after a plan or bounded simulation step
without downloading both complete projections and comparing them itself.

The comparison uses the same entity selection, presence, provenance and
generation rules as the existing foreground query baseline engine. It does
not allocate a baseline, register a watch, acquire a native observation or
advance the game. Its scope is the laboratory reference world; it establishes
no live-game compatibility, production admission or mutation authority.

## Request

Pass a JSON string as the existing `mode` argument of `fortress_query`, with
the owning `session_id` as usual:

```json
{
  "mode": "query_changes",
  "before": "<exact earlier state hash>",
  "after": "<exact later state hash>",
  "select": {
    "kind": "entities",
    "kinds": ["unit"],
    "fields": ["profession", "alive", "labor.BREW"],
    "order": "id_ascending"
  },
  "limit": 4
}
```

Replace both placeholders with the complete, 64-digit lowercase hashes from
this session's observed or retained worlds. `before` is required. Omitting
`after` selects the session's exact current world while the runtime owns the
session. An unavailable or collected hash is refused; no later world is
substituted and the request does not pin retention.

The closed wrapper accepts only `mode`, `before`, optional `after`, `select`,
optional `limit` and optional `continuation`. The selection accepts only
`kind: "entities"`, `kinds`, the existing typed `where` filter, `fields` and
`order`. Its complete rows are acquired internally; selectors cannot supply
their own limit or continuation. Duplicate wrapper or selection fields are
rejected. Entity field names, kinds, predicates and ordering use the existing
[structured-query contract](LAB_STRUCTURED_QUERY.md).

For example, select currently suspended jobs at each endpoint with:

```json
{
  "kind": "entities",
  "kinds": ["job"],
  "fields": ["state", "worker"],
  "where": {
    "op": "compare",
    "field": "state",
    "comparison": "eq",
    "value": {"type": "text", "value": "suspended"}
  }
}
```

This is a selection over fields actually present in the canonical source.
Requesting an unsupported or unrecorded field does not create that observation.
Unavailable facts remain explicit and cannot satisfy a typed predicate.

## Meaning of the result

The result has `mode: "query_changes"` and
`kind: "historical_changes"`. `basis` is the complete earlier anchor and
`anchor` is the complete later anchor. `current_anchor` and the Agent Turn
anchor identify the current session world. `historical` is true when the later
endpoint is itself historical. An old source tick never renews current grants.

Changes are ordered by numeric entity ID, then generation:

| Change | Meaning |
| --- | --- |
| `entered_result` | This generation matches at the later endpoint only. |
| `left_result` | This generation matches at the earlier endpoint only. |
| `changed_in_result` | Both endpoints select this generation, and its label or selected fact semantics differ. |

Each record contains its complete available `before` and `after` rows. Leaving
a filtered selection is not proof that an entity died, was deleted or became
absent from the whole fortress. Reused numeric IDs yield a departure for the
old generation and an arrival for the new generation.

Presence, source kind and epistemic-class changes remain visible even if the
underlying value is equal. Observation ticks, source digests and revisions
alone are counted in `provenance_only_refreshes`; they do not manufacture
selected-value changes. Unselected fields are outside the comparison.

`matched_before`, `matched_after`, `change_count`, `change_counts` and both
selected-result digests describe the complete comparison. `returned`,
`truncated` and `continuation` describe page delivery. An empty page with no
changes proves only that the selected endpoints agree. Intermediate changes,
including changes that were later reversed, remain unknown. Coverage remains
explicitly partial and `eligible_as_mutation_evidence` is false.

## Continuation and source identity

Copy the returned token to the wrapper's `continuation`, keeping the same
resolved endpoint hashes and selection. The page width may change. The token
binds both complete anchors, selected-result digests, selection, session and
current authority anchor, under a laboratory-specific identity domain. It is
an identity checksum, not an authorization credential.

Finish the pages before advancing or restoring the session world. If the
current anchor changes, start a new comparison without the old token; explicit
`before` and `after` can still select the original worlds while retained and
in the current epoch. A token cannot resume a different pair, selection,
session or native archived comparison.

Both endpoints must belong to the current fortress and observation epoch,
proceed forwards in sequence and game tick, and be no later than the current
anchor. A conflicting state at the same cursor, a reversed pair, a restore
boundary, or an invalid canonical hash is refused instead of being presented
as a mass arrival or departure.

## Bounds and authority

Current whole-fortress Query authority is checked before either source is
examined and again before returning the result. Revoked, expired, exhausted,
entity-scoped or cancelled authority is not revived by historical selection.

Distinct complete canonical sources share one record and byte-work budget,
even when the selected page contains one row. Equal complete endpoint anchors
charge one source; both supplied snapshots must still pass source bounds and
canonical-hash verification. `source_work` reports the distinct source record
count and canonical-byte size. The full foreground operation shares the
request's wall-time allowance.

The existing endpoint engine limits each selected endpoint to 256 rows and
256 KiB of serialized rows, acquired through at most 64 bounded query pages
and 2,000,000 source-row visits. The output page limit is 1–128, further
narrowed by the session budget, and defaults to at most 16. The request has
a 64-KiB ceiling and the continuation a 128-byte ceiling.

The helper reserves its source and coverage metadata before allocating the
change page's output allowance. The enclosing runtime also budgets the
complete Agent Turn. Whole before/after rows and their cursor are published
together. A change too large to fit, insufficient complete-source budget, or
an oversized final packet produces `budget_exceeded`; it does not publish a
clipped change or advance an invisible continuation.

## Implementation and verification scope

`lab_query_comparison.rs` validates the closed wrapper, exact-source/current
lineage, joint source budget and presentation. It calls the existing
`semantic_query::compare_endpoints` implementation; comparison semantics are
not reimplemented and neither process-local baseline nor watch storage is
accessed.

The focused tests cover source and current-anchor separation, entity-generation
reuse, filtered departures, unknown facts, provenance-only refreshes, same
endpoints, exact hashes, restore and fork rejection, authority expiry and
revocation, pagination binding, native-token domain separation, joint source
budgets, complete selection bounds and whole-row output. Public-facade tests
add session lookup, retained-world ownership and final Agent Turn publication.
These tests do not establish live DFHack or full-workspace qualification.

## Transcript replay

A transcript that successfully consumes a comparison continuation is marked
nonreplayable: that cursor binds the original session, while replay creates a
new session. The replay log states this limitation instead of reporting a false
semantic divergence. First-page comparisons still use exact retained anchors.
