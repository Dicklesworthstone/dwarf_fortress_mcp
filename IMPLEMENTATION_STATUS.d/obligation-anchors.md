# Anchored obligations: source present, not qualified

Work item: `df-action-coordinator-exec-ero.4` (remains open).

The existing ObligationRuntime now binds each action to the fortress, observation
epoch, sequence, tick and digest of every accepted snapshot, independently of
positive sampling cadence. Changed lineage, cursor forks and either regressing
clock are rejected in whole-batch preflight. Cancellation cannot precede an
accepted off-cadence read. Terminal records keep their historical anchor.

`register_obligation_at` binds the creation snapshot before the first sample;
the existing tick-only registration remains available but cannot validate source
identity until its first accepted observation. `last_observation_anchor` exposes
this distinction. `observation_interrupted` clears only an unfinished stability
streak, without changing the fixed deadline, cadence, source or terminal history.
The caller must report acquisition interruptions through that method. These are
sampled endpoints, not proof of continuous state between observations. A cursor
sequence jump is allowed for complete snapshots, not treated as a delta or proof
that intervening observations were retained. Epoch transitions require separate
reconciliation/new registration, not rebinding the old obligation.

Ten additional Rust regression tests are registered, UNCOMPILED AND UNEXECUTED.
This environment has no Rust/Cargo/rustfmt or complete dependency checkout. The
command to run in a qualified checkout is `cargo test --locked --offline -p dfmcp-intent`.
No new MCP route, durable journal, native protocol, dependency, region ownership,
compensation execution, live-game qualification or production admission is added.
