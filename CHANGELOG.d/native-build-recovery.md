## Recover exact original furniture placement evidence

- Add read-only `Engine::reconcile` for retained indeterminate furniture attempts.
- Share commit's double-capture and exact construction-job/item proof, preserving
  original keys and byte-identical terminal receipts without redispatch.
- Keep uncertainty through all failures, source changes and field mismatches;
  release it only after the complete original expected state is proved.
- Executed GCC/Clang component evidence: 12 scenarios, 1,985 assertions and three
  killed compiled mutants per compiler. Native callbacks are injected; no real
  DFHack, protobuf runtime, Rust, live-game or admission qualification is claimed.
