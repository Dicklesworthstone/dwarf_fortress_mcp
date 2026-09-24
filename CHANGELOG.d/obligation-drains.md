# Measured cancellation progress

- Add bounded compensation inventories, monotone progress registration and current
  progress inspection to the reference obligation coordinator.
- Require exact retained quiescence evidence before finalization; reject forgotten
  work, count overflow, time regression, altered replay and cross-action evidence.
- Tighten caller requirements: register nonzero compensation work explicitly and
  record verified progress before finalization, including zero-work drains.
- Add twelve Rust regression tests and correct the old lifecycle test's already
  invalid True predicate. Tests are uncompiled and unexecuted in this environment.
- Keep game-effect verification, runtime ownership and durability with their owners.
