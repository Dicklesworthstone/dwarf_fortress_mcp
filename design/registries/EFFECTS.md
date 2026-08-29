# Effect registry

All nondeterminism and external state crosses a named effect boundary. Pure planning and transition
logic may consume recorded effect results but may not perform effects directly.

| Effect ID | Boundary | Deterministic input record | Required fault modes | Recovery discipline |
|---|---|---|---|---|
| FX-001 | wall/monotonic/game clocks | clock domain, requested instant/delta | jump, stall, rollback, overflow | domain-specific monotonic checks |
| FX-002 | randomness | stream ID, seed, draw index | exhaustion, replay mismatch | seeded transcript equality |
| FX-003 | scheduler choice | runnable set, chosen task | delay, reordering, starvation | schedule trace/DPOR exploration |
| FX-004 | DFHack bridge connect/handshake | endpoint, versions, nonce, feature request | refusal, timeout, partial frame, downgrade | fail closed or read-only degradation |
| FX-005 | bridge observation batch | groups, bounds, limits, base anchor | truncation, duplication, omission, corruption, reset | validate, continue, or epoch reset |
| FX-006 | bridge mutation prepare | plan digest, actions, scopes, fencing | rejection, stale state, unsupported action | no game effect; amend/replan |
| FX-007 | bridge mutation commit | operation/idempotency key, prepare token | dropped request, dropped receipt, duplicate, partial batch | operation lookup and reconciliation |
| FX-008 | bridge cancellation | action/operation key | too late, partial stop, timeout | drain and prove terminal state |
| FX-009 | ledger transaction | transaction kind, prior sequence, frame digest | I/O error, torn write, duplicate commit | WAL recovery and checksummed scan |
| FX-010 | checkpoint filesystem | source save, destination, manifest | partial copy, rename failure, bit rot, out-of-space | seal, verify, atomic publish |
| FX-011 | restore filesystem | checkpoint manifest, target slot | partial restore, stale process view | offline/paused protocol and new epoch |
| FX-012 | search/index update | canonical source anchor, index version | lag, corruption, nondeterministic score | rebuildable projection, never authority |
| FX-013 | knowledge import | source bytes, parser version | malformed input, prompt injection, encoding bombs | tainted non-authoritative corpus |
| FX-014 | MCP transport | negotiated version, request ID, envelope digest | disconnect, duplicate, out-of-order, oversize | idempotent request ledger and limits |
| FX-015 | telemetry export | redacted event envelope | sink unavailable, backpressure, leakage | bounded buffers and fail-safe redaction |

## Effect result algebra

Every effect returns one of:

- `ok(value, evidence)`;
- `error(code, retryability, evidence)`;
- `cancelled(reason, evidence)`;
- `indeterminate(reconciliation_key, evidence)`.

`indeterminate` is not an error alias. It means repeating the effect could duplicate a real-world
mutation and therefore requires lookup, observation, or human policy before retry.
