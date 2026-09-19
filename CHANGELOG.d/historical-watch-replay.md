### Retrospective foreground-monitor replay

- Add `historical_watch_replay` to actual live and archive-only spatial/1.8 query
  dispatch and schema discovery. Exact endpoint records bound 1..32 consecutive
  captures, reconstructed in one raw/delta prefix pass.
- Reuse the foreground Watch transition implementation for cadence, stability,
  failure/unknown precedence, deadline expiry, generation and epoch invalidation.
  Freeze the first terminal result while still verifying all selected records.
- Keep request-owned replay evidence separate from retained watches and actual
  historical watch activity. No watch handle, native capture, journal append,
  authority or effect is created. Summary/detail share deterministic identities.
- Preserve current authority, acquisition/output bounds, and one shared monitor
  work allowance. Live active work is displayed without sampling; archive-only
  sessions require no Observe grant or watch-journal load.
- Register 11 core and 6 actual-handler Rust tests; uncompiled/unexecuted because
  Rust/Cargo/rustfmt are unavailable. Execute 104 isolated envelope cases and 8
  independent range-boundary checks; these do not execute Rust or the full
  composed predicate schema. No production admission or live qualification.
