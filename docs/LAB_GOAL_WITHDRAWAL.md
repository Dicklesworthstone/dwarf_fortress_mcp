# Withdraw a quiet original goal without losing its history

The laboratory can explicitly stop pursuing an unwanted original goal while
keeping its source, action receipts, historical achievement and first reserve
endpoint. This releases one of the 64 active goal slots. It is useful after
canceling unwanted work, a failed goal, or a missed reserve horizon; none of
those outcomes should be converted into success merely to admit another plan.

Owner: `df-lab-goal-withdrawal-g6w`. This applies to the deterministic laboratory
and uses the existing eleven-tool surface.

## Request and authority

Call the existing cancellation tool with both the exact original goal selector
and the explicit withdrawal mode:

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<current session ID>",
    "scope": "goal:<64 lowercase original plan digest digits>",
    "mode": "withdraw_goal"
  }
}
```

Use the actual committed plan digest. Withdrawal does not accept a pending plan,
a witness-rebase request alias, the latest-plan shorthand, or an implicit mode.
It selects one original; continuation descendants remain separate originals.
The laboratory wire name is `fortress_cancel`, as returned by `tools/list`.

Current whole-fortress `observe` and `query` grants are required, together with
`plan` at risk `reversible` or above. The ordinary `plan: read_only` grant does
not permit this ledger change. Negotiate the stronger Plan grant explicitly
when opening a session that needs withdrawal. Each call rechecks current grants,
source and output budgets, the complete sealed original and its ownership.

A current-process original must belong to the calling session. A verified
quiet original recovered from the same durable fortress can be withdrawn under
fresh authority. A historical owner ID does not recreate dispatch authority.

## Stop physical work before withdrawing its goal

Every original action must already be terminal and its exact physical work
proven quiet. An active, missing, replaced or uncertain creator refuses
withdrawal. If live original handles remain available, use the existing
`scope: "plan:<digest>"`, `mode: "stop_future_steps"` cancellation first, then
inspect its drain evidence. Withdrawal itself never advances time, stops game
work, compensates an effect, or changes an action receipt.

Canceled deferred steps need positive evidence that they never dispatched.
The withdrawal certificate retains that original disposition separately from
the action's immutable `Cancelled` receipt. Missing entities alone do not prove
nondispatch or quiescence. Recovered originals additionally require complete
archived final receipts and verified recovery lineage; an unfinished durable
commit blocks withdrawal.

Restore-abandoned pursuits are refused by this first increment. They no longer
have an active lineage, and a missing entity after a rewind cannot establish
that earlier physical work is safely retired. Withdrawal does not provide a
new stop operation for active recovered work.

## Outcome and discovery

A successful receipt reports `status: "withdrawn"`, the exact `plan_digest`,
`active_objectives`, `archived_objectives`, and an immutable `withdrawal`
certificate containing its anchor, requester and evidence digest.
`physical_work_changed` and `goal_success_claimed` are false. The certificate's
physical-quiescence statement applies to its archived withdrawal frontier.
It is not a statement about every later world or checkpoint restore.

Repeating the exact request is idempotent and returns the same certificate with
`already_withdrawn: true`. The world anchor, pending plan, action receipts and
original achievement remain unchanged. A full active book can then accept its
previously blocked pending review, subject to the ordinary fresh commit checks.

`fortress.explain(entity_id="goal:<digest>")` and the equivalent plan selector
include a `withdrawal` affordance. It contains a concrete request when all
current conditions permit withdrawal, or the reason it is unavailable. The
complete affordance participates in the explanation's observation pin, so
withdrawal or changed authority invalidates older explanation pages even at an
unchanged world anchor. Inspection remains read-only.

Withdrawn originals remain in the plan catalog with retention
`withdrawn_objective`; their exact goal/source resources and typed plan archives
remain available. Goal projections report `archived: true` and
`status: "withdrawn"`, with achievement, withdrawal and any
`retained_reserve_endpoint` kept separate. Compact receipts retain the full
withdrawal outcome. Their history cannot acquire a new achievement or a new
reserve endpoint, and selected waiting or `continue_goal` refuses renewed
pursuit. New work requires a new reviewed request.

## Durable publication and bounds

The existing original-source, terminal-receipt and reserve-endpoint encodings
are unchanged. An additive `U` journal record retains the fortress, original
digest, exact withdrawal anchor, requesting session, proof digest and strictly
ordered positive nondispatch step IDs. The digest binds the immutable original
source, complete terminal census and withdrawal frontier. The semantic owner
independently verifies exact current physical identities before publication.

The world frontier must already be durably published. A new marker is installed
in memory only after the durable append succeeds. A failed or uncertain write
keeps active custody, fences later effects, and requires the existing reopen
and replay recovery path. It does not report a successful archive.

There are at most 64 active originals and 256 explicitly withdrawn originals
per fortress, for a total of 320 retained originals. Reaching the archive limit
refuses another withdrawal; it does not silently delete history. Each request
still obeys its negotiated complete-source, action, byte, output and wall-time
bounds, with a 64-step withdrawal ceiling. Session history has a fixed 4,096
pinned-version ceiling; source admission can still refuse a large request.

Compaction retains the original source, final receipt map, existing endpoint and
withdrawal record, including the exact withdrawal snapshot as a root. Reopen
verifies those objects and rechecks physical quiet at the historical withdrawal
frontier. Later restore preserves the archive without turning it back into an
active pursuit. Older executables fail closed on the unknown `U` tag rather than
silently dropping its history.

## Validation scope

Storage regressions exercise active capacity, separate archive capacity,
compaction and repeated reopen, exact endpoint preservation, immutable retries,
forged custody and failed publication. Owner/facade regressions cover a full
unmet goal book, current authority and budgets, deferred cancellation, recovered
withdrawal, publication fencing, exact public selectors, discovery and retained
compact evidence. Executed results are recorded in `IMPLEMENTATION_STATUS.md`;
authored tests alone are not execution evidence.
