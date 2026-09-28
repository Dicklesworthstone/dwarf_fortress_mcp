# Interruptible inventory-to-placement planning

Add `furniture_supply::plan_with_check` and `Handoff::allocate_with_check` so a
foreground owner can stop work during inventory scanning and global matching,
not only at the surrounding native I/O boundaries. Preserve the existing pure
APIs, item-selection objective, canonical bytes, source evidence and work counts.

The extra check runs before source work, across phase boundaries, at most every
256 charged work units in scans/matching, and before complete result publication.
Callback time consumes the same deadline. A refused callback discards the entire
assignment or shortage; it cannot grant Query or placement authority. Reject a
handoff entity allowance before hashing the full graph.

Nine Rust regression methods cover every reachable checkpoint for feasible and
infeasible fixtures, cancellation/revocation/budget errors, exact result/work
parity, non-renewed budgets and the work quantum. These methods are source
present but uncompiled and unexecuted in this environment. The container has no
Rust toolchain and cannot resolve GitHub for a local checkout. No native, live,
MCP process or full-workspace qualification is claimed. MCP owner wiring is a
separate increment.
