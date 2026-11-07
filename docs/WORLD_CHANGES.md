# Laboratory world changes

The laboratory Agent Turn reports differences between the exact canonical world
version the session last saw and its current retained version. The same comparison
is available explicitly through the existing query tool:

```json
{"mode":"changes","since":"<retained source state hash>"}
```

The comparison attaches `basis.from` and `basis.to` to every item, each with the
fortress, observation epoch, sequence, game tick and state hash. Explicit changes
queries and briefing turns retain these endpoints. The existing pulse profile
can elide repeated per-item basis under its compact continuity contract. This is
an endpoint comparison, not a replay of every intervening action or event. The
result grants no mutation authority. The source is the deterministic laboratory;
it establishes no native Dwarf Fortress behavior or compatibility admission.

Automatic Agent Turn disclosure continues to require the session's observation
grant. An explicit changes query requires current Query authority, just like the
other historical queries. Retained state never revives an expired grant.

## Changes an agent can act on

| Kind | Meaning |
|---|---|
| `game_time_passed`, `fortress_paused`, `fortress_unpaused` | Clock or pause differences within one fortress and observation epoch. |
| `entity_created`, `entity_removed` | Entity record membership changed between the endpoints. The subject includes identity, generation, revision, kind and label. |
| `entity_changed` | A field or entity metadata changed. `before` and `after` preserve label, kind, revision and generation, including changes with no changed field value. |
| `relation_created`, `relation_removed`, `relation_changed` | A relation record, its fields, or its observed endpoint generations changed. The report includes the exact edge ID, kind, revision and endpoint IDs on both sides. |
| `terrain_changed` | Per-level terrain summary with observed tile transitions and a bounding box when tile detail is available. |
| `event_observed`, `retained_event_changed`, `event_no_longer_retained` | A canonical event entered, changed within, or left the retained event projection. These names describe records, not inferred event causality. |

An unchanged edge can still receive `relation_changed` when an endpoint entity's
generation changes. Its `from_generation` and `to_generation` are resolved only
from the corresponding endpoint snapshot. A missing entity produces null, never
zero or a generation borrowed from the other snapshot. These values identify the
observed endpoint records; `EdgeRecord` does not itself retain a historical
generation binding.

Changed entity and relation fields use the existing presence-preserving
projection. Unknown, absent, unsupported, omitted, redacted, stale and internally
inconsistent known facts withhold their retained compatibility values. A field
not recorded on one side has `recorded:false` and unknown presence; explicit
absence and a known null remain distinct. A provenance-only fact update is also
reported with `provenance_changed:true`, even when its displayed value stays the
same. Exact source records remain identified by the two anchors.

Entity generation changes explicitly invalidate handles naming that entity.
Relation differences flag witnesses naming the relation or its adjacency for
revalidation. These invalidation hints do not replace the ordinary witness and
authority checks at commit.

## Retained events are bounded evidence

Event arrivals can include older events newly observed at the target. An event
leaving the target can reflect retention, a revised projection or a source
correction. It never means that the event was undone. Each event change therefore
reports `history_complete:false`, `occurrence_since_basis_proven:false` and
`eligible_as_mutation_evidence:false`.

Event kind, tick, subject ID and summary are preserved on both sides of an
update. The canonical event type has neither a subject generation nor per-field
provenance. The report leaves `subject_generation` null and states that source
authenticity is not established. Event summaries and fields remain untrusted
source text; they are data, never instructions or authority.

Event fields retain structured JSON values and explicit presence. Binary values,
including nested values, use the shared byte-length rendering and are marked
omitted. The comparison proves differences within these two retained projections;
even an empty result does not prove complete fortress history or the absence of
unobserved intermediate events.

## Deterministic bounds and omissions

Each comparison has independent allowances for 24 entity changes, 24 relation
changes, 24 retained-event changes and 24 terrain levels. Entity churn cannot
consume the relation or event allowance. Within each domain, records are ordered
by canonical numeric identity; terrain is ordered by level, and field names are
lexically ordered. Clock and pause changes precede entities, relations, terrain
and events.

Each record includes at most eight changed fields. `fields_changed` is the exact
number of changed fields, and `fields_omitted` names the remainder. A domain that
exceeds its allowance ends with `<domain>_changes_omitted`, whose `subject.omitted`
is the exact number of additional changed records or levels. Every omission
marker retains the same two source anchors.

These summaries are not a complete paginated diff. To inspect omitted detail,
query entities, relations, terrain or events at both retained hashes, using the
ordinary historical `at` selector. The outer response's existing output budget
can further reduce the presentation and reports its own omissions. A missing or
collected basis produces the established `history_not_retained` Agent Turn item
or `stale_anchor` query refusal; the server never substitutes a newer basis.

## Restore and lineage discontinuity

A different fortress or observation epoch produces one `world_observation_reset`
item with the exact endpoints and reason. The comparison emits no ordinary
entity removals, event arrivals or apparent backward game time across that
boundary. The restore operation still carries its separate protocol reset and
invalidates pre-restore plans, action handles and continuations.

If the target's sequence or tick precedes the basis within the same epoch, the
comparison returns `world_history_gap` with unknown epistemic state. It does not
construct a forward delta from reversed endpoints. Callers should establish a
new baseline or select a correctly ordered retained basis.

Implementation: `crates/dfmcp-mcp/src/world_changes.rs`. Pure regressions cover
metadata-only and relation-only changes, endpoint generation reuse, unavailable
facts, event correction and retention loss, independent bounds, canonical kind
aliases and reset semantics. Public façade regressions in
`lab_actions_mcp_tests.rs` exercise Agent Turn, historical query and actual
checkpoint restore paths. Executed evidence is recorded separately in the
current implementation status; source tests alone do not imply full MCP or
native qualification.
