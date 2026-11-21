# Release a read-only live session

The protocol-1.0 citizen server, protocol-1.1 announcement development server
and protocol-1.2 jobs development server can release a finished or unusable session through the existing cancellation
tool. This returns capacity to the running server without restarting it or
changing the game.

| Profile | Concurrent session ceiling | Release request |
| --- | --- | --- |
| Citizens / protocol 1.0 | 32 registered sessions | `fortress_cancel` with `scope: "session"` |
| Announcements / protocol 1.1 | 32 registered sessions | `fortress_cancel` with `scope: "session"` |
| Jobs / protocol 1.2 | 16 sessions, including in-flight bootstrap | `fortress_cancel` with `scope: "session"` |

The operations/1.3 and paged operations/1.4 servers already expose the same
resource-release selectors; see [operations history](OPERATIONS_HISTORY.md).
The laboratory's separate `mode: "close_session"` contract also accounts for
owned game work and Task monitors; it is not the read-only live contract.

## Request and retained work

Pass the exact session identifier returned by this process:

```json
{
  "session_id": "<exact session identifier>",
  "scope": "session"
}
```

Omit `mode` and the legacy `_mode` selector. A non-null mode, an unsupported
scope, or a discard selector without `scope: "session"` refuses the close.
An ordinary cancellation call without a session scope keeps its read-only
refusal and does not release the session.

If this session owns retained query baselines or watches, the initial close
refuses without discarding them. To consent explicitly, use:

```json
{
  "session_id": "<exact session identifier>",
  "scope": "session",
  "discard_process_local_work": true
}
```

The release clears only this session's query resources. It does not sample a
watch, advance an announcement cursor, create a historical observation, write
a journal or interpret missing jobs/reports as successful completion. The
citizen-only server has no retained structured-query baselines or watches.

## Release semantics

A valid process-session handle establishes ownership of these process-local
resources. Closing does not require a fresh game observation, an unexpired
read grant or another semantic request identifier. An expired or revoked grant,
a fenced connection, an exhausted request counter or a poisoned session mutex
therefore does not prevent cleanup. Ordinary reads retain their normal guards;
only teardown may recover the poisoned session guard, and it never clears that
poison for later use. Damaged global registry or receipt custody still refuses.

The complete Agent Turn acknowledgement must fit the session's existing byte
and output-token limits before any cleanup. Missing discard consent, insufficient
output space or a changed registry owner leaves the session and retained work
intact. A request already using the session finishes before teardown acquires
its mutex; close does not interrupt that request or start another native read.

Successful teardown removes the session from its registry and drops its native
adapter/connection, credentials and cached observations immediately. Holding an
old internal `Arc` does not retain the socket or usable session state. A waiter
that acquired the handle before close must pass the closed-state check before
reading cached or native data. Other sessions remain open.

The response reports only process-resource release: no native observation,
game-effect cancellation, mutation dispatch or current game-state anchor. It
makes no claim that physical fortress work stopped or a gameplay goal succeeded.
Development profiles retain their explicit unadmitted posture; closing a
protocol-1.0 session makes no new process-admission assertion.

## Repeats and reopening

Each server retains its latest 32 successful close responses. Repeating an
eligible close for one of those exact handles returns the same response bytes
without releasing resources again. The bound is independent of the session
capacity. After eviction, the old handle returns `session_not_found`; it never
resurrects a session or aliases a new owner.

Reopening uses the normal open-session path, including authentication, current
configuration, fresh bootstrap and a new process-session identifier. A close
receipt cannot restore grants, native observations, baselines or watch handles.
Receipts are process-local and do not survive server restart.

## Validation boundary

The generic lifecycle tests exercise atomic refusal, exact ownership, bounded
repeat receipts, poisoned session cleanup, concurrent closes and immediate
resource destruction with retained handles. Profile tests cover their actual
selectors, retained-query ownership and public entry points. Executed evidence
and outstanding integrated/native qualification are recorded separately in
[implementation status](../IMPLEMENTATION_STATUS.md). These changes do not add a
native method or widen the production protocol map or compatibility registry.

Owning bead: `dfmcp-live-read-session-release-r4s`.
