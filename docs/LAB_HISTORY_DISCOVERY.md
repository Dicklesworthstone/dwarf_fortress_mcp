# Discover retained laboratory history without old transcript hashes

The laboratory `fortress_query` tool can list the exact world versions still
retained by the current session. A fresh agent can find an earlier source for
an `at` read, an endpoint comparison, or sampled condition replay without
recovering hashes from an earlier conversation.

This is a metadata query over the session's actual retention store. It neither
observes nor advances the world, samples a watch, retains additional versions,
or exports world contents. Current Query authority remains required. The
catalog describes laboratory history and establishes no native compatibility
or mutation authority.

## Start from the current handoff

The session handoff includes `orientation.retained_history`, a complete
`fortress.query` next step with the owning `session_id`. Its `mode` argument
is this JSON string:

~~~json
{"mode":"history","limit":4}
~~~

The same request can be sent directly to the existing `fortress_query` tool.
The decoded object accepts only:

| Field | Meaning |
|---|---|
| `mode` | Exactly `history`. |
| `limit` | Optional maximum whole rows, 1–32; omitted or null defaults to four. |
| `order` | `newest_first` by default, or `oldest_first`. |
| `continuation` | Optional exact returned cursor; omitted or null starts discovery. |

Ordering uses the retention store's recording ordinal. It does not sort by game
tick, original observation cursor, state hash, or an inferred chronology.
Re-recording an older known world moves its retention ordinal. That movement
does not rewrite the world's original anchor.

Every row includes its `recording_sequence`, complete `anchor`, whether it is
the actual current world, and its compatibility with current-epoch condition
replay. The full anchor contains fortress identity, observation epoch and
sequence, game tick, and exact state hash.

`retained_versions`, `oldest_retained_anchor` and `newest_retained_anchor`
describe the complete retained catalog, independently of the page size.
`current_anchor_retained` can be false: another shared session may have advanced
the world to a frontier this session has not yet recorded. History discovery
reports that honestly and leaves retention unchanged. Use an ordinary
`fortress.observe` if a later read needs that actual frontier retained.

The response's `anchor`, `current_anchor` and Agent Turn anchor identify the
current session world. Each row preserves its own historical anchor. Independent
pending plans, game work, baselines and watches remain discoverable in the Agent
Turn and are not advanced by catalog inspection.

## Use discovered endpoints

Each page supplies a bounded `replay_windows` array. A window's `from` and
`to` hashes identify one inclusive interval that the actual
`VersionRetention::consecutive_range` reader can resolve. It contains no more
than 32 versions, belongs to the current fortress and epoch, advances in
original observation sequence, and does not move backwards in game tick.

A window ends at a row on the current page. For each maximal compatible retained
interval represented on that page, the window contains up to the latest 32
versions ending at that page endpoint. Earlier members can precede the current
page; their exact endpoint identities are still supplied. The catalog never
dumps every segment's details on every page.

For example, copy a returned window's endpoint hashes into:

~~~json
{
  "mode": "condition_replay",
  "from": "<window.from>",
  "to": "<window.to>",
  "definition": {
    "condition": {"op":"tick_at_least","value":100},
    "deadline_tick":1000,
    "poll_interval_ticks":10,
    "stable_observations":2
  },
  "detail":"evidence"
}
~~~

Choose a condition and deadline appropriate to those original observations.
The catalog supplies no condition definition and makes no claim that any
particular temporal condition was true. The later replay independently admits
and validates every complete selected world under its own current authority,
source and output allowances. A metadata window can be discoverable even when
replaying its full world contents exceeds a smaller work budget.

To inspect one version, use its `anchor.state_hash` as `at` in the existing
structured wrapper:

~~~json
{
  "mode":"structured",
  "at":"<row.anchor.state_hash>",
  "request":{
    "schema":"dfmcp.query/1",
    "query":{"kind":"entities","kinds":["unit"],"fields":["alive"],"limit":4}
  }
}
~~~

For endpoint changes, use discovered hashes as `before` and `after` in
`query_changes`. Those consumers keep their own lineage, source-validation,
selection, authority and budget rules. Catalog discovery does not bypass them.

## Recording holes and observation gaps mean different things

Rows report both boundaries explicitly:

- `recording_gap_before` names exact missing retention recording positions
  before the row. Those positions are not currently retained. Collection or
  re-recording can create a hole; the catalog does not invent a specific reason
  when recording metadata alone cannot prove it.
- `observation_gap_before` names missing original observation sequence numbers
  between adjacent retained anchors of the same fortress and epoch. Its count
  is independent of missing retention positions.
- `observation_continuity_from_previous` separates adjacent cursors, cursor
  gaps, epoch changes, fortress changes, nonmonotonic observations, and the lack
  of an earlier retained row.

A pinned old endpoint does not preserve intervening history. Missing recording
positions split replay windows. A collected prefix remains explicit even when
all currently retained rows fit on one page.

Original cursor gaps may occur inside a consecutively retained interval.
They remain eligible for sampled replay, whose watch state machine resets an
unfinished positive streak at a gap. Each window reports
`observation_cursor_gaps` and `stability_resets_at_cursor_gaps: true`.
No observation is reconstructed and no continuous truth between samples is
claimed.

Restore epochs remain visible in retained rows, but old-epoch rows receive
`condition_replay_eligibility: "different_epoch"` and do not enter current
replay windows. Future or conflicting current-cursor anchors are similarly
excluded. Same-cursor forks and decreasing ticks split intervals. A retained
foreign fortress is refused before its metadata is disclosed under the current
fortress's authority.

`world_contents_validated: false` is deliberate. This query reads owned
recording metadata and copies anchors. It does not serialize, inspect, or hash
entities, relations, chunks, events or their fields to establish canonical
world-content integrity. The selected read must do that work later.
`history_complete`, `continuous_between_observations` and
`eligible_as_mutation_evidence` remain false.

## Page one exact catalog generation

The `lh1` continuation is an opaque, bounded 85-byte string. Copy
`next_query` for the next request, or copy `continuation` while preserving
the same `order`. The page width may change.

The cursor binds:

1. the owning session;
2. the complete current authority anchor;
3. every retained recording ordinal and complete anchor;
4. the selected recording order;
5. the exact last returned recording ordinal.

Its checksum is an identity check, not an authorization credential. Query
authority and cancellation are checked again on every call. The caller cannot
substitute a different session, source catalog, ordering or position under an
unchanged cursor.

Appending, collecting or re-recording history, changing the current world, or
restoring a new epoch invalidates earlier continuations. Restart with
`{"mode":"history"}`; any original hashes still retained remain available
through their ordinary exact-source consumers. Catalog reads never pin or
extend retention.

`returned`, `truncated` and `continuation` describe this page's delivery.
A final page means that no further identities remain after that position in
the selected order. It does not prove the caller received previous pages.
Complete catalog membership metadata is separate from complete page delivery
and from complete game history.

A transcript that successfully consumes a history continuation is marked
nonreplayable because a fresh replay session has a different owner identity.
An initial history request without a continuation can remain replayable.
A refused continuation is not recorded as a successfully consumed page.

## Bounds and publication

The decoded request has a 2-KiB ceiling and rejects duplicate or unknown fields.
The complete metadata catalog must contain no more than 4,096 records and fit
the current `max_entities` allowance before its ordered anchor vector is
allocated. The normal session retention policy currently retains its recent
window plus bounded roots; this query does not change that policy.

The complete metadata source also fits `max_bytes` before hashing. Its
canonical encoding uses a domain separator, session identity, current complete
anchor, record count, then ascending recording ordinals with full anchors.
Integers use fixed-width unsigned big endian bytes. Each anchor is 64 bytes
and each ordinal-plus-anchor record is 72 bytes. This is metadata work, not a
charge for scanning unexamined world contents. The response reports both
`retained_metadata_records` and `canonical_metadata_bytes`.

One cooperative wall-time deadline covers owning the session, metadata
acquisition, interval discovery, encoding and final Agent Turn publication.
Current whole-fortress Query authority precedes retention access; revoked,
expired, entity-scoped, region-scoped, exhausted or cancelled authority cannot
be revived by an old source hash or continuation.

The helper can reduce a requested page to a smaller prefix of whole rows to
fit its result allowance. It rebuilds the continuation from the last row
actually returned. It never skips an oversized row or publishes an empty page
that advances its cursor.

The complete page and Agent Turn must then fit the negotiated response bytes
and output-token estimate. Only the lossless full or compact-Agent-Turn tiers
are eligible. Generic record clipping is forbidden for history pages. If the
complete packet cannot fit, the call refuses without records, replay windows
or an advanced continuation. Reduce `limit` or explicitly negotiate a larger
response allowance. A budget too small even for its refusal receives the
existing explicit minimal budget-error envelope.

## Implementation and verification

The closed request schema is
[`lab_history_catalog.schema.json`](../schemas/lab_history_catalog.schema.json).
The machine contract is
[`lab_history_catalog_v1.json`](../architecture/lab_history_catalog_v1.json).

`VersionRetention::recorded_anchors` copies only bounded recording metadata.
`lab_history_catalog.rs` validates the request, current authority, catalog
binding, gaps, replay candidates and exact row-prefix delivery. Server
integration uses the read-only query session fence so neither normal durable
persistence nor implicit retention publication runs after the query.

Focused source regressions cover bounded extraction, ordered paging, changed
catalog and owner rejection, collection holes, old epochs, cursor gaps, same
cursor forks, moved recording ordinals, metadata budgets, output refusal,
cancellation and unrecorded shared frontiers. Public-facade regressions follow
the handoff to history, then use discovered endpoints for actual condition
replay, `at` reads and endpoint comparison while preserving unrelated work.
They also cover public hole/epoch rejection, lossless budget refusal and
session-bound replay behavior.

Exact executed evidence and remaining workspace limitations belong in
`IMPLEMENTATION_STATUS.md`. These regressions do not establish native DFHack,
production admission or full workspace qualification. The owning bead is
`df-lab-structured-query-f7q.4`.
