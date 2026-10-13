# Original placement reconciliation

Add explicit full-receipt Indeterminate-to-Placed validation and an append-only
Python journal recovery frame. Preserve the original uncertainty, exact source,
original key and no-retry rules. Fourteen actual Python codec/private-store tests
pass; native RPC/CLI integration and broader qualification remain separate.
See docs/BUILD_PLACEMENT_CLIENT_RECOVERY.md; bead df-dfhack-bridge-plane-c-pic.4.

## Query integration

Connect the standalone and route-batch query paths to exact native receipt
recovery. Unknown/absent outcomes remain pending, cancellation does not erase an
attempt, and resolved receipts stay offline. Preserve current authority through
journal publication and one CLI output attempt. Native RPC support from bb13666
is preserved, not reimplemented. All 31 focused Python tests pass (14 codec/store,
14 runtime, 3 injected-parent route-handler cases); no live/native/Rust/full-suite
qualification. Legacy non-route batch wrapper and Rust journal evolution remain
outside this increment; standalone recovery works on original child journals.
