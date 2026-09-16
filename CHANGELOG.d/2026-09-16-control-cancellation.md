# Durable cancellation of prepared pause effects

## Implemented

- `fortress.cancel` in the existing control/1.7 runtime, scoped to one exact
  prepared idempotency key and plan digest. No native RPC or compensating pause
  is dispatched; an already-open writable session need not reconnect to cancel.
- A persistent `CancelledBeforeDispatch` state, tag 6, reachable only from
  Prepared. It retires the key under its original immutable preparation identity
  without fabricating native non-application or a terminal effect receipt.
- Complete cancellation-response rendering before append, successful sync before
  publication/acknowledgement, write/sync fencing, and exact record replay without
  rewinding a newer journal head. Whole Agent Turn bytes count toward the budget.
- Commit and repreparation cannot reactivate cancelled keys. Started or
  indeterminate attempts remain reconciliation-required; verified outcomes are
  not undone. Existing session locking serializes cancellation and commit-start.
- State-filtered discovery, separate cancellation counts, stored explanations and
  query-only reconciliation skips across writable restart and offline recovery.
  Read-only descriptors remain unable to cancel even with injected clock grants.
- Authority/custody rechecks before cached public journal mutation returns, plus
  rejection of absent pause evidence carrying a nonzero raw observation tick.
- Updated live-control guide and `docs/CONTROL_CANCELLATION.md` covering scope,
  failure/recovery, reference vectors and downgrade restrictions.

Cancellation applies only to this journal's dispatch path. It does not prove
current pause state, exclude other independent controllers, release native prepare
retention, close a session or provide a writable offline bootstrap. Repeated
cancellation is not journal compaction; the retired key remains retained.

Existing state tags, framing and digest domains are unchanged. Older readers
reject the new complete cancellation state, not reinterpret it or repair it away.
No dependency, native protocol, mutation family, production runner, compatibility
registry or admission state changed. The formal unadmitted phase remains unchanged.
This fragment records the implementation and evidence status of this increment.

## Validation

Twenty-one new Rust test functions are registered: ten adapter cancellation and
fault cases, two public binary-vector tests, eight actual handler scenarios and
one effect-discovery test. Existing recovery cases remain and gain cancellation
refusal coverage. Tests include partial writes, uncertain sync, response refusal,
replay, stale custody, retired keys, competing commit-start and cancellation,
no-connection handlers, readonly recovery and metadata/head preservation.

**None has been compiled or executed here.** Rust, Cargo and rustfmt are unavailable.
No Rust, MCP-runtime, native, live-game, filesystem crash or full-repository
qualification is established.

The independent Python framing reference executed successfully: 564 single-byte
corruptions, 562 incomplete prefixes, 43 correctly hashed contradictory outcome
mutations and four additional invalid/legacy cases were rejected by that model.
Its 564-byte Prepared/Cancelled journal SHA-256 is
`25f115661f385a23817e59069ac9c7317bc53002244af4671a4ef27e39c46c9d`.
New Rust regressions assert this golden vector but remain unexecuted.

The executed script SHA-256 is
`9f5f194590348d8552767857001ac7018624c4e95e5f375fdabe9e2de884b0f2`.
Its Git blob matches the committed source. This narrowly scoped Python model does
not execute the actual Rust parser, cancellation state machine, renderer, native
bridge or real storage, and does not establish runtime or crash durability.
