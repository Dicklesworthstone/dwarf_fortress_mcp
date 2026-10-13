# Furniture placement exact-frontier recovery

The native value engine can recover an original indeterminate furniture placement
through read-only reconciliation. It requires original source/sequence, complete
expected-after capture equality and exact insertion links bracketed by another
capture. Commit and recovery share that proof. No receipt is fabricated for an
absent, changed or unverifiable effect; no writer is reachable from reconciliation.

GCC 14 and Clang 17 execute 12 scenarios / 1,985 assertions and reject all three
compiled mutants with warnings denied and UBSan. This is actual-engine component
evidence with injected boundaries, not real SDK, protobuf, Rust or live execution.
See docs/BUILD_PLACEMENT_RECOVERY.md. This engine increment alone does not wire the
new method into native RPC dispatch. Broader native placement integration and
qualification remain tracked by df-dfhack-bridge-plane-c-pic.4 and .5.
