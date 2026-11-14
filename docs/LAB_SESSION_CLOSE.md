# Close a quiet laboratory session

Use the existing cancellation tool with both explicit selectors:

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<owning-session-id>",
    "mode": "close_session",
    "scope": "session"
  }
}
```

This releases a session after its owned work is proven quiet. Ordinary
`scope: "session"` with the existing cancellation modes continues to drain
retained work and leaves the session open. Closing never selects another
session, dispatches a pending plan, advances the clock, compensates an effect,
or automatically cancels a Task.

## Before closing

Export any replay bundle, evidence, checkpoint information, or process-local
history you need while the session is still open. Close deliberately discards
the pending review, foreground baselines and watch handles, private
presentation/replay/history state, and the session's grants. It does not claim
that closing was recorded in the discarded replay log. An earlier exported
replay remains a separate private laboratory input.

If work remains, explicitly drain the intended original plan or session first,
then inspect the result. Terminal Failed or Verified goal receipts do not by
themselves establish physical quiescence. Stop or finish an active modern Task
monitor separately; even a monitor that observes only an original goal owns its
session until its supervisor has returned. Task cancellation scope remains
independent of session close.

## Admission and refusal

Close requires the explicit session identifier and its current Observe
authority. It inspects complete original commit custody and current physical
work without polling or dispatching deferred steps. A prepared successor is
still nonterminal, including when its prerequisites have already verified.

The request refuses before releasing session or query resources when:

- Current authority, durable ownership, or source integrity is unavailable.
- A retained owned action is nonterminal, physically active, unknown, or missing.
- Recovered work lacks a complete final disposition or current quiet physical
  identity, or untracked active/unknown work remains in the observed world.
- A queued, working, input-required, or still-running Task supervisor belongs
  to the session. A terminal wire status alone does not end supervisor custody.
- The complete census, original receipt sources, or final Agent Turn cannot fit
  the negotiated source, entity, wall-time, byte, or token allowance.
- A durable head cannot be published or an owned/unattributed durable commit
  remains unfinished after publication. In particular, durable admission before
  an adapter commit refusal cannot be discarded as a harmless pending review.
- The relevant owner, world, or custody registry is poisoned or inconsistent.

The source allowance includes the complete canonical world (entities, edges,
terrain chunks and events), retained original receipts and the complete proof/
physical census. Structural checks precede recursive encoding and hashing.
World source records and census rows share the negotiated record allowance.
The source allowance is the session's existing allowance. A large accumulated
commit history can exceed it; closing does not bypass that limit or silently
truncate the census. A superseded durable session remains fenced and cannot
use its stale world to claim fresh quiescence.

A refusal releases nothing in that call. Its `closed: null` does not assert
whether an unavailable handle was previously closed, and its partial Agent
Turn does not claim that active work is absent. A successful durable
publication may precede a later refusal; the session and its recovery custody
remain available.

## Shared and durable worlds

Closing one member removes its membership, its already-quiet lease bookkeeping,
and its unpause consent. It does not unpause the fortress or stop another
member's work. Remaining members keep the canonical world, goals and action
history. The departing session's certificate covers only its owned action set.

The last member must establish quiet work across the complete adapter index.
After success the shared world is removed from the process registry, reclaiming
its bounded world slot. A private session similarly releases its private
in-memory world. Process-local world state that has no remaining owner is
discarded by this explicit operation.

For a durable world, close first publishes its current head and final action
frontier. The durable world, checkpoint objects and retained goal evidence
remain in the operator's store. Closing a private durable session removes its
current owner entry only after that publication. Reopening uses a new session
and the ordinary recovered observation epoch; old grants, action handles,
baselines, watch handles and dispatch authority do not return.

Closing does not withdraw a goal, reinterpret historical proof, or certify
goal success. Volatile goal records are discarded only when no process owner
or durable store retains them.

## Final receipt and lifetime fencing

Success returns a complete Agent Turn that already fits the session's output
allowance. It has no resource links requiring the closed session. It reports:

- Session and orientation capacity slots released; pending-review and derived
  query-handle counts.
- Shared membership and remaining-member count, world release, durable
  publication, and lease release.
- Terminal Task history retained by the process. Retained Task IDs grant no
  read authority after their session closes.
- A quiescence certificate with the selected scope, action/recovered-step
  counts, current anchor, and a complete census digest.

The census binds each original action identity, original plan and step,
idempotency key, terminal proof state/receipt digest/proof anchor/message,
and freshly inspected physical-work state. Recovered rows bind their retained
proof and original physical identity separately from the new physical
observation. The outer certificate binds that census digest to the session,
anchor and scope. It is evidence of the inspected close boundary, not a claim
about another session's work or future game state.

A write admission guard drains preceding foreground calls before close. Task
creation and teardown share a custody fence, so a descriptor validated before
close cannot create a new orphaned Task afterward. A revoked closed flag and
cleared grants also reject callers that resolved an Arc before removal. A
delayed opening presentation cannot recreate an orientation slot after close.

The most recent 32 successful close receipts are retained in close order,
process-locally. Repeating the exact close for one of those identifiers returns
the same final receipt. Older receipts may be evicted; an absent receipt never
reopens a session. Receipt retention is not durable and session identifiers are
not reused within a process.

## Regression source

`lab_session_close_tests.rs` covers resource release, stale handles, current
authority and output refusal, terminal proof versus active/unknown work,
deferred successors, shared-member preservation, final world release, Task
creation/supervisor races, durable publication failure, unresolved durable
admission and reopening. The facade regression covers delayed opening
presentation. Execution and qualification status are recorded separately in
`IMPLEMENTATION_STATUS.md`; source coverage is not a claim of a passing run.
