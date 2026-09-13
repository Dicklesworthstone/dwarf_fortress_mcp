# Foreground query baselines and endpoint changes

## Purpose and scope

An agent can capture a small, complete selection of observed entities, refresh
the live observation later, and request only the selected changes. This avoids
reconstructing and comparing a full roster in the agent transcript every turn.

These operations are integrated into the protocol-1.1 development server's
existing `fortress.query` tool. They add no top-level MCP tool, bridge method,
DFHack command, mutation capability, or production admission. They operate on the
session's published canonical projection. Query execution never refreshes the
bridge, advances game time, or starts a watcher.

The implementation is source present. The Rust tests and live-game paths have
not been executed in this editing environment. No qualification or performance
measurement is claimed.

## Lifecycle

Use an open protocol-1.1 development session with Query capability. Pass each
structured envelope as the `query` argument of `fortress.query`, alongside the
session ID. Do not mix the structured envelope with top-level `mode`, `limit`,
or `continuation`. Schema discovery remains available with `mode = "schema"`.

A capture envelope can be:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "capture",
    "key": "citizen-welfare-start",
    "max_game_ticks": 1000,
    "select": {
      "kind": "entities",
      "kinds": ["unit"],
      "fields": ["sane", "alive", "profession"]
    }
  }
}
```

The example lifetime must fit the session's negotiated game-tick budget. Add a
supported typed `where` filter to narrow the result. Requested facts unavailable
in the published projection remain explicitly unknown; a selector does not
create missing live data.

The result's `captured.baseline` is an opaque `qb1` handle. Store that handle, or
recover it later using the `baselines` query. Capture performs all required
bounded pages against one exact snapshot before storing any result.

Refresh with the existing `fortress.observe` or `fortress.wait` operation when a
new observation is needed. Then submit a `changes` query with its `baseline`
field set to the returned handle. Its optional `limit` bounds complete changes
per page. Continue by passing the returned `continuation` with the same baseline.
Do not refresh the session between those pages: a changed target anchor rejects
an old continuation instead of mixing observation generations.

| Query kind | Required fields | Result |
|---|---|---|
| `capture` | `key`, `select`, `max_game_ticks` | Complete immutable baseline metadata |
| `changes` | `baseline` | Bounded before/after differences from that baseline |
| `baselines` | None | This session's retained, expired, or epoch-invalidated baselines |
| `release_baseline` | `baseline` | Explicit process-local storage release |

For a rolling monitoring loop, consume the complete changes result, capture a
new baseline under a fresh key at the target anchor, and then release the old
baseline. Supplying the prior response's complete target as `expected_anchor`
on the new capture prevents an intervening refresh from silently changing that
handoff point. Reading a page never acknowledges, advances, or consumes a baseline.

## Change semantics

Rows are compared by canonical numeric entity ID plus generation. Output is
ordered by that pair, not lexicographic ID strings or incidental hash order.

- `entered_result`: this entity generation is selected at the target but was not
  selected at the baseline.
- `left_result`: it was selected at the baseline but is no longer selected in the
  target projection. This is not a death, deletion, destruction, or absence proof.
- `changed_in_result`: both endpoints select the same generation, but its label,
  selected values, presence, epistemic class, or source-kind representation differs.

Each change includes the before/after row when available, preserving fact source
and provenance. Recycled entity IDs generate a leave for the old generation and
an enter for the new generation, never an ordinary update to the old entity.

Entity revision and selected facts' observation tick/source digest do not alone
produce a semantic-change row. These selected-view bookkeeping differences are
counted in `provenance_only_refreshes`. That count does not establish that
unselected fields were unchanged. Presence changes, including known-to-unknown,
remain material changes; unknown never becomes a false value or absence claim.

Both exact anchors and result digests accompany the comparison. A current
snapshot sharing a cursor with conflicting state, a regressed tick or sequence,
a changed fortress, an epoch reset, or an expired baseline refuses comparison.

This is **endpoint comparison only**. An entity can change and change back, or
enter and leave the selected set, between the two retained endpoints without
appearing in the result. Intermediate observations are not retained by this
feature. An empty result therefore means no selected endpoint difference, not
that nothing happened in the fortress.

## Agent Turn and publication

The complete Agent Turn is budgeted before query result construction. It keeps
existing live-source warnings, admission state, attention, and coverage. Changes
responses name the actual baseline in `agent_turn.continuity.basis`, retain the
current target anchor, and provide a compact summary in `agent_turn.changes`
without duplicating all before/after rows.

An advanced endpoint comparison has partial temporal coverage even when every
output row fits on one page. A same-anchor comparison is a heartbeat. A page- or
depth-limited query remains explicitly partial. None of these labels upgrades
incomplete retained-announcement coverage into complete fortress history.

Capture and release prepare their full encoded response before mutating retained
state. Failed rendering or insufficient response space leaves the store unchanged.
A transport loss after a successful capture can be recovered through `baselines`.
The capture key returns the same baseline only for the same session, exact anchor,
selection, and lifetime; conflicting reuse returns a conflict rather than replacing
previous state. Release is explicit and repeatable.

## Bounds and recovery

| Bound | Ceiling |
|---|---:|
| Captured rows | 256 |
| Retained serialized row bytes per baseline | 256 KiB |
| Baselines per session | 8 |
| Baselines per process | 128 |
| Underlying query calls per acquisition | 64 |
| Source-row visits across one acquisition | 2,000,000 |
| Change rows per page | 256, further narrowed by the session budget |
| Change continuation bytes | 128 |

Existing query shape, predicate, scan, field, and whole-response bounds still
apply. The baseline deadline is exclusive and may not exceed the session's
game-tick budget. A first change too large for the result budget returns a budget
error rather than an empty page with a non-progressing continuation.

Expired and epoch-invalidated records are retained for explicit discovery and
release; they do not silently consume fresh game observations or renew themselves.
A poisoned live source is still fenced by the parent query handler, including
history management queries. Retained state remains bounded until it can be
released or the process ends.

State is process-local, not durable. Restart destroys retained rows and changes
the handle incarnation. Handles and continuation digests do not authenticate the
caller: every request independently checks session Query authority, scope,
cancellation, and the exact current anchor. Other sessions cannot list, compare,
or release this session's baselines.

## Source and tests

`query_history.rs` implements the bounded store and comparison. The
`semantic_query::execute_with_publisher` boundary connects it to the existing
live query handler and full `QueryResponseProjection` renderer.

Ten registered Rust scenarios in `query_history_tests.rs` exercise multi-page
capture, idempotent keys, generation reuse, filter departures, unknown presence,
bookkeeping suppression, repeated pages, changed-target cursor refusal,
authority/session/epoch/deadline rejection, explicit retention release, rejected
publication, and full Agent Turn pagination at an 8192-byte ceiling. They invoke
the actual structured-query dispatcher rather than a substitute query engine.

The tests are checked-in source, not passing execution evidence. Rust compilation,
formatting, Clippy, the repository's qualification scripts, actual stdio sessions,
and a disposable-fort live campaign remain required validation steps.
