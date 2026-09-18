## Native pause dispatch fencing and failure confinement

- Fence competing prepared pause/resume effects before the setter, including
  no-op, failed and ambiguous attempts; reject same-tick stale/ABA preparations.
- Bound native preparation lifetime to 60 monotonic seconds without renewing
  idempotent replays; revalidate pause state and clock and retire stale guards.
- Invalidate native preparation/receipt records on map as well as world changes.
- Confine setter/readback exceptions without fabricated terminal receipts or
  redispatch, and enforce the existing query-only restriction on mutations.
- Execute the actual C++ handlers with explicit native/protobuf doubles: ten
  grouped scenarios pass on GCC/Clang with UBSan and on optimized GCC; removed-
  gate mutants fail as expected. The original source reproduces stale dispatch.
  No Rust, real SDK/plugin, protobuf-runtime, live or admission claim is made.
