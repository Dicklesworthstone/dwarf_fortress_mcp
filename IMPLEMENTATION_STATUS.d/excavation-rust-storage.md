# Excavation Rust storage/workflow: source present, not Rust-qualified

The 1.18 Rust coordinator now has a concrete Linux x86_64/aarch64 private-file
backend and concrete foreground start/recover entry points. The previously missing
concrete TCP/source and storage seams are implemented in source. Runtime Cx region,
global clock lease, MCP/session policy, real DFHack/live and admission remain absent.

This increment adds 17 Linux backend/workflow tests and one joined real-file/TCP
integration test. All 34 Rust tests added with the transport work are UNCOMPILED
AND UNEXECUTED. No Rust formatting, Clippy or compile success is claimed.
Six independent Python request vectors and eight Linux-primitive experiments pass;
these do not execute Rust, establish physical durability, or qualify a live tuple.

No dependencies, native/Python protocols, compatibility entries or production
runner changes. Beads df-dfhack-bridge-plane-c-pic.4/.5 and
 df-action-coordinator-exec-ero.4 remain open. See EXCAVATION_RUN_RUST_STORAGE.md.
