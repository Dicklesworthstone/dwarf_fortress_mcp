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
digits. Existing `last_action`, `plan`, `oldest_open_plan`, and `session`
scopes keep their meanings. All three existing cancellation modes are supported.
An explicit `emergency_pause_and_drain` also pauses the shared fortress clock
under current clock authority; ordinary `stop_future_steps` needs no clock
advance or clock grant.

## Complete original selection

The current session must retain its original commit response and the adapter
must retain the matching complete original commit. Before the first effect,
selection checks:

- The owning session, canonical requested/actual digest and original plan ID.
- The complete nonempty action list within the current byte, entity and action
  budgets and the fixed 64-action ceiling.
- Every action in original step order, without duplicates or omissions, against
  the adapter's original commit, retained step and current action receipt.
- Current Observe authority and every selected nonquiescent action's original
  scoped capability/risk, plus the aggregate stop/compensation/pause budget.
  Emergency pause also checks current clock authority before any selected stop.

Each adapter effect still rechecks current authority at its own boundary.
This preflight prevents a known missing capability on an earlier prerequisite
from allowing a later selected action to stop first.

A witness-rebased commit can retain both the originally requested digest and
the actual newly sealed digest. Either retained alias selects the same complete
original commit. The response distinguishes `requested_plan_digest` and
`plan_digest`; the certificate binds the exact requested scope.

Pending plans, another session's shared-world actions, durable source history,
or an adapter receipt without the session's original commit custody cannot
authorize cancellation. An unknown or unavailable selection refuses. Restore
and restart do not reconstruct old foreground effect handles from source history.

## Drain and proof semantics

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

The response explicitly declares `selection_scope: "exact_original_commit"`
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

Owner: `df-cancellation-progress-certificates-vvw`.
