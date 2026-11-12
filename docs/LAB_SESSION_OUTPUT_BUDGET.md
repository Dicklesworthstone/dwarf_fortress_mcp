# Recover an oversized review in the same laboratory session

A later complete plan review can need more response space than the session's
opening allowance. The laboratory accepts an explicit output-only negotiation
through the existing `fortress_open_session` tool:

```json
{
  "session_id": "<the existing 32-character session identifier>",
  "max_output_tokens": 8192
}
```

Supply only those two arguments. Omit `session_id` for ordinary new-session
opening. The old direct Rust opening API remains available; integrations that
need the alternative use `agent_facade::fortress_open_session_with_session`.
The closed alternative is described by
[`lab_session_output_budget.schema.json`](../schemas/lab_session_output_budget.schema.json).

After an accepted change, repeat the original `fortress.plan` request, inspect
its complete new review, and commit its exact seal separately. For a retained
room project, repeat the returned `continue_goal` request with the same root.
The negotiation itself does not prepare a replacement, commit work, advance the
clock, or change the world. It preserves the existing private session and all
observed inputs, so opening a fresh fortress is unnecessary.

## What the receipt means

The response uses `dfmcp.session-output-budget/1`, with
`kind: "output_budget_renegotiation"`. It reports the same session and current
anchor, the previous and admitted six-dimensional budgets, the actual response
byte ceiling, and an Agent Turn carrying current pending work, action and
obligation state, original goals, query baselines/watches and Task discovery
coverage. This is an orientation of an existing owner, not a bootstrap or an
epoch reset. It claims no new game observation or proof.

The server serializes and admits that complete acknowledgement under the
proposed response limit before changing the stored output allowance. A refusal
leaves the old allowance and work intact. This also applies to narrowing: a
requested smaller limit is accepted only if the complete acknowledgement fits.
Repeated negotiation to the current value is allowed and reports `changed:
false`.

## Authority and limits

Current whole-fortress `Observe` and `Query` grants at read-only risk are
required because the acknowledgement includes current world and work evidence.
Durable owner fencing remains in force. An old owner cannot use this operation
to recover authority after another session reopens its fortress.

`max_output_tokens` is an explicit integer from 1 through 65,536. The complete
response is still bounded by the existing `max_bytes` limit and the
laboratory's ordinary token-to-byte estimator. Raising tokens cannot bypass a
source-byte or entity limit, grant expiry, game-tick allowance, action limit,
wall deadline, or a pending plan's own validity rules. Source admission charges
the complete canonical world together with retained records/steps, original
source text and the full materialized work/catalog projection against the
unchanged entity/byte ceilings. If source admission
fails, requesting more output cannot repair it.

The operation reuses the opening issuer's bounded negotiation boundary. It is
not a delegated operation widening a child context. All existing capability
grants, scopes, risk ceilings, expiries, uses and stored operation contexts stay
unchanged, preserving INV-030. In a shared fortress the change belongs only to
the named session. Other members keep their own budgets. Previously retained
Tasks keep their original work/deadline contracts; their presentation follows
the current session response allowance, as it does for other current reads.

This metadata does not become durable fortress state. A fresh session after
restart negotiates its own allowance through ordinary opening. Successful
negotiations and refusals are recorded in the private session's normal replay
log. The portable `existing_session` call marker remaps to the replay's current
session, so replay does not open a new world or reuse the old process-local ID.
Old bundles retain their original opening arguments. Successful transitions
also record the accepted previous and new output allowances. Replay compares
both receipts, so concurrently recorded calls arriving out of owner order
produce an explicit divergence even when their canonical anchors are equal.

## Scope and validation

The eleven tool names, native protocols, compatibility registry and live
admission boundaries are unchanged. This alternative is implemented in the
reference laboratory facade; development live runtimes keep their existing
opening contracts.

Regression sources cover owner/authority fencing, unchanged non-output budgets
and work, complete-publication refusal, shared-session isolation, portable
replay and the actual oversized two-bedroom continuation: harvest observed
wood, refuse its default-sized complete review, explicitly raise the allowance,
then review, commit and complete the same original room. Current execution
results and full-runtime limitations are recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md).
