# Historical job progress

`historical_job_progress` compares job indicators in two exact retained
spatial/1.8 captures. It is available through the existing `fortress.query`
surface in both journal-backed live development sessions and archive-only
sessions. It performs no native capture, journal write, watch update or game
mutation. The runtime remains unadmitted development; this query adds no
compatibility evidence or mutation authority.

## Request

First query `{"kind":"history"}` to obtain record numbers and digests. Supply
the exact references from that response:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_job_progress",
    "from": {"record": 1, "record_digest": "<exact retained digest>"},
    "to": {"record": 4, "record_digest": "<exact retained digest>"},
    "include_unchanged": true,
    "limit": 4
  }
}
```

Optional `job: {"entity_id":"3","generation":0}` focuses on the original
job in the `from` record. Use its actual canonical handle, including generation.
A missing or wrong basis generation is rejected. If that job disappears or its
native ID is reused, the report retains the original identity and discloses the
disappearance or replacement. It does not substitute the replacement as evidence
about the original job.

Both endpoints must belong to the same fortress, observation epoch, software
versions and native source generation, in forward record and game-clock order.
The current session's authority is checked before replay and throughout the
request. Archived observations cannot revive an expired grant.

## Reading the result

Each row contains exact before/after handles and one identity status:
`same_generation`, `newly_observed`, `no_longer_observed`, or
`generation_replaced`. A missing endpoint is evidence of roster absence only
when the original native roster is complete. Unestablished coverage cannot prove
absence.

For the same job generation, comparisons include suspension, repeat mode, job
type and reaction, position, raw completion timer, worker assignment, holder
identity, holder construction stage, required-filter count, and attached-item
membership. Item membership includes item generation, role and filter index;
equal attachment counts alone cannot establish equal membership. Examples are
bounded to four attachments, with a digest covering the complete qualified set.

Worker assignments require agreement among native worker identity, assignment
flags, the strict-citizen entity and its observed `performs` edge. An assigned
worker outside the strict-citizen roster is disclosed without a canonical worker
identity. The same outside-roster native ID in both records cannot establish
that the same worker generation remained assigned. Holder stages are compared
only for the same qualified building generation and unchanged maximum stage.
Contradictory, stale, absent, unsupported or wrongly sourced facts produce
`unestablished` indicators.

The summary covers the entire job union before the optional job focus or
`include_unchanged` filter. It counts changed and unchanged indicators, missing
evidence, construction-stage advances/regressions, and jobs observed suspended or
unassigned at both endpoints. These last two counts describe the endpoints;
they do not prove continuous suspension or nonassignment between them.

**Disappearance does not prove completion. Unchanged endpoints do not prove a
stall.** A holder-stage advance concerns the observed building, not completed
work by this particular job. Raw timers are not an ETA or work quantity, and
`elapsed_game_ticks` is endpoint distance, not job age. Native blocker reasons,
intermediate events, reachability and current game freshness remain unknown.

Rows contain ready-to-run `inspect_before` and `inspect_after` query envelopes.
These pin inspection to the corresponding exact record/digest and canonical job
generation. They remain useful after the live session advances.

## Bounds and continuity

`limit` is 1–128 (default 4); `max_work` is 1–4,000,000. The complete job union is
assessed before pagination. Pages contain whole rows within the response byte
and token allowances, including both record witnesses and the Agent Turn
Packet. If one complete row cannot fit, the request returns a budget error
without a non-progressing continuation.

The continuation binds the session, current authority anchor, archive identity
and head, both endpoint anchors and source digests, policy, normalized selection
and work allowance. Page size and output allowance may change; selection,
records, work allowance or session changes require a fresh query. Reopening the
same archive allows the comparison to be recomputed but does not revive an old
session's continuation.

Agent Turn continuity stays partial, identifies the basis endpoint and exposes
the report under `changes`. Live-session active work remains based on the
current session. Reading history neither advances nor creates a watch.

Contract: `schemas/mcp_historical_job_progress_v1.json`. Implementation:
`crates/dfmcp-mcp/src/job_progress.rs` and `spatial_job_progress.rs`. Native
decoder/model tests exercise provenance and identity; archive-handler tests
exercise exact replay, executable inspection, output bounds, reopening,
authority expiry and fenced-source reads.
