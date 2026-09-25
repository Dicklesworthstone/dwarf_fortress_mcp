# Excavation-run Rust coordinator: source present, not qualified

Beads df-dfhack-bridge-plane-c-pic.4/.5 and df-action-coordinator-exec-ero.4.

An adapter coordinator now enforces local durable dispatch ordering, exact plans,
source bindings, whole-journal pending fences and query/cancel-only recovery.
The native source and private storage are injected contracts, not newly supplied
or qualified implementations. No production path or native protocol changed.

Executed: seven independent Python byte fixtures and exact uploaded-blob checks.
Added Rust tests: 9 evidence + 13 coordinator. ALL UNCOMPILED AND UNEXECUTED;
Cargo invocation fails because the executable is absent. No Rust-qualified,
real-DFHack, live-game, Cx/lease/MCP or full-repository qualification is claimed.
See docs/EXCAVATION_RUN_RUST.md for backend obligations and remaining integration.
