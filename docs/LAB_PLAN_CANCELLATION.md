# Cancel one original laboratory plan

The existing `fortress.cancel` tool accepts
`scope: "plan:<64-lowercase-original-plan-digest>"`. Use the digest returned
by the original commit to stop that plan while other plans in the same session
continue. Selection does not depend on the latest committed plan, the order of
open work, the pending review, or a Task monitor.

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<owning-session-id>",
    "scope": "plan:<original-commit-digest>",
    "mode": "stop_future_steps"
  }
}
```

The placeholder digest must be replaced with exactly 64 lowercase hexadecimal
digits. `last_action` and `plan` retain their live-handle meanings.
`oldest_open_plan` selects one oldest unfinished original across verified
durable recovery and current live work. `session` selects all eligible recovered
originals and all complete originals owned by the requesting live session.
Live-only selections support all three existing cancellation modes.

An explicit set of one to eight live originals can be drained together with
`scope="plans:<digest>[,<digest>...]"`. The complete set is admitted before any
pause or stop, and unselected originals and descendants remain outside the drain.
All existing modes are supported. Recovered originals require a separate exact
`plan:<digest>` or session request; their presence refuses the whole selected set
before effects. See [selected original sets](LAB_PLAN_SET_CANCELLATION.md) for
canonical identities, shared budgets, partial-failure semantics and examples.
An explicit `emergency_pause_and_drain` also pauses the shared fortress clock
under current clock authority; ordinary `stop_future_steps` needs no clock
advance or clock grant.

The same exact selector also accepts a verified original recovered from a
durable fortress. Fresh current authority can stop its physical work without
restoring old action handles. Recovered originals support `stop_future_steps`
and `emergency_pause_and_drain`; compensation remains unavailable without its
original live handles. See [recovered original cancellation](LAB_RECOVERED_PLAN_CANCELLATION.md)
for complete custody, publication and budget requirements.

## Emergency pause after recovery

With `scope: "session"`, an explicit `emergency_pause_and_drain` request pauses
the shared laboratory clock even when a durable restart left no live action
handles. Plan-scoped drains likewise pause before draining, including an
already-quiet selected original. An `oldest_open_plan` request needs a retained
live or verified recovered original, and `last_action` still needs its original
live handle.

Current Observe and ControlClock authority and the aggregate action budget are
checked before the pause. Emergency drains also require at least 1,500 output
tokens and 6,000 bytes so the exact outcome can be returned; an insufficient
allowance refuses before the effect. Ordinary stop and compensation modes do
not acquire this clock effect.

The pause creates no replacement commit or action handle and advances no game
time. Its `emergency_pause` receipt retains the exact before/after anchors,
evidence digest, whether the pause changed state, and durable publication
status. A successful emergency pause clears prior shared unpause consent even
when later cleanup refuses. Repeating it against an already paused world is
idempotent, while still requiring current clock authority.

For a durable fortress, the paused frontier is published before subsequent
physical stops. A failed publication retains the actual paused world, fences
later effects and reports the uncertainty; it does not continue the drain or
invent durable success. A later refusal to stop work likewise retains the pause
and its receipt.

Session scope admits the complete mixed recovered/live selection before its
single pause, then stops eligible physical work under fresh scoped authority.
It does not restore old dispatch handles or adopt untracked work. A result with
unresolved carried or untracked work has `drain_progress.quiescent: false` and a
null `finalize_certificate`. Empty session scope can still pause the world
without certifying unknown work; malformed selected custody refuses before
that effect. Use an exact recovered `plan:<digest>` request when only one
original should stop.

## Complete session selection

Session drainage verifies original commit custody even when volatile open-action
lists have lost their entries. Retained request aliases are deduplicated by
their exact original. Recovered originals pass the same complete archive,
lineage, fresh physical-identity and current-authority checks as exact recovered
cancellation. Every selected original is admitted before the first stop.

One source/census allowance, action allowance and deadline cover the whole
request. Stops proceed from newer originals to older originals, with reverse
step order inside each original. Each physical stop publishes its changed world
and disposition before another begins. A failed append leaves the actual
stopped frontier visible, fences further effects and withholds final proof;
retry cannot treat an unpublished frontier as complete.

The response retains `selection_scope: "session_originals"`,
`selected_plans_digest`, aggregate original/live/recovered counts, measured
progress and publication status. A final certificate binds that complete
selection and its fresh final census. Known work owned by other live members is
preserved; unknown or untracked work prevents complete session certification.
The session, pending review and latest-plan orientation remain available.
See [recovered session drainage](LAB_RECOVERED_PLAN_CANCELLATION.md)
for the request and durability limits.

## Complete original selection

For a live original, the current session must retain its original commit response and the adapter
must retain the matching complete original commit. Before the first effect,
selection checks:

- The owning session, canonical requested/actual digest and original plan ID.
- The complete nonempty action list within the current byte, entity and action
  budgets and the fixed 64-action ceiling.
- Every action in original step order, without duplicates or omissions, against
  the adapter's original commit, retained step and current action receipt.
- Current Observe authority and every selected nonquiescent action's original
  scoped capability/risk, plus the aggregate stop/compensation/pause budget.
  Any selected compensation must independently pass its current capability,
  risk and scope checks before the drain starts. Unsupported compensation
  effects refuse at admission. Emergency pause likewise checks current clock
  authority before any selected stop.

Each adapter effect still rechecks current authority at its own boundary.
This preflight prevents a known missing capability on an earlier prerequisite
from allowing a later selected action to stop first.

Automatic compensation for pause, labor and burrow setters is derived from
qualified prior state. A mixed assignment restores only members whose settings
actually changed; a no-op or unknown prior value does not acquire an automatic
inverse. The sealed plan retains source guards, and dispatch captures the exact
restoration basis. Cancellation checks that basis and the current effect state
before any selected drain, and again before applying compensation. A replaced
entity, intervening setting change or unavailable evidence refuses restoration
instead of overwriting later work. Explicit registered civilian protection
remedies retain their distinct protective policy. Terminal receipts still do
not reopen for compensation.

Recovery can reproduce the historical target-negation compiler only when the
entire reconstructed plan digest matches the archived seal. This preserves old
evidence; it does not create a live action handle or authorize an unsafe inverse.
Fresh plans, rebases and continuations use source-qualified compensation.

A witness-rebased commit can retain both the originally requested digest and
the actual newly sealed digest. Either retained alias selects the same complete
original commit. The response distinguishes `requested_plan_digest` and
`plan_digest`; the certificate binds the exact requested scope.

Pending plans, another live session's actions, or an adapter receipt without
the session's original commit custody cannot authorize live cancellation.
Absent live custody, an exact verified durable original requires its complete
anchored action census, accepted recovery lineage and current scoped grants.
Source history alone remains insufficient; malformed live custody cannot be
bypassed by falling back to recovery. Unknown or unavailable selection refuses.
Restore and restart never reconstruct old foreground effect handles.

## Drain and proof semantics

Automatic compensation for pause, labor and burrow setters now comes from the
qualified planning source. A mixed selection restores only units whose prior
boolean differs from the requested target. Existing assignments remain intact;
missing, unknown, unavailable or unadmitted facts never become an assumed
`false`. A complete no-op has no inverse. The planner seals the observed prior
values as dispatch guards. If an earlier step writes the same configuration,
the later setter has no automatic inverse from that initial source.

For an unfinished setter, the live adapter checks the sealed inverse against
its actual dispatch preimage and retains the effect identity. Cancellation
checks that this preimage matched and that the affected entity generations, revisions and
field facts still match the observed effect. This conservatively refuses even
an unrelated edit to the same entity; stop without compensation and review a
fresh action when configuration has changed. A later explicit setter of the
same field supersedes the earlier inverse even when its value does not change.
Pause compensation also refuses
after another explicit clock action, including an emergency reassertion, while
ordinary observation and time advancement do not by themselves supersede it.
The checks run before the cancellation request and again before the effect;
a refused finalization leaves its pending receipt and the world intact.

Explicit protective reassertions keep their authored policy. The registered
civilian-release compensation still restores protection under its sealed roster
identity checks when danger returns. Historical plan reconstruction preserves
its original exact digest; it neither restores live compensation custody nor
authorizes an unsafe legacy inverse. These rules do not make an already
Verified or Failed receipt cancellable.

The shared laboratory drain executes dependents before prerequisites and never
polls a deferred action into dispatch. Failed and Verified receipts stay
historical proof. Any physical work they still own is stopped separately with
its own evidence. Unknown work prevents a quiescence certificate and retains
the corresponding recovery and lease obligations.

The response contains measured `drain_progress`, action/physical-work rows,
the current anchor and, only once all selected actions are terminal and their
physical work is proven quiet, a `finalize_certificate`. The certificate's
canonical material includes the exact `plan:<digest>` scope and selected
actions. Repeating an already completed selected drain is idempotent.

A live response explicitly declares `selection_scope: "exact_original_commit"`
and `descendant_plans_included: false`. A later reviewed continuation is a
separate commit and requires its own selected cancellation if it must stop.
Other plans, the pending review, and the latest-action/latest-plan orientation
are preserved. A selected-plan certificate proves only that selected scope;
it does not certify all work in the session or fortress.

When output is constrained, response fitting preserves the exact selected scope,
action or plan identity, pause state, drain counters, original error and
finalization certificate (including a null certificate). The
`cancellation_receipt` tier explicitly omits oversized session orientation and
per-step historical detail, preserves continuity hazards, and offers a read-only
observation of current work. That follow-up does not reconstruct omitted
historical step evidence. A request accepted with `ok: true` still does not imply
that its drain is complete.

## Discover the stop from Explain

A typed `fortress.explain` request for `plan:<digest>` or `goal:<digest>`
includes a `cancellation` affordance. When exact commit custody, current
authority and budget allow a stop, it returns a concrete `fortress.cancel`
request with `mode: "stop_future_steps"`. Otherwise it gives the reason the
stop is unavailable, including already terminal and physically quiet work.

Inspection remains read-only: it does not consume an operation context, poll
work, change historical proof, or advance time. The complete affordance is
included in the explanation observation pin. A same-anchor custody or authority
change that changes the available stop invalidates an old explanation page.
An offer is not a grant; cancellation checks current authority again.

This foreground operation is independent of modern Tasks monitoring. It does
not change the pinned runtime's single-active-monitor ownership contract.

## Stop pursuing a quiet goal

After a complete drain, explicit `scope: "goal:<digest>"` with
`mode: "withdraw_goal"` can archive the unwanted original and release its active
goal slot. It requires current Observe, Query and reversible Plan authority and
retains all original proof without declaring success. Active/unknown work must
be resolved first. See [goal withdrawal](LAB_GOAL_WITHDRAWAL.md) for the exact
request, durable history, archive bounds and recovery limits.

## Regression coverage

`plan_cancellation_mcp_tests.rs` exercises the actual public cancellation
facade for a middle plan among three, preservation of pending/latest work,
complete preflight before effects, terminal failed-proof preservation, foreign
and lost custody, malformed/incomplete/reordered receipts, rebased aliases,
source bounds, repeat drains and current read-only affordances. The source is
part of the existing physical-work test module so it uses the same real
laboratory adapter fixtures. Execution evidence is tracked in
`IMPLEMENTATION_STATUS.md`; source coverage alone is not a passing test run.

Current focused execution passed all 14 laboratory effect-drain tests and all
23 tests in the complete output-budget module, using the actual source and real
pinned dependencies. These include the new adapter emergency-pause and bounded
cancellation-receipt regressions. The seven new emergency-pause facade/durable
tests remain unexecuted: complete MCP compilation and typecheck attempts were
SIGKILLed in unchanged Asupersync under memory pressure. These component results
do not establish full MCP, real stdio or native qualification.

Owner: `df-cancellation-progress-certificates-vvw`.
