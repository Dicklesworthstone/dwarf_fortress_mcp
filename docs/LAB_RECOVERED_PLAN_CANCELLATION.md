# Stop an original laboratory project after a durable restart

A durable laboratory restart retains original plans, anchored action receipts,
and physical work, while discarding the old process's action handles and grants.
Original work can be stopped through the ordinary cancellation tool under
freshly negotiated authority. Select one exact original, a set of up to eight
live or recovered originals, discover the oldest
unfinished original, or drain the complete current session selection.

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<current recovered session>",
    "scope": "plan:<exact original 64-character lowercase plan digest>",
    "mode": "stop_future_steps"
  }
}
```

Use the original committed digest retained before the restart. A typed
`fortress.explain` request for that `plan:<digest>` or `goal:<digest>` offers
the concrete cancellation request when its current custody, authority, and
budgets permit it. An offer remains advisory; execution checks again.

## Stop the oldest unfinished original without retaining its digest

The same tool can discover and stop one oldest original:

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<current recovered session>",
    "scope": "oldest_open_plan",
    "mode": "stop_future_steps"
  }
}
```

Recovered originals are ordered by their original observation epoch and
sequence, with the sealed digest breaking ties. They precede work newly
committed in the recovered process. Selection inspects fresh physical work as
well as retained dispositions: a terminal proof does not hide work that is
still running. Quiet originals are skipped, and a successful repeat can move
on to the next original. An unavailable or ambiguous older original refuses
selection rather than silently choosing a newer project.

Discovery and the selected stop share the existing source, action and wall-time
bounds. Current read authority precedes discovery; current scoped effect
authority is still required to stop the selected original. `oldest_open_plan`
does not grant compensation for recovered work, include continuation
descendants, or change the pending review. Its response retains the requested
scope and the exact selected original identity, and its finalization
certificate covers only that original. Use the returned digest for later exact
inspection or cancellation.

This is deterministic laboratory functionality under
`df-cancellation-progress-certificates-vvw`. Native DFHack cancellation and
production admission are separate boundaries.

## Drain recovered and newly committed work together

For only a chosen group, use `scope: "plans:<digest>,<digest>"`. The complete
set is admitted before any stop or pause, including every recovered archive and
current effect grant. Its final certificate covers those exact originals while
unselected recovered and live work continue. The response preserves each
original's source of custody and never restores dispatch handles. See
[selected original sets](LAB_PLAN_SET_CANCELLATION.md) for complete identities,
output bounds, repeat requests and failure semantics.

Use session scope to stop every eligible recovered original and every original
currently owned by the requesting live session:

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<current recovered session>",
    "scope": "session",
    "mode": "stop_future_steps"
  }
}
```

The coordinator reconstructs live custody from complete original commit
receipts, including canonical aliases, instead of trusting only the volatile
open-action list. It verifies the full recovered selection and current scoped
authority for every selected original before stopping anything. Source, census,
action, output and wall-time bounds apply to the complete call, including the
final proof and possible cancellation-history growth. If the accumulated source
cannot fit, use `oldest_open_plan` or one exact original to drain a smaller
selection. A missing grant or invalid older original cannot leave newer work
stopped by a partial preflight.

The drain stops newer originals before older originals and reverses original
step order within each one. Every physical stop is published before the next
effect. Terminal historical proof and already-completed game progress remain
intact. No deferred step is dispatched, no old handle is restored, and the
pending review and latest-plan orientation remain unchanged. The session stays
open after the drain.

Other live members retain their own work. Unknown or untracked work is never
adopted and prevents a complete session certificate. Recovered compensation
remains unavailable: a mixed selection with recovered work refuses
`compensate_reversible` before effects.

`emergency_pause_and_drain` admits one shared clock pause plus the whole
selected drain. An empty selection can still pause the world, including
untracked work, while reporting that the drain is incomplete. Invalid selected
custody or insufficient authority/budget refuses before that pause.

Session responses declare `selection_scope: "session_originals"` and retain
`selected_plans_digest`, which identifies the complete admitted original set.
Aggregate progress distinguishes original plans, live actions, recovered steps,
completed stops, and remaining work. Final certification requires a fresh
complete selected census, quiet exact physical identities, no unresolved
untracked work, and confirmed durable publication. A failed publication retains
actual partial progress and the full selection digest, stops subsequent effects,
and leaves the certificate null. These fields survive the minimum admitted
1,500-token response even when detailed per-original history is omitted.

## What the stop changes

The operation stops the original's remaining mining, resource harvesting,
construction, furniture placement, and finite production work in reverse step
order. It does not advance game time. Completed excavation, consumed resources,
produced items, and other partial progress remain in the world.

An unfinished dispatched step receives a fresh cancellation disposition and
current physical-work evidence. A previously Verified or Failed receipt keeps
its original state and proof anchor; if its physical work is still running,
that work is stopped with separate evidence. Positive archived nondispatch
remains nondispatch, and deferred steps are never started by cancellation.

An exact or oldest-original selection contains only that original plan.
Other originals and continuation descendants remain independent. Session scope
explicitly includes every eligible original in its complete selection, while
the pending review remains independent. Selection does not reconstruct dispatch
handles, adopt untracked work, or retry a prior effect.

`emergency_pause_and_drain` additionally pauses the shared clock under current
ControlClock authority, publishes that frontier, and then stops the selected
work. `compensate_reversible` is refused for recovered originals because recovery
does not recreate the inverse-operation handles needed to compensate them.

## Admission before effects

The owner verifies the complete original source and sealed plan, its accepted
durable recovery lineage, and every original step's anchored disposition.
Archived goal achievement alone cannot replace missing action receipts.
Current physical identity is checked independently of historical proof.

The complete selection is admitted before any stop or emergency pause:

- Current whole-fortress Observe and Query grants are required.
- Each selected unfinished or physically active step requires its current
  original scoped effect capability and risk allowance.
- Spatial and entity ownership are checked against current leases. Expired
  lease time does not excuse another owner's unresolved physical work.
- Unknown physical identity, conflicting untracked work, incomplete or ambiguous
  custody, and foreign ownership of selected effects refuse the selection.
  Known work owned by another live session remains outside session scope.
- Withdrawn and restore-abandoned originals are not eligible. Recovery accepts
  the exact sealed digest and does not infer former live request aliases.
- The complete world, original source, archived custody, and step/ownership
  census must fit the negotiated bounds. The fixed ceiling is 64 original steps
  and 16 MiB of source, with a shared wall-time and action allowance.
- A response allowance of at least 1,500 tokens and 6,000 bytes is required.
  Emergency pause and physical stops share the aggregate action budget.

Each adapter stop repeats the current anchor, authority, recovery-lineage and
physical-identity checks. Historical preparation expiry does not authorize a
new effect or prevent a freshly authorized cleanup of still-running work.

## Read the outcome, not only `ok`

Recovered responses identify
`selection_scope: "exact_recovered_original"`,
`selection_origin: "durable_recovery"`, and
`dispatch_handles_restored: false`. They preserve the originating owner as
historical metadata and name the current requesting session separately.

`steps` distinguishes each archived disposition and proof anchor from current
work. `physical_drains` contains fresh per-step before/after observations,
original-custody identities, and stop evidence. `drain_progress` reports the
number of stopped effects, newly cancelled steps, separately stopped terminal
work, remaining nonterminal steps and remaining physical work.

A finalization certificate is issued only when every original disposition is
final, its exact physical work is currently quiet, and durable publication is
confirmed. The certificate binds the original plan object, archived custody,
complete final census, and current anchor. It certifies the selected original,
not goal success, unrelated work, or future fortress state.

Repeating a completed stop performs no new physical effect. Fresh current
inspection and authority still apply. A zero-tick selected wait can inspect the
same final outcome, and an unwanted quiet goal can then be explicitly withdrawn
with `scope: "goal:<digest>"`, `mode: "withdraw_goal"`. Closing the session
remains a separate operation with its full session-wide quiet-work checks.

## Durability and partial failure

Every physical stop publishes its new world and action disposition through the
existing atomic durable frontier before another stop begins. Terminal historical
proof is retained. Retired original action maps survive compaction and later
recovery without renewing the original goal or its authority.

If publication fails after a physical stop, the response preserves the actual
in-memory stopped state, reports `durability_pending`, withholds the finalization
certificate, and stops further effects. The durable fault fences subsequent
work until ordinary recovery resolves it. A partial failure never claims that
the whole original was cancelled or that the stopped frontier is durable.

Output fitting preserves the exact selected original, measured progress,
publication status, pause result and nullable certificate. An abbreviated
receipt explicitly omits detailed step history and offers current read-only
inspection; a later observation cannot reconstruct omitted historical evidence.

## Related workflows

- [Original-plan cancellation](LAB_PLAN_CANCELLATION.md)
- [Selected original-plan waits](LAB_SELECTED_PLAN_WAIT.md)
- [Goal withdrawal](LAB_GOAL_WITHDRAWAL.md)
- [Quiet session close](LAB_SESSION_CLOSE.md)

Executed test results and remaining qualification limits are recorded in
`IMPLEMENTATION_STATUS.md`; this workflow document does not establish a live
game or production admission claim.
