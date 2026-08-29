# Error registry

Error identifiers are stable wire values. Messages may improve; meanings and retry classes do not
change incompatibly inside `dfmcp/0`.

| Code | Default retry class | Meaning | Required caller action |
|---|---|---|---|
| `version_mismatch` | after negotiation change | Protocol/schema versions have no safe overlap. | renegotiate or upgrade |
| `session_not_found` | no | Session is unknown or terminal. | open a session |
| `fortress_not_loaded` | after state change | No supported fortress is loaded. | load/observe game state |
| `adapter_unavailable` | bounded backoff | Bridge is unreachable or not ready. | health-check and reconnect |
| `cursor_gap` | no direct retry | Requested delta continuity cannot be proved. | request full snapshot/new continuation |
| `stale_anchor` | replan | Request was anchored to prior state. | observe and recompile |
| `invalid_request` | no | Envelope/schema/bounds are invalid. | correct request |
| `invalid_intent` | no | Goal or constraints are semantically invalid. | revise intent |
| `invalid_plan` | no | Plan violates registry/state-machine rules. | recompile or fix planner |
| `capability_denied` | after authority change | Required capability/scope is absent. | obtain narrower valid authority |
| `risk_ceiling_exceeded` | after policy change | Classified risk exceeds grant ceiling. | raise policy explicitly or revise plan |
| `budget_exceeded` | with revised budget | A work dimension was exhausted. | narrow request or authorize more budget |
| `preconditions_failed` | replan | Semantic precondition is false or unknown. | observe and revise |
| `conflict` | after competing work changes | Write scopes overlap without safe commutativity. | serialize, lease, or replan |
| `lease_denied` | after lease state change | Required lease/fencing token is unavailable/stale. | wait or acquire current lease |
| `checkpoint_required` | after checkpoint | Policy requires durable checkpoint evidence. | checkpoint then prepare again |
| `adapter_rejected` | request-specific | Bridge safely rejected before effect. | inspect details and revise |
| `adapter_failure` | bounded/reconcile | Bridge failed and effect status is known not applied. | retry only if marked safe |
| `effect_indeterminate` | never automatic | Effect may or may not have occurred. | reconcile by operation key and observation |
| `verification_timeout` | observe/reconcile | Dispatch occurred but postcondition deadline expired. | inspect obligation; do not blindly retry |
| `cancellation_requested` | no | Operation is draining after cancellation request. | wait for terminal cancellation outcome |
| `cancellation_incomplete` | reconcile | Stop/drain could not prove a safe terminal state. | inspect/reconcile/compensate |
| `restore_required` | no | Safe continuation requires restore or explicit abandonment. | invoke authorized restore policy |
| `corrupt_ledger` | offline repair only | Durable state failed integrity checks. | seal and run doctor/repair workflow |
| `compatibility_unknown` | after probe/upgrade | Required DF/DFHack semantics are unverified. | run probes or stay read-only |
| `internal_invariant_violation` | no automatic retry | A supposedly impossible state was observed. | stop mutation, preserve bundle, investigate |

## Retry law

Retryability is attached to the concrete error occurrence, not inferred solely from the code.
Mutating calls additionally require an idempotency/reconciliation decision. A transport timeout can
be retryable for `observe` while the same symptom after `commit` is `effect_indeterminate`.
