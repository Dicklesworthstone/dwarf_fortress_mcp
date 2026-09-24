# Measured obligation drains: source present, not qualified

Work item: `df-action-coordinator-exec-ero.4` (remains open).

Cancellation has a bounded, retained progress inventory rather than accepting a
single unrecorded terminal count. `request_cancel_with_steps` fixes 0..65536
compensation steps. `record_drain_progress` preserves that total, requires counts
and game ticks to move monotonically, rejects quiescence with outstanding steps,
and retains the latest certificate. Same-tick progress remains legal while paused.
Repeated cancellation cannot change the inventory or erase prior progress.
`get_drain_progress` exposes active and historical terminal progress.

`finalize_cancel` requires the exact latest recorded quiescent certificate with
zero outstanding steps. Only an exact terminal replay is idempotent; changed
final ticks, identities or counts cannot overwrite or masquerade as old evidence.
Rejected requests publish neither progress nor terminal changes.

Caller migration: ordinary `request_cancel` starts a zero-compensation drain,
or repeats an existing drain with its original inventory. Callers with actual
compensation steps must use `request_cancel_with_steps`. All callers must record
their owner's verified progress before finalizing, even for a zero-step drain.
The old integration test now uses a nontrivial terminal predicate (its old True
predicate was already refused by registration), registers its one-step inventory,
and records progress before finalization.

These counters DO NOT prove game effects, own asynchronous tasks, authorize
compensation, stop in-flight RPCs, persist across restart or replace an owner's
postcondition evidence. The cancellation owner remains responsible for verifying
compensation and actual quiescence. No MCP/adapter caller is migrated by this
semantic-core increment. Durable/structured-region integration remains separate.

Twelve new Rust regression tests and the corrected existing lifecycle test are
UNCOMPILED AND UNEXECUTED. Rust/Cargo/rustfmt and locked dependencies are unavailable
here. Run `cargo test --locked --offline -p dfmcp-intent` in the controlled checkout.
No native/live-game, full repository qualification or production admission is claimed.
