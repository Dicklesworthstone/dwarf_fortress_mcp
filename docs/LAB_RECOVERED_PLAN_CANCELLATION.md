# Stop an original laboratory project after a durable restart

A durable laboratory restart retains original plans, anchored action receipts,
and physical work, while discarding the old process's action handles and grants.
The exact original can now be stopped through the ordinary cancellation tool
under freshly negotiated authority.

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

The selected scope contains only the exact original plan. Other originals,
continuation descendants, and the pending review remain independent. Selection
does not reconstruct dispatch handles, adopt untracked work, or retry a prior
effect.

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
  custody, and foreign live ownership refuse the selection.
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
