### Add typed sampled floor-goal evaluation for mining progress integration

Reuse the existing Rust map/1.5 observation to evaluate exact visible dry-floor
conditions, advancing-tick stability, fixed deadlines, interruption/gap resets
and absorbing terminal outcomes. Source drift invalidates rather than rebinding.
Keep goal evidence independent of native designation-effect reconciliation.

Ten Rust tests are registered but uncompiled/unexecuted; no Rust toolchain is
available. Source/test blob identities match local reviewed bytes. No dependency,
native protocol, mutation authority or production admission is changed.
See docs/RUST_EXCAVATION_GOALS.md.
