# Excavation Rust transport — source present, independent fixtures checked

The concrete 1.18 TCP source now implements ExcavationRunSource and consumes the
coordinator's dispatch permit. Recovery does not require a terrain read or regain
commit permission. Cancellation permission is distinct from unpause opt-in.

Six Python request vectors executed and agree with existing native plan/token
commitments. Sixteen Rust tests were added but NOT compiled or executed; Cargo,
rustc, rustfmt and the locked dependency graph are unavailable. This is not a
Rust qualification, real SDK/protobuf test, live campaign or production admission.
Native/Python protocols, compatibility registry and production runner map remain
unchanged. Runtime regions, global clock leases, session policy and MCP wiring
remain outstanding. Beads df-dfhack-bridge-plane-c-pic.4/.5 and
 df-action-coordinator-exec-ero.4 stay open. See docs/EXCAVATION_RUN_RUST_RPC.md.
