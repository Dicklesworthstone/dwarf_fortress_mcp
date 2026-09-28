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

The furniture MCP startup now uses the checked handoff API with the actual joined
request's runtime boundary and current operator opt-in. Cancellation, inherited
runtime restriction and configuration changes can stop CPU-bound allocation
before furniture bootstrap or placement-custody creation. Existing checks around
native work, durable publication and disclosure remain intact.

Nine adapter Rust methods cover every reachable checkpoint for feasible and
infeasible fixtures, cancellation/revocation/budget errors, exact result/work
parity, non-renewed budgets and the work quantum. Two more MCP Rust methods test
the real joined runtime owner and inherited restriction at interior allocation
checkpoints on 512-item fixtures. All eleven methods are source present but
uncompiled and unexecuted in this environment. The container has no Rust
toolchain and cannot resolve GitHub for a local checkout. No native, live,
MCP process or full-workspace qualification is claimed.

See `docs/FURNITURE_ALLOCATION_CANCELLATION.md` for the exact scope and remaining
validation. Existing handoff/completion process suites still require execution.
