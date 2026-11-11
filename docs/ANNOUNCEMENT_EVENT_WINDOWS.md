# Retained announcements in event queries

Use `fortress.query` with `kind: "events", source: "announcements"` to inspect
the protocol-1.1 reports already published in the session's canonical world.
This connects the existing retained-announcement reader to chronological event
windows, field projection and continuations. An agent can inspect recent
cancellations, arrivals or other report text without downloading every report
entity or interpreting entity IDs as dates.

The query is read-only. It consumes the current published source and makes no
bridge call. `fortress.observe` remains the explicit way to acquire a new
announcement batch. The protocol-1.1 runtime remains unadmitted development;
this query changes no bridge protocol, native method or production admission.

## Request

In the protocol-1.1 development runtime, call `fortress.query` with:

```json
{
  "session_id": "<session returned by fortress.open_session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "events",
      "source": "announcements",
      "fields": ["text", "report_type", "repeat_count", "continuation", "announcement"],
      "order": "tick_descending",
      "limit": 4
    }
  }
}
```

The structured envelope belongs in the `query` argument. Omit the legacy
top-level `mode`, `limit` and `continuation` arguments. `mode: "schema"` discovers
the same closed grammar, including the new source selector.

Optional `from_tick` and `through_tick` bounds are inclusive. They default to
zero through the published snapshot's game tick. Use absolute Dwarf Fortress
ticks, derived from each original report's `year` and `year_tick`; the query
does not substitute the tick at which the report was observed. Reversed windows
and future upper bounds are refused.

The only source names are `world_events` and `announcements`. Omitting `source`
means `world_events`, with the existing canonical event query and continuation
identity unchanged. There is no implicit union, native refresh, or history
reconstruction. The same selector is available in shared structured-query
consumers, including the laboratory wrapper, when their exact source contains
the corresponding announcement entities.

## Rows and evidence

Every returned report has `kind: "announcement"`, its original `report_id`,
canonical `entity_id`, generation and revision, and a deterministic 128-bit
`event_id`. The latter retains the complete entity ID and generation; the full
source anchor supplies the fortress and observation epoch. This is a query
view identity, not a new canonical `WorldEvent` or native event identifier.
The view is versioned as `dfmcp.announcement-event-window/1`.

Rows are ordered by `(original report tick, event_id)`. Descending order
reverses both components. Original report IDs need not have chronological
dates. The `announcement` field preserves the native flag, which can be false:
the source contains the retained reports supplied by protocol 1.1, including
reports that the game did not flag as announcements. Unknown native
`report_type` integers survive unchanged rather than being assigned invented
semantic names or severities.

Selected fields use the existing typed `Fact` encoding, preserving presence,
epistemic state, observation tick, source label and batch digest. Each row also
carries the three required identity/time facts in `identity_evidence` and its
`source_batch_digest`. Establishing a report requires consistent known
`report_id`, `year` and `year_tick` facts from the protocol-1.1 projection, the
same batch and observation tick, a valid native report-ID namespace, and a
report date no later than that observation. Incomplete or contradictory
identity/time evidence is counted under
`coverage.unestablished_identity_or_clock`; it is never converted to absence.

`summary` contains the retained report text only when the text fact is known
and shares that source, batch and observation tick. Otherwise it is null and
`summary_presence` is unknown. Missing, omitted, unsupported, stale, redacted or
contradictory selected facts keep their unavailable state; their compatibility
values are not exposed as known. A known fact from a different source, batch or
observation is masked as unknown.

Native report text does not establish a canonical subject ID. Rows therefore
have `subject: null`. A nonempty `subjects` selector cannot match a report by
names or numbers in its text: candidates count toward
`coverage.unestablished_subject_selection`, and selection completeness becomes
false. An empty kind set includes reports; a nonempty kind set must include
`announcement` to include them.

All report text remains `untrusted_source_text`. The derived event view is
`inferred`; individual source facts retain their own evidence class. It grants
no capability and cannot establish a causal game event, a mutation
precondition, task success or production admission.

## Retention, gaps and filtered results

The query reports three distinct scopes:

| Scope | Result fields | Meaning |
| --- | --- | --- |
| Retained canonical projection | `retained_records`, retained tick extrema, unavailable-identity counts | The announcement entities present in this exact snapshot. Extrema do not prove coverage between ticks. |
| Selected query page | `matched`, `returned`, `truncated`, `continuation`, `page_complete` | The established matches and the complete ordered prefix returned by this call. |
| Native retained batch | `native_retained_window`, `retained_suffix_complete` | The exact published native bounds when supplied by the owning protocol-1.1 runtime. |

The live runtime attaches `coverage.native_retained_window` only after checking
the private projection receipt against its already-published capsule. It
includes the announcement-batch and combined-capsule digests, source anchor,
requested and next report IDs, oldest and latest available report IDs, native
returned count, `complete_through_latest`, and
`gap_before_retained_window`. Filtering or paginating the query never rewrites
these batch bounds. `query_refreshes_source` is false.

An entity-only source cannot reconstruct native retention from its rows. Other
shared consumers therefore return `native_retained_window: null` and
`retained_suffix_complete: "unknown"`. They do not infer a complete suffix from
an empty result or consecutive report IDs. `selection_complete_within_projection`
concerns establishing the projected records' identity and selectors; it does
not describe native capture completeness.

Even a native batch complete through its high-water mark can have an older
retention gap. A complete suffix and a complete query page never establish
complete fortress history. `history_complete`, `absence_proven`, and
`eligible_as_mutation_evidence` always remain false. The same limits apply to
empty results.

## Continuations and bounds

Copy the returned continuation into `query.query.continuation`. Keep the source,
time bounds, kind/subject/field sets and ordering unchanged. Page width may
change. The cursor binds the session, complete source anchor and last returned
`(tick, event_id)` using a distinct announcement-query digest domain. It cannot
be resumed against `world_events`, a refreshed batch, a new entity generation
or another session. Its integrity binding grants no read authority.

The existing limits of 32 kinds, 64 subjects, 32 fields and 128 rows per page
apply. The complete canonical source is admitted before hashing or selecting
reports, including unselected entities, edges, events and nested values.
Source work is bounded by the current session limits, 100,000 records, a
16-MiB conservative source estimate, 4,000,000 value/work visits, depth 64 and
the request deadline. Narrowing a page cannot bypass source costs.
The live publisher also admits its already-published capsule's raw record
counts and cached base/batch/combined byte lengths before revalidating that
capsule. This conservatively counts bytes repeated in the combined encoding.
Capsule verification, event selection and final packet rendering share one
wall-time allowance.

Protocol-1.1 query publication fits whole report rows together with the full
Agent Turn, active work, source references and exact native retention metadata.
The laboratory wrapper uses its existing complete-packet page fitting. A later
row that cannot fit leaves the continuation at the last published row; an
oversized first row or empty-result envelope returns `budget_exceeded`.
Current Query authority is checked again before success is published.

## Implementation checks

`announcement_event_query_tests.rs` covers real projected batch facts, original
report dates, source-specific cursors, generation/session/selector drift,
unavailable and contradictory facts, tainted text, subject uncertainty,
complete-source limits, canonical hash refusal and whole-row fitting.
`live_announcement_event_query_tests.rs` exercises the actual protocol-1.1
publisher with combined capsule/projection receipts, complete and partial
retained suffix fixtures, retention gaps, whole Agent Turn pagination,
cross-capsule refusal, current authority and schema discovery. These checks
do not establish live-game compatibility or native admission.
