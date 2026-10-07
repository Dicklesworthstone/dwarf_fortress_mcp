# Modern MCP Tasks in the laboratory

The laboratory `dwarf-fortress-mcp serve` entry can retain a sealed plan as a
real **MCP 2026-07-28 Task**. Use `fortress_commit(as_task=true)` when the client
needs an opaque handle for the original plan's completion, failure or
cancellation. The ordinary commit call remains available. Both paths use the
same session authority, plan digest, idempotency receipts and obligation engine.

This is process-local laboratory execution. It does not admit a live DFHack
tuple, add a production protocol, or qualify native game behavior. Task handles
and task history last until the server process exits, including when the
underlying laboratory fortress uses a durable archive.

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
workflow. The session must have `observe` and every action capability the plan
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

## Observe progress and prove completion

The pinned modern API provides these methods:

| Method | Laboratory behavior |
|---|---|
| `tasks/get` with `{"taskId":"…"}` | Returns the retained task state; a completed task includes its result, and a failed task includes failure evidence. |
| `tasks/cancel` with `{"taskId":"…"}` | Requests cancellation of the task's original plan and runs the bounded drain. |
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
in the original plan must verify and their physical work must become quiescent
before its task becomes `completed`.

Goal proof and physical work are separate. A deadline can produce an immutable
`Failed` action receipt while its work order, designation or construction still
progresses. An explicit early terminal predicate can likewise produce a
`Verified` receipt before that physical work finishes. Neither proof outcome
silently stops game work. The task detail exposes both:

| Field | Meaning |
|---|---|
| `proof_status` | The original goal outcome: `working`, `completed`, `failed` or `cancelled`. A failure is reported as soon as a failed or indeterminate action is observed. |
| `status` | The monitor's lifecycle. It remains `working` while an original action is nonterminal or its physical work is active or unknown. |
| Each action's `work_state` | `never_dispatched`, `active`, `quiescent` or `unknown`, with the entity identity and current observed anchor. |
| `drain_progress.remaining_nonterminal` | Original action receipts that have not reached a terminal state. |
| `drain_progress.remaining_work` | Original actions whose physical work is active or cannot be proven quiescent. |
| `drain_progress.quiescent` | True only when every original receipt is terminal and every physical effect is proven quiescent. |

For a failed goal, `status: "working"` therefore means retained monitoring or
cleanup, not that its failed proof is still pending. Terminal but active actions
remain in `agent_turn.active_work.actions`, and `cleanup_required` identifies
the need for an explicit stop. Use `tasks/cancel` with that task's opaque handle
to drain its original plan, even after another plan has been committed.
Observation and request-phase cancellation do not stop these effects. Cleanup
requires the current original action capability and scope at finalization.

Completion payloads retain the original plan digest, each action's exact
receipt digest, observed anchor and evidence references. A later unrelated
commit does not replace those original action identities. Normal retries of
the same sealed commit still return the original receipt; creating a task is
not a new idempotency namespace for effects. Repeated reads of a terminal task
return the same retained terminal result.

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
summaries retain `proof_status`, drain progress and physical quiescence as well
as the continuation to complete evidence.

## Cancel the original plan

`tasks/cancel` targets the session and plan digest captured at task creation.
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

Each drain phase rechecks current observation and action authority. A fully
verified and physically quiescent plan cannot be cancelled. A verified plan
whose early proof leaves physical work active can be cleaned up without
rewriting that proof. Explicit `tasks/cancel` finalizes the transport task as
`cancelled`; if the goal had failed, both drain phases still retain
`proof_status: "failed"` and the original failure evidence. Without explicit
transport cancellation, a monitor becomes `failed` once failed work is quiet,
or `completed` once verified work is quiet.

The bounded laboratory drain completes before
cancellation intent is forwarded to the upstream store. Both ordered phases
are retained, but a separate client poll between request and finalization is
not guaranteed. A failed drain refuses transport cancellation and retains the
original work and an explicit reconciliation diagnostic instead of claiming
quiescence. The supervisor and stdio server share a caller-owned Asupersync lifetime; no game
worker is detached.

Transport-owned cancellation changes the engine outside the ordinary tool-call
replay stream. Until that event is representable in the replay format, a
session that has executed such a drain marks its exported call log
`replayable: false` with a specific explanation, and the replayer refuses to
execute it. It must not claim that a call log omitting cancellation effects can
reproduce the session.

## Capacity and retention

The first implementation admits **one active task monitor per server process**
and retains at most **256 task records**. The upstream runner owns one active
supervisor handoff at a time, and this bound is enforced before dispatch so a
second accepted task cannot starve behind an unfinished monitor. Ordinary
fortress tools can still operate on multiple plans and sessions.

Task admission accepts at most 64 original actions and requires at least
1,500 output tokens and 6,000 bytes so a complete bounded summary can be
returned. The ordinary conservative session defaults satisfy these bounds.

Tasks have process-lifetime retention (`ttlMs: null`), bounded by the record
limit. A time-to-live must not silently erase an unfinished game obligation.
Once the retained-record limit is reached, new task creation is refused before
effects; retained evidence remains readable. Larger concurrent supervision,
restart-persistent task identities and a replay event for transport-owned
drains remain future work, as do independently observable asynchronous drain
stages for adapters that cannot drain in one bounded call.

## Regression entry points

```bash
cargo test --locked --offline -p dfmcp-mcp --test tasks_tests
cargo test --locked --offline -p dwarf-fortress-mcp --test modern_tasks_golden
```

The process tests use the real `serve` binary and explicit laboratory game
ticks, with bounded response waits and owned reader cleanup. They cover
negotiation, task discovery, original-plan progress, completion evidence,
idempotent effects, capacity refusal, cancellation and deadline failure. Test
results qualify only their exercised laboratory behavior; they do not replace
the repository's full qualification gates.
