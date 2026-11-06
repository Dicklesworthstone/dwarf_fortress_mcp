# Laboratory publication event history

The explicit `starter_fortress_history` scenario runs the existing provisions
world with `dfmcp.lab-event-history/1`. Normal plan, commit, wait and cancellation
calls now populate canonical `WorldEvent` records, so an agent can inspect what
happened to an order, worker, construction or inventory without remembering a
conversation transcript. Other scenarios and previously stored snapshots keep
their established behavior and canonical bytes.

## Use through the existing tools

Open `fortress_open_session` with `scenario: "starter_fortress_history"` and the
ordinary capabilities needed for the intended work. For example, an unpaused
session with Observe, Query, Plan, ControlClock and ConfigureProduction can
prepare and commit a `create_work_order` for `BREW_DRINK`, then advance it with
`fortress_wait`. Query that resulting source using `fortress_query`:

```json
{
  "mode": "structured",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "events",
      "kinds": ["job_changed"],
      "fields": ["change", "changes", "generation_before", "generation_after"],
      "order": "tick_descending",
      "limit": 2
    }
  }
}
```

Add `subjects: ["<the order's canonical entity ID>"]` to follow the exact
historical ID. A row retains its subject's before/after generations; an ID alone
does not identify a replacement entity. Omitting `fields` returns compact event
identities and summaries. Select `changes` only when the nested field details
are needed. An oversized complete row is refused, so a caller can select fewer
fields rather than receive an incomplete event.

The ordinary event query also accepts inclusive `from_tick`/`through_tick`
bounds. Its continuation binds the full source anchor, session and selection.
After another mutation or wait, continue against the original source using the
outer `at: "<source state_hash>"` while that exact laboratory version is retained.
Using the cursor against the changed current world is refused. Current authority
is still checked before historical access.

## What is recorded

The producer runs on the unpublished shadow at the real `MemoryAdapter`
publication boundary. A successful action, bounded tick advance or physical-work
cancellation publishes its world changes and history in one final canonical
snapshot. A later failure discards both. Idempotent commit replay, observations,
preparation, checkpoint creation and repeated proof polling append no events.
A deferred action appends records when it actually dispatches.

Material entity changes produce `job_changed` for jobs/work orders,
`unit_changed` for units/creatures, `construction_changed` for buildings and
excavation designations, and `laboratory_entity_changed` for other records.
Changes are visited in canonical entity-ID order. Rows identify creation,
update or removal, generation/revision before and after, and a bounded list of
changed field observations. Observation-time-only refreshes do not create
entity events. Changes in values, presence, source identity, generation, kind
or label can create an event; unavailable diagnostic reason text is not itself
a material change.

Each publication also produces an `adapter_notice`. It names the transition,
pause state, changed relation/chunk counts and any omitted entity/field/value
details. Action and cancellation notices retain the exact action digest and
effect-key digest. A tick notice records the requested elapsed ticks. These
are observations of the publication's endpoints: a batched wait may cross
internal simulation transitions that were never foreground observations.
The producer does not reconstruct those intermediate states or award extra
obligation samples.

Every event names its producer model, fortress, publication epoch/sequence,
predecessor anchor and branch ordinal. `publication_content_digest` seals the
complete effect-applied snapshot **before** the new event batch and retention
counters are appended. It is a distinct named intermediate digest, not the final
snapshot hash. This gives equal-count transitions on different hypothetical
forks distinct event identities without a self-referential hash. The final
canonical source anchor seals the complete published world and all its events.
Event IDs are domain-separated SHA-256-derived 128-bit identities covering the
event content. A conflicting retained ID is refused.

## Presence, source claims and limits

Nested before/after field observations preserve known null and explicit absent,
unknown, unsupported, omitted, redacted and stale presence. Missing fields say
unknown, not absent. Inconsistent known markers and future observations expose
no value. Unavailable compatibility values never enter history. Included facts
retain their observed tick, source kind/label and source digest; an oversized
source label is explicitly omitted. These are captured source claims, not a new
source-admission decision. Event text is untrusted data.

The model retains at most **128 events**, removes the lowest ordinals first,
and never rewrites a retained event. Each publication emits at most **32 entity
events** plus its notice, each entity includes at most **8 changed fields**, and
each captured value is preflighted against a conservative **256-byte**, 32-node,
four-level bound. Oversized values keep their known presence with
`value_included: false`. Longer field names and additional fields are counted as
omitted. Entity comparison is capped at 100,000 total source/target records and
200,000 source/target entity fields; exhaustion refuses the whole publication.
Retained event shape, framing and content identity are checked before reuse.

The structured result's `coverage.producer` names the model and these bounds,
published and evicted event totals, omitted entity/field/value counts, and oldest
and newest retained ordinals. Counters describe the current branch, including
events later evicted from its current snapshot. Field/value omission counters
cover captured entity events; entirely omitted entities have their separate
counter. Changed relations and terrain have record counts, without detailed
relation/tile event reconstruction.

`history_complete`, `absence_proven` and `eligible_as_mutation_evidence` remain
false. Even zero omissions cannot establish complete fortress history or native
event coverage. The canonical `WorldEvent` type does not carry independently
qualified per-field provenance, so query values remain assumed. Validating an
imported model marker and its internal hashes does not authenticate its origin.

## Checkpoint, retention and durable restart

The model marker and counters live on the existing fortress-settings entity.
Canonical snapshot objects already contain the events. The durable store writes
and verifies those exact bytes through its existing object/journal transaction;
this feature needs no sidecar or alternate durability path. Initialization
requires a sealed world with no pre-existing events and refuses conflicting
settings, so it cannot silently reinterpret an older event source.

A checkpoint retains its exact event suffix and counters. Restore starts from
that checkpoint's history, allocates the normal new epoch and appends a restore
boundary naming both the abandoned predecessor anchor and the checkpoint's
`retained_history_anchor`. Events from the abandoned future are not copied into
the restored world. Historical snapshots remain immutable while pinned by the
existing store or process retention rules. Event eviction requires a full
snapshot transition: the existing append-only event delta format still refuses
event removal, and the laboratory adapter does not advertise delta history.

Recovery retains the durable head's suffix and counters, enters a new epoch and
appends a recovery boundary. Physical work can then continue through ordinary
wait calls and append new events. Old action handles are not resurrected, and
event records do not authorize redispatch. A malformed history or exhausted
counter/revision prevents publication; failed restore leaves the current world
and its active work untouched. Injected external snapshots keep their exact
source bytes and do not manufacture a continuity event.

## Implementation and validation

`dfmcp-lab/src/event_history.rs` owns the pure bounded producer.
`MemoryAdapter` invokes it from actual action, tick, stop, restore and recovery
paths. `dfmcp-mcp/src/event_query.rs` consumes its validated coverage metadata
after checking the exact authorized canonical query source. No native bridge
generation or native announcement projection changes.

`event_history_tests.rs` exercises real prepare/commit/poll/observe/replay,
brewing completion, cancellation, exact canonical decode, retention, restore,
durable store reopen, field-presence protection, bounded omissions, fork IDs and
atomic refusal. Shared executor tests cover producer-backed source-bound
pagination. `lab_structured_query_tests.rs` additionally drives the history
scenario through the normal public facade and resumes an exact retained page.
These are focused component/runtime tests; they do not establish native capture,
admission or a full latest-nightly repository qualification.
