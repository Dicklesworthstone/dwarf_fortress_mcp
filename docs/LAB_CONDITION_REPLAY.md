# Replay sampled laboratory conditions over retained history

The default laboratory `fortress.query` tool can evaluate a temporal condition
over an exact retained interval. This answers questions that comparing two
endpoints cannot: did a stock threshold become stably satisfied, did a failure
guard trigger first, or did missing observations prevent a positive streak?

`condition_replay` uses the same condition evaluator and watch state machine as
[foreground laboratory watches](LAB_CONDITION_WATCHES.md). It creates no retained
watch, baseline, obligation, timer, observation or game effect. Its result is a
retrospective sampled analysis, not evidence that a watch existed at the time,
that a particular action caused the result, or that truth held between samples.

## Select the original observations

Pass this object as the existing `fortress_query` `mode` JSON string, alongside
the owning `session_id`. Replace the hashes with exact state hashes returned
earlier by this session:

```json
{
  "mode": "condition_replay",
  "from": "<exact earlier retained state hash>",
  "to": "<exact later retained state hash>",
  "definition": {
    "condition": {
      "op": "field",
      "entity_id": "5001",
      "generation": 1,
      "field": "stock.drink",
      "comparison": "ge",
      "value": {"type": "u64", "value": 60}
    },
    "deadline_tick": 1000,
    "poll_interval_ticks": 10,
    "stable_observations": 3
  },
  "detail": "evidence"
}
```

The example's stock entity is the starter laboratory ledger; inspect the actual
source identity and generation before selecting facts in another world. The
deadline must follow the first selected observation's tick. It is a historical
analysis parameter and does not require or grant ControlClock permission, nor
does it consume the current session's forward game-tick allowance.

`from` is required. Omitting `to`, or setting it to null, selects the current
world. Both endpoints must already belong to this session's retained history,
in its current fortress and observation epoch. If another shared session has
advanced to a frontier this session has not recorded, first use `fortress.observe`
to retain that actual observation. Replay never invents a current or intermediate
sample to make a range complete. Unknown, collected, reversed, restored,
future and conflicting same-cursor endpoints refuse explicitly.

The interval contains every consecutively recorded version from `from` through
`to`, inclusive, with at most 32 versions. A plan or checkpoint can retain an old
endpoint after an interior version has been collected. Such a hole returns
`stale_anchor`; surviving endpoints do not authorize stitching an incomplete
history together. Reading a range does not pin or extend retention.

Recording continuity is separate from observation continuity. A retained
version can follow another by several canonical observation sequences, for
example when an ordinary bounded wait advanced internal work before its final
published observation. Those original sequence numbers are preserved. The
ordinary watch rule resets unfinished positive stability at such a gap.
`range.observation_cursor_gaps` reports how many adjacent retained pairs have a
cursor gap; no unrecorded game tick or observation is reconstructed.

## Same predicates and temporal rules

The closed definition accepts the laboratory `field`, `entity_count`, `field_sum`,
`paused`, `tick_at_least`, `all`, `any` and `not` conditions. A `field_sum` recomputes
its selected observed population and source-qualified unsigned sum at each
historical anchor; it never fills missing values from a later observation. An optional
`failure_condition` uses the same grammar. Native-only stack, relationship,
terrain, furniture and shelter predicates remain outside this laboratory
boundary. The caller cannot select a source policy or supply a native archive,
path, endpoint, watch handle, key or label.

Only registered laboratory facts establish field truth. Missing, stale,
redacted, conflicting, future, asserted and ineligible facts remain unknown;
their unavailable values are not published. Exact entity generation changes
invalidate unfinished monitoring even behind a decisive Boolean branch.

The first selected observation acts as registration and the first due sample.
Cadence defaults to one game tick and positive stability to two samples.
Changed observations at the same tick cannot add positive stability. False or
unknown success, an unknown failure guard, and observation gaps reset the
unfinished streak. A known failure guard takes precedence over success. A due
sample at the exact deadline can satisfy the definition; a later observation
cannot establish late success. A deadline between selected observations is
detected at the next actual observation, without inventing a sample at that tick.

The first terminal outcome stays terminal. Later selected versions are still
fully admitted and validated, including their canonical identities and hashes,
but do not reopen or resample that monitor. A corrupt or oversized later source
refuses the whole result even when the first sample could have satisfied the
condition.

## Result and evidence

Every successful response has `mode: "condition_replay"` and
`kind: "historical_watch_replay"`. It includes:

| Field | Meaning |
|---|---|
| `status`, `terminal` | The ordinary sampled monitor outcome. |
| `sample_count`, `stable_observations` | Actual due samples and the final positive streak. |
| `first_terminal_record` | One-based position in this exact selected interval, or null. |
| `records_verified`, `records_evaluated`, `records_after_terminal` | Full source validation versus actual monitor advancement. |
| `basis`, `anchor` | First and final selected source anchors. |
| `last_evaluated_anchor` | Source at which monitoring last evaluated; it can precede the final selected source. |
| `current_anchor` | Current session authority and Agent Turn anchor. |
| `range` | Exact endpoint hashes, every selected anchor, interval binding and cursor-gap count. |
| `definition` | The complete requested condition and normalized temporal defaults. |
| `replay_id`, `evidence_digest` | Identities binding the definition, full ordered source interval and sampled outcome. |
| `source_work` | Combined record and canonical-byte admission for every selected version. |

`detail: "summary"`, the default, omits transition detail and explicitly reports
`status_changes_complete: false`. `detail: "evidence"` adds the status transitions
and the last predicate evaluation. Both modes run the complete selected interval
and produce the same replay and evidence identities. Later history appends and a
different current request do not change those identities when the selected
interval and definition remain unchanged.

A replay ID is an evidence reference, not a retained watch handle. It cannot be
polled, cancelled, released or committed. `historical` is true even when the
interval ends at the current world. The Agent Turn keeps the current complete
anchor, marks the analysis stale, and preserves independent pending plans,
action work, baselines and watches without sampling or modifying them.
`continuous_between_observations`, `original_watch_activity_proven`,
`game_action_completion_proven` and `eligible_as_mutation_evidence` remain false.

## Authority and complete budgets

Current whole-fortress Query authority is checked before retention lookup and
through evaluation and rendering. Historical ticks never renew an expired or
revoked grant. The original current cancellation state remains binding.

All selected complete worlds share one source record and canonical-byte
allowance, including unselected facts and versions after an early terminal
result. The existing structural shape and work bounds precede canonical
serialization. All predicate evaluations share the existing one-million-work
monitor allowance. One cooperative wall-time limit covers ownership acquisition,
source admission, evaluation and final response publication.

The decoded request is limited to 64 KiB. Success and failure share at most 64
condition nodes and depth eight; cadence is 1–1,000,000 ticks and stability is
1–64 observations. The complete response and Agent Turn must fit both bytes and
tokens. Output fitting may compact the Agent Turn, but cannot clip the replay
result or publish a partial temporal conclusion. Use a shorter independently
defined interval, summary detail, or an explicitly negotiated larger response
allowance if necessary. Independent shorter intervals start independent streaks;
their outcomes cannot be stitched into one longer proof.

The decoded request schema is
[`lab_condition_replay.schema.json`](../schemas/lab_condition_replay.schema.json).
This is a deterministic laboratory capability. It changes no native protocol,
admission registry, production runner or native archived replay policy. Exact
executed evidence and broader qualification limits are recorded in
`IMPLEMENTATION_STATUS.md` under bead `df-lab-structured-query-f7q.3`.
