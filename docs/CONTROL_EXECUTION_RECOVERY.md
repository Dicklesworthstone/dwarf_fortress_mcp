# Control session recovery and one-shot pause execution

The control/1.7 development runtime now supports explicit session closure without restarting the
server and routes pause commits through a separately testable one-shot execution coordinator.
These are implemented source paths, not qualified production features. Rust compilation and test
execution have not been established for this increment.

The existing native protocol, journal framing, state tags, receipt domains, dependencies and
production admission boundaries are unchanged. Pause remains the only supported live mutation
family. Observation profiles receive no new game-effect authority.

## Release a failed or finished session without forgetting effects

Call the existing `fortress.cancel` tool with an explicit session scope:

```json
{
  "session_id": "<control or control-recovery session>",
  "scope": "session"
}
```

No effect key or plan digest may accompany session closure. Optional `max_bytes` and
`max_output_tokens` can only narrow the session's response allowance. An unknown scope is invalid.
An omitted scope with an exact key and plan digest retains the existing prepared-effect
cancellation behavior; `scope: "effect"` makes that choice explicit. An empty request does not
implicitly cancel every effect or close the session.

Session closure and effect cancellation are different operations:

| Operation | Resources | Prepared effects | Started or indeterminate attempts |
|---|---|---|---|
| `scope: "session"` | Releases connection, journal lock and session capacity | Remain prepared | Remain reconciliation-required |
| Exact-key effect cancellation | Keeps the session open | Retires the selected key if still prepared | Refuses cancellation |

Close performs no native RPC, journal append, sync, repair, truncation or deletion. It does not
release the bridge's native preparation cache, change pause state, compensate an effect or
establish any native outcome. It clears this process's session ownership, not the durable record.

The opening response includes a complete `session_close` request so an agent can discover this
operation without retaining external instructions. Opening itself renders and checks its complete
response before publishing the new session; a failed acknowledgement reservation releases the
unpublished session's connection, journal and capacity.

### Foreground drain and stale handles

A close obtains the session mutex, so a foreground prepare, commit or reconciliation already in
progress finishes or returns before ownership is released. The entire session value is then taken
from its shared handle. Calls that resolved that handle before closure still see an empty slot and
fail before executing their operation. Keeping an old reference alive cannot retain the exclusive
journal lock or the single control-session capacity permit.

Connection and journal destruction precede permit release. The session registry is removed only
for the same exact shared handle; no stale handle selects a replacement session. Close response
rendering, output validation and registry acquisition precede destructive release. An inadequate
response budget leaves ownership intact.

The process retains the last 32 completed close acknowledgements in completion order. A repeated
close can return the exact retained response, subject to output limits. Replaying a receipt does
not affect a newly opened session or refresh the old receipt's retention position. After eviction
or process restart, that response is unavailable; no persistent close receipt is claimed.

### Teardown does not need evidence-reading authority

Close can release an owned session after its grants expire or are revoked, its request sequence is
exhausted, its journal is fenced, or a panic poisons that individual session mutex. This exceptional
path only drops resources. It does not resume a poisoned operation or expose cached game facts,
effect keys, journal counts or a reconstructed outcome. Global registry corruption is still a
refusal, not an excuse to guess ownership.

The close response explicitly reports that preparations were not cancelled, no reconciliation was
performed, current freshness is unproved, and same-effect retry is not safe. The number of prior
unresolved effects is null rather than inferred from inaccessible or unverified evidence.

### Reopen and rediscover

After closure, call `fortress.open_session` again. It uses a fresh session identity and the normal
operator configuration, custody checks and authority rules. `recovery_only: true` opens stored
Query-only evidence without a bridge; live opening still requires the normal authenticated native
handshake. Closing does not upgrade an offline session into a writable one.

Use `fortress.query` to rediscover retained effects. A Prepared record remains eligible only under
its existing exact identity and generation checks; CancelledBeforeDispatch remains retired;
CommitStarted and Indeterminate remain unresolved. Use `fortress.wait` or `fortress.explain` in a
live session for read-only native reconciliation. Never retry a mutating commit to learn whether
its previous attempt succeeded.

A damaged journal releases its lock when closed but remains damaged. Its next owner must pass the
existing verified replay policy. Close is neither repair nor a substitute for recovering evidence.

## Commit preflight before any mutation

The existing `fortress.commit` request shape is unchanged:

```json
{
  "session_id": "<live control session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same 64 lowercase hex characters returned during preparation>",
  "prepare_token_hex": "<same 32 lowercase hex characters from the durable preparation>"
}
```

The actual MCP handler delegates to `control_commit.rs`, which reserves the complete possible
outcome packet, including the Agent Turn and journal metadata, before invoking the adapter's
`commit_once` coordinator. Reservation retains the exact key and immutable preparation fields and
uses worst-case widths for mutable numbers, nullable observations, receipts and Boolean spellings.
A too-small positive response allowance fails before commit intent or source invocation. An
invalid zero budget dimension keeps the core `InvalidRequest` contract. Cancellation, authority
and custody checks still apply to cached terminal outcomes.

The coordinator enforces this order:

```text
current authority, budget, custody and exact preparation identity
→ complete MCP outcome-response reservation
→ local connection readiness and generation preflight
→ remaining-deadline check
→ append and sync CommitStarted
→ remaining-deadline and authority check
→ at most one CommitPause source invocation
→ independently verify native receipt and observed outcome
→ append and sync terminal evidence, or retain an unresolved attempt
→ bounded outcome response
```

The mutation-source trait has only local preflight, one commit method and a fence method. It does
not inherit the read-only recovery transport. The runtime implementation uses the already-open
fixed control RPC client; mutation execution never reconnects or automatically retries.

Response rendering and durable-intent work consume the same foreground allowance. The RPC receives
only the remaining time after intent sync, not a fresh full timeout. These are cooperative deadline
checks: synchronous filesystem operations and session-lock acquisition do not gain a hard
preemption or cancellation guarantee.

If a valid native response has already arrived, the coordinator attempts to persist its verified
evidence even when wall time expired while the RPC completed. Discarding that evidence would make
recovery less informative. Current authority and journal custody are still checked before writing.

### Failure classification

A failure before CommitStarted is synced does not invoke the mutation source. An uncertain intent
sync may nevertheless leave a complete CommitStarted frame on disk; verified reopening conservatively
recovers that frame as an unresolved attempt, not as permission to dispatch again.

If the remaining deadline ends after durable intent, the source is not invoked. The coordinator
attempts to annotate the attempt as indeterminate and refuses same-effect retry. It does not undo
the durable intent merely because this invocation knows it did not finish dispatch.

Lost replies, contradictory receipt evidence and failed terminal durability fence the source and
return `EffectIndeterminate`. A best-effort indeterminate annotation never overrides a fenced
journal or becomes an acknowledgement of successful persistence. Complete frames written before
an uncertain sync may be recovered; incomplete frames still fail the existing replay policy.

A merely known native key without a complete terminal receipt remains unresolved. Unknown or
lost-generation evidence is not fabricated as VerifiedNotApplied. Cancelled keys cannot dispatch,
and repeated commits for an unresolved key do not call the source. Identical retained verified
outcomes may be replayed without a connection, but still require the current valid context and
writable-session contract.

Successful result payloads distinguish `replayed_terminal`, `mutation_dispatched`,
`reconciliation_required`, and `current_freshness_proven`. Here `mutation_dispatched` records entry
into the source commit call; it is not proof of native application. Error payloads retain the
existing conservative unknown dispatch marker for indeterminate failures. Historical applied
receipts never establish the current game pause state.

## Implementation and evidence status

Twenty-three new Rust test functions are registered:

- Eight session-release tests cover state preservation, real private-file locks, bounded close
  receipts, stale handles, concurrent closers, foreground drain, expired/revoked authority, request
  exhaustion, poisoned ownership, damaged storage and cross-session isolation.
- Nine adapter execution tests inject failures around intent and terminal sync, partial writes,
  lost and contradictory replies, generation mismatch, invalid authority/budgets, and deterministic
  deadline crossings. A source-side assertion replays stored bytes to verify CommitStarted was
  synced before the source commit call.
- Five actual handler tests cover output refusal before connection use, failed opening publication,
  pre-resolved stale handles, and real framed loopback RPC followed by close/reopen under valid or
  lost replies. One checks that closing a connected session sends no native method at all.
- One complete-packet reservation test checks exact accepted byte ceilings across escaped and
  multi-byte keys, mutable numeric extremes and terminal/unresolved response shapes.

Existing cancellation and offline-recovery tests remain registered and were adapted to consumable
session ownership. No prior test family was removed.

**None of these Rust tests has been compiled or executed in this editing environment.** Rust,
Cargo and rustfmt are absent, and the local environment cannot reach the toolchain/repository
hosts. No passing build, TCP test execution, DFHack campaign, crash/power-loss qualification,
whole-repository verification or admission is claimed. The loopback tests are a protocol fixture,
not the native producer or a real game. This increment was checked by source and diff review plus
GitHub commit verification only.

Focused commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-adapter pause_reconciliation::execution
cargo test --locked -p dfmcp-mcp live_control_server -- --test-threads=1
```

These commands do not replace full repository qualification. No new mutation family should be
inferred from improved coordination of the existing pause-only development path.
