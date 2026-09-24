# Obligation evidence: source present, not qualified

Work item: `df-action-coordinator-exec-ero.4` (remains open).

The reference obligation runtime preflights an entire observation before publishing
any action transition. Failure evidence is evaluated on every supplied observation,
including off-cadence and changed same-tick observations. Contrary or unknown terminal
conditions reset the matching streak without postponing the scheduled positive sample.
Duplicate game ticks cannot increase stability. At the deadline, an incomplete
stability count now fails immediately even when the endpoint predicate matches.
Cancellation cannot be backdated before the last scheduled evaluation.

Ten additional Rust regression tests are registered alongside the existing tests.
They are UNCOMPILED AND UNEXECUTED: this environment has no Rust/Cargo/rustfmt or
complete dependency checkout. No Python model is substituted for execution of Rust.
The intended command is `cargo test --locked --offline -p dfmcp-intent`.

This is synchronous in-memory semantic-core work, not durable obligation recovery,
actual compensation, native/game behavior, full qualification or production admission.
No dependency, native wire generation, capability or production runner is changed.
