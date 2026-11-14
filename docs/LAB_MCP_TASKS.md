# Modern MCP Tasks in the laboratory

The laboratory `dwarf-fortress-mcp serve` entry can retain a sealed plan as a
real **MCP 2026-07-28 Task**. Use `fortress_commit(as_task=true)` when the client
needs an opaque handle for the original plan's completion, failure or
cancellation. The ordinary commit call remains available. Both paths use the
same session authority, plan digest, idempotency receipts and obligation engine.
Use `fortress_wait(plan_digest=..., as_task=true)` to attach a new passive Task
to an existing original, including a verified durable recovered original.
That second kind of Task observes the original without acquiring its effect
or cancellation ownership.

This is process-local laboratory execution. It does not admit a live DFHack
tuple, add a production protocol, or qualify native game behavior. Task handles
and task history last until the server process exits, including when the
underlying laboratory fortress uses a durable archive.

The original goal has a separate durable lifetime. For a durable fortress,
session open and observations retain its exact source, current truth, immutable
first achievement and restore abandonment even after its action commit retires
or the process restarts. Current Observe authority gates that history. Recovery
does not recreate a Task handle or grant another session ownership of the old
task; it exposes the retained goal for observation and explicit follow-up.
An explicitly requested observation Task receives a new process-local handle
and preserves the original owner's identity as evidence.

## Negotiate and create a task

Every request carries the modern metadata already described in
[`FASTMCP_INTEGRATION.md`](FASTMCP_INTEGRATION.md). A task-capable client includes
the Tasks extension in its capabilities:

```json
{
  "io.modelcontextprotocol/protocolVersion": "2026-07-28",
  "io.modelcontextprotocol/clientCapabilities": {
    "tools": { "listChanged": true },
    "extensions": { "io.modelcontextprotocol/tasks": {} }
  },
  "io.modelcontextprotocol/clientInfo": {
    "name": "fortress-agent",
    "version": "1.0"
  }
}
```

Open a laboratory session and prepare a plan through the usual eleven-tool
workflow. The session must have `observe`, `query` and every action capability the plan
requires. For an excavation followed by a bed, that includes `plan`,
`designate`, `construct`, and the checkpoint authority required by its risk
policy; explicit clock advancement also requires `control_clock`. See the
complete starter-fortress example in
[`LAB_SEMANTIC_ACTIONS.md`](LAB_SEMANTIC_ACTIONS.md).

Then send `tools/call` with these parameters, plus the metadata above in
`params._meta`:

```json
{
  "name": "fortress_commit",
  "arguments": {
    "session_id": "<session ID returned by open_session>",
    "plan_digest": "<exact digest returned by plan>",
    "as_task": true
  }
}
```

The modern response has `resultType: "task"`, an opaque `taskId`, and initially
`status: "working"`. The task fields are at the top level of the JSON-RPC
`result`; they are not wrapped in a `task` field. Creation acknowledges retained
work, not game completion. The application-owned supervisor re-enters the
ordinary commit path with that exact session and digest, rechecking authority
before any effect. A changed or invalid plan fails with its refusal evidence.

Omitting `as_task`, or setting it to `false`, preserves the normal synchronous
commit response. Requesting a task without negotiated Tasks support is refused
before a task or effect is created. No additional fortress tool is registered.

## Attach a Task to an existing or recovered original

An already committed original can be observed through the same negotiated
Tasks API, without repeating its commit. Supply the exact original digest and
the current session explicitly:

```json
{
  "name": "fortress_wait",
  "arguments": {
    "session_id": "<current session ID>",
    "plan_digest": "<exact committed original digest>",
    "max_game_ticks": 0,
    "as_task": true
  }
}
```

The current session needs `observe` and `query`, complete eligible original
custody and the Task output/source budgets. It does not need the original
mutation capabilities merely to observe. Admission verifies the complete
action census both before returning a Task request and at the store's creation
boundary. A pending uncommitted plan, another live session's original,
incomplete or mismatched evidence, restore-abandoned or explicitly withdrawn
pursuit, and lost current read authority are refused. A recovered original
must pass the same exact durable-custody checks as a selected foreground wait;
opening a new session alone does not grant original dispatch handles.

Omit `max_game_ticks` or set it to zero. A positive tick request, any non-null
watch selector or policy, an unknown argument, a malformed digest or a
non-Boolean `as_task` is refused before Task creation. Ordinary foreground
waits with `as_task` omitted or false retain their existing behavior.

The two Task modes make their cancellation scope explicit in discovery,
detail and terminal evidence:

| Creation | `monitor_kind` | `cancellation_scope` | Effect ownership |
|---|---|---|---|
| `fortress_commit(as_task=true)` | `commit_original` | `original_plan` | Retains the original commit and authorized drain behavior. |
| `fortress_wait(plan_digest=..., as_task=true)` | `observe_original` | `monitor_only` | Observes the exact original; commits, clock changes, replacement actions and physical drains are never dispatched by this Task. |

An observation Task consumes only already-retained evidence. Its creation,
supervisor wakeups, reads and cancellation do not advance time, poll actions,
hydrate an archived reserve snapshot, capture an endpoint or publish durable
history. Use an explicit selected foreground `fortress_wait` to make progress
or retain a required exact endpoint, and inspect the Task again. If an archived
reserve snapshot is not yet available in the current version book, the Task
reports the missing evidence; a selected zero-tick foreground wait can perform
the separately explicit load or observation when it is eligible.

Completion still requires the complete original goal and exact selected-plan
proof, including the original reserve horizon. Verified harvesting alone does
not finish a larger room or production project. A meaningful selected stop,
such as a failed action, unavailable physical identity, unmet original goal or
required continuation, finishes the observation Task as `failed` with its
selected outcome and follow-up request. Its failure may coexist with active
physical work. Review the exact original and explicitly authorize any cleanup
or continuation through the existing foreground tools.

**Cancelling an observation Task stops only that monitor.** It requires current
Observe authority but does not require Query or mutation grants just to stop
observing. The retained `monitor_finalized` record states that no effect
cancellation was requested and no physical-quiescence certificate was issued.
The original action receipts and physical work remain unchanged. Its
`owns_original_dispatch: false` refers to the Task; the nested selected-plan
projection can separately identify the current session as the original owner.
Recovered projections always retain the historical owner and state that
dispatch handles were not restored.

## Observe progress and prove completion

The pinned modern API provides these methods:

| Method | Laboratory behavior |
|---|---|
| `tasks/get` with `{"taskId":"…"}` | Returns the retained task state; a completed task includes its result, and a failed task includes failure evidence. |
| `tasks/cancel` with `{"taskId":"…"}` | For a commit Task, cancels its original plan through the bounded drain. For an observation Task, stops only the monitor and leaves physical work unchanged. |
| `tasks/update` | The framework's typed input-response method. Laboratory plan tasks do not request client input, so it cannot be used to alter action state or manufacture completion. |

There is no modern `tasks/list` or separate `tasks/result` method on this pin.
Task discovery is available in the session's Agent Turn and handoff, and through
the `df://session/{session_id}/tasks` resource. Its bounded pages lead to
`df://session/{session_id}/task-{task_id}` for the original plan's detailed
progress, action states, evidence and cancellation certificate. URI knowledge
does not grant authority: reads check the session's current `observe` grant.

The supervisor follows receipts produced by foreground tools. **Only an
explicit `fortress_wait` advances laboratory game time.** `tasks/get`, resource
reads and supervisor wakeups neither advance time nor dispatch a deferred
action. A paused fortress remains paused. Use bounded waits and inspect
`open_actions_remaining`, each action's state, and the task result. All actions
in the original plan must verify, their physical work must become quiescent, and
current eligible evidence must establish the original plan-level goal before its
task becomes `completed`.

Goal proof and physical work are separate. A deadline can produce an immutable
`Failed` action receipt while its work order, designation or construction still
progresses. An explicit early terminal predicate can likewise produce a
`Verified` receipt before that physical work finishes. Neither proof outcome
silently stops game work. The task detail exposes both:

| Field | Meaning |
|---|---|
| `proof_status` | The original goal outcome: `working`, `completed`, `failed` or `cancelled`. A failure is reported for failed/indeterminate actions or finished quiet work whose original goal is false or unknown. |
| `action_proof_status` | The separate aggregate action-receipt outcome; verified work does not by itself establish the original goal. |
| `original_goal` | Current source-qualified truth, original predicate, actual committed digest and observation anchor, together with separately labeled immutable first achievement and restore abandonment. |
| `original_goal_satisfied` | The complete original goal is established, including any required first exact reserve endpoint. True current stock alone does not establish a future reserve. |
| `original_goal_pending` | A future reserve still needs its explicit endpoint observation; finished actions can leave the Task working with no physical cleanup required. |
| `selected_plan_wait` | Read-only inspection of the exact original project, stage truth, physical work, reserve evidence and any separately reviewed continuation. |
| `needs_replan` | The original work has finished below its original goal. Review a new plan explicitly; the monitor dispatches no replacement. |
| `status` | The monitor's lifecycle. It remains `working` while an original action is nonterminal or its physical work is active or unknown. |
| Each action's `work_state` | `never_dispatched`, `active`, `quiescent` or `unknown`, with the entity identity and current observed anchor. |
| `drain_progress.remaining_nonterminal` | Original action receipts that have not reached a terminal state. |
| `drain_progress.remaining_work` | Original actions whose physical work is active or cannot be proven quiescent. |
| `drain_progress.quiescent` | True only when every original receipt is terminal and every physical effect is proven quiescent. |

Production goals retain all requested stock minima. Consumption can leave a
quota unmet even after every requested batch verifies. Such a finished task
reports `failed`, `needs_replan: true` and the current goal evidence. Missing or
ineligible goal evidence stays `unknown` and requires reconciliation. An
idempotent witness-rebase alias resolves to the actual original committed goal;
a later unrelated plan cannot replace it.

Future reserves keep their original absolute game tick. Even if all production
actions have verified and current stock is sufficient, the Task remains
`working` until an explicit selected `fortress_wait` retains the first exact
endpoint. Task reads, supervisor wakeups and cancellation checks only consume
already-retained evidence: they do not record an endpoint, load an archived
snapshot into the version book, poll actions or advance time. The recommended
wait names the Task's exact original `plan_digest`, so progression stops at its
reserve frontier.

At the exact horizon without a retained endpoint, the Task stays pending and
recommends `fortress_wait(plan_digest=..., max_game_ticks=0)` to capture that
observation explicitly. After the horizon, a missing endpoint produces
`reserve_unobserved`; a first false or unknown endpoint produces `reserve_unmet`
or `reserve_unknown`. Later sufficient stock cannot repair these outcomes.
The nested `original_goal.predicate_truth` continues to report current stock
truth independently. Full and bounded Task results retain the selected reserve
outcome and its exact anchor. No Task operation renews the original deadline.

For a resource-backed room or production pursuit, verified harvesting is only
the current stage. When the original complete project remains unmet and the
physical work is quiet, the Task fails with `needs_replan: true` and the
eligible explicit continuation request. It never dispatches the next stage.

For a commit Task with a failed goal, `status: "working"` therefore means retained monitoring or
cleanup, not that its failed proof is still pending. Terminal but active actions
remain in `agent_turn.active_work.actions`, and `cleanup_required` identifies
the need for an explicit stop. Use `tasks/cancel` with that task's opaque handle
to drain its original plan, even after another plan has been committed.
Observation and request-phase cancellation do not stop these effects. Cleanup
requires the current original action capability and scope at finalization.

Commit Task completion payloads retain the original plan digest, each action's exact
receipt digest, observed anchor and evidence references. A later unrelated
commit does not replace those original action identities. Normal retries of
the same sealed commit still return the original receipt; creating a task is
not a new idempotency namespace for effects. Repeated reads of a terminal task
return the same retained terminal result. Observation Task payloads retain the
complete selected action census, original and stage goal outcomes, exact
reserve frontier, recovery identity and custody diagnostics. They do not
reconstruct action handles or borrow another Task's effect ownership.

At smaller output budgets, a terminal result contains a bounded summary,
action counts, the observed anchor and a digest of the complete retained
evidence. Follow its evidence continuation to
`df://session/{session_id}/task-{task_id}~evidence-{byte_offset}`. Each page
states its offset, total byte count and document digest. Concatenate only
pages with one unchanged digest before parsing the complete JSON document.
The same bound includes JSON escaping in resource envelopes. Summary output
does not discard the underlying proof.

An obligation first proved after its game-time deadline fails. Deferred
descendants of failed, cancelled or compensated prerequisites are closed with
evidence that they were never dispatched. Indeterminate effects remain
explicitly `indeterminate` in the failed task's evidence, with
`recovery_class: "reconciliation_required"` and `blind_retry_allowed: false`.
A failed transport task does not erase unresolved engine work. If the original
receipt can no longer be read after restore or monitor recovery, failure
evidence explicitly reports unknown physical work and
`physical_quiescent: false`; unavailable counts are `null`, never an invented
zero. Bounded terminal
summaries retain `proof_status`, `action_proof_status`, compact original-goal
evidence, replanning status, drain progress and physical quiescence as well as
the continuation to complete evidence.

## Cancel the original plan

For a commit Task, `tasks/cancel` targets the session and plan digest captured at task creation.
It continues to target that plan after the session prepares or commits other
work. Ordinary `fortress_cancel(scope="plan")` continues to refer to the last
committed plan.

Cancellation processes dependents before prerequisites. It reads retained
receipts without polling eligibility, records `CancelRequested`, reports
measurable drain progress, and finalizes only after the original action receipts
are terminal and their physical work is quiescent. The task detail retains both
the request-phase progress and the finalization certificate. A terminal action
with active work receives a separate stop-only drain receipt; its original
`Failed` or `Verified` proof, evidence and receipt digest stay unchanged. All
work identities are inspected again at the final anchor before certification.
Stopping future excavation or construction does not undo completed game
progress. Missing work after a known dispatch, replacement identities and
ineligible lifecycle facts remain unresolved and cannot certify a drain.

Each drain phase rechecks current Observe, Query and action authority. A fully
verified and physically quiescent plan whose complete original goal holds,
including any required retained exact reserve endpoint,
cannot be cancelled. A verified plan
whose early proof leaves physical work active can be cleaned up without
rewriting that proof. Explicit `tasks/cancel` finalizes the transport task as
`cancelled`; if the goal had failed, both drain phases still retain
`proof_status: "failed"` and the original failure evidence. Without explicit
transport cancellation, a monitor becomes `failed` once failed work is quiet,
or `completed` once verified work is quiet and the original goal holds.
An unfinished future-reserve monitor can be cancelled after its physical work
is already quiet. Its drain certificate proves only physical quiescence;
`proof_status: "working"` and `original_goal_pending: true` remain explicit,
and cancellation creates no endpoint evidence. Missing current read authority
or exhausted source budgets cannot bypass the completed-goal cancellation guard.

The bounded laboratory drain completes before
cancellation intent is forwarded to the upstream store. Both ordered phases
are retained, but a separate client poll between request and finalization is
not guaranteed. A failed drain refuses transport cancellation and retains the
original work and an explicit reconciliation diagnostic instead of claiming
quiescence. The supervisor and stdio server share a caller-owned Asupersync lifetime; no game
worker is detached.

Transport-owned cancellation is recorded as two ordered
`task_cancel_phase` engine events in a `dfmcp.replay.bundle/2` transcript.
Each event identifies the exact original plan and the `request` or
`finalize` phase. Recording holds the same engine ownership as the drain and
retains failed phases, including any partial effects. Both before and after
witnesses bind the canonical anchor, next request counter, current scoped
authority, all six source budgets, and the original action receipts, evidence,
physical work and held leases. The normalized result digest covers the drain
result or the exact error code and message.

Replay validates the complete bounded transcript before opening a fresh
private laboratory session. It compares the before witness before attempting
any cancellation, then calls the same drain helper with the recorded selector
and phase under that same engine ownership. A mismatched input publishes no
snapshot history, goal progress or replay event. That helper rechecks current authority and budgets; the transcript
never supplies replacement grants or action identities. Replay checks the
outcome, after witness and evidence digest and stops at the first difference.
A request-counter or anchor gap between a phase and its recorded predecessor,
a truncated log, or shared/durable external state still makes the export
nonreplayable.

Five direct typed result locations may replace the current owner session ID with
`owning_session`: `original_goal.owner_session_id`,
`original_goal.continuation_request.arguments.session_id`,
`selected_plan_wait.original_owner_session_id`,
`selected_plan_wait.continuation_request.arguments.session_id`, and
`selected_plan_wait.next_step.arguments.session_id`. User text and predicate
data are never normalized. When the exact original was explicitly withdrawn,
its nested withdrawal projection must also match verified retained custody:
the original source, complete terminal census, positive nondispatch evidence
and archived quiet frontier. Only its current original owner and requesting
session inputs are replaced by a fixed comparison identity, and its proof is
resealed through the existing typed withdrawal codec. The published withdrawal
is unchanged. A missing, foreign or invalid withdrawal proof cannot be
normalized and makes the export explicitly nonreplayable.

A drain certificate is verified against its original material before its digest
is rederived over the same normalized material; invalid certificate identities
remain distinguishable. Withdrawal proof verification and comparison are
internal replay evidence, not renewed pursuit or authority.

Tool-only sessions retain exactly the existing `dfmcp.replay.bundle/1`
encoding and digest domain. Version 2 has a separate digest domain and adds
request counters to tool entries. Passive observation Task cancellation has no
engine effects and adds no drain events. Replay reproduces engine transitions,
not process-local Task identities, Task notifications or native game behavior.

## Capacity and retention

The first implementation admits **one active task monitor per server process**
and retains at most **256 task records**, shared by both commit and observation
Tasks. The upstream runner owns one active
supervisor handoff at a time, and this bound is enforced before dispatch so a
second accepted task cannot starve behind an unfinished monitor. Ordinary
fortress tools can still operate on multiple plans and sessions.

Task admission accepts at most 64 original actions and requires at least
1,500 output tokens and 6,000 bytes so a complete bounded summary can be
returned. The ordinary conservative session defaults satisfy these bounds.

Tasks have process-lifetime retention (`ttlMs: null`), bounded by the record
limit. A time-to-live must not silently erase an unfinished game obligation.
Once the retained-record limit is reached, new task creation is refused before
effects; retained evidence remains readable. Larger concurrent supervision
and restart-persistent task identities remain future work, as do independently
observable asynchronous drain stages for adapters that cannot drain in one
bounded call.

## Regression entry points

```bash
cargo test --locked --offline -p dfmcp-mcp --test tasks_tests
cargo test --locked --offline -p dfmcp-mcp --lib server::task_session
cargo test --locked --offline -p dfmcp-mcp --lib task_service::observer_request_tests
cargo test --locked --offline -p dfmcp-mcp --lib replay::task_tests
python scripts/test_replay_bundle_schema.py
cargo test --locked --offline -p dwarf-fortress-mcp --test modern_tasks_golden
```

The process tests use the real `serve` binary and explicit laboratory game
ticks, with bounded response waits and owned reader cleanup. They cover
negotiation, task discovery, original-plan progress, completion evidence,
idempotent effects, capacity refusal, cancellation and deadline failure. Test
results qualify only their exercised laboratory behavior; they do not replace
the repository's full qualification gates.

The observation regressions exercise current-session and durable-recovered
originals, complete-census admission, current grants, malformed clock/watch
selectors, shared Task capacity, and each upstream store cancellation entry.
They compare snapshots, request counters and original action receipts before
and after passive observation and cancellation; physical work must remain
active when only its monitor is stopped.

The Task replay regressions cover exact export/replay/re-export equality,
each upstream cancellation entry, preservation of a different active plan and
pending review, verified withdrawal history, failed and partially applied phases, passive observers,
before-effect tampering refusal, output evidence divergence, closed input
validation and truncated or incomplete recording. The schema script checks
both generations and field-mutation shapes; it is not an execution test.

## Foreground stop of a selected original plan

A session can call `fortress.cancel` with `scope: "plan:<original-plan-digest>"`
to stop one exact retained commit independently of which plan is latest or
which monitor is active. The complete original action set and current scoped
authority are checked before effects, and physical drain evidence remains
separate from historical proof. This does not add Task owners or change the
single-active-monitor contract. See `LAB_PLAN_CANCELLATION.md`.
