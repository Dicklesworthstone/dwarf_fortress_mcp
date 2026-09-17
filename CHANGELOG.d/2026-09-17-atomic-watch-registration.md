# Atomic monitoring-set registration

## Implemented source

- Add `register_watches` to the existing live spatial/1.8 fortress.query path.
  Install one to eight related monitor definitions from one current capture,
  rather than a sequence that can leave a partially configured monitoring plan.
- Parse members through the existing single-watch request and validators. Derive
  their live discovery schema from the same watch variant after population and
  item-quantity conditions have been composed. Unknown fields and duplicate keys
  refuse the complete request.
- Canonically order definitions, preflight every identity/deadline and aggregate
  retention limit, then evaluate all new monitors with one shared work allowance.
  Preserve existing selected and unselected records without resampling them.
- Render the complete Agent Turn and active work before one existing-format watch
  checkpoint and one root publication. Definition conflicts, late quantity
  overflow, work exhaustion and output rejection cannot publish a subset or burn
  watch identities. Existing journal sync-failure fencing remains in force.
- Exact key/definition retries return retained status and handles with no extra
  sampling, checkpoint, deadline renewal or terminal-state reactivation. Mixed
  sets reuse matching records and install missing members together. Reordering
  members or spelling out existing defaults does not alter their identities.
- Report created/replayed counts, ordered configuration digest and per-key
  evidence. Return a selected await_watches request for unfinished members;
  registration itself performs zero native captures and needs Query, not Observe.
- Recovery uses the existing paired observation/watch journals. Retrying the
  definitions returns fresh recovered handles without undoing stability resets.
  Idempotency ends on explicit key release; no indefinite tombstone is claimed.
- Preserve archive/historical refusal, source-health checks, current authority,
  observation-journal custody, existing retention and whole-request input limits.

Usage: docs/ATOMIC_WATCH_REGISTRATION.md. Native protocols, serialized watch
formats, dependencies, top-level tools, game-effect families and production
admission remain unchanged. This fragment records the source/evidence status of
this increment; the formal unadmitted implementation phase is unchanged.

## Validation

Fourteen Rust test functions are registered: eight engine scenarios and six
actual spatial-handler/private-file scenarios. They cover order/default
normalization, retries, mixed sets, full retention, late failures, quantity
overflow, shared scan exhaustion, output refusal, authority, one-checkpoint
installation, one-capture follow-up, restart recovery and archive restrictions.
Existing batch, quantity and population tests remain registered.

No Rust compilation or test execution occurred here. Rust, Cargo and rustfmt are
absent, and container DNS access to the toolchain host failed. Validation is
source review and GitHub diff/branch verification only. There is no independent
Python mirror offered as a substitute for runtime execution, and no native,
live-game, filesystem-crash, full-repository or admission qualification claim.
