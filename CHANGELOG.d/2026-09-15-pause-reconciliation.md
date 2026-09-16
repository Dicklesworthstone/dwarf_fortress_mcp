# Pause-control recovery and terminal-outcome correctness

The explicitly unadmitted control/1.7 slice now has bounded foreground reconciliation through `fortress.wait`, building on offline journal discovery. It does not add a live mutation family, native method, dependency, or production runner.

## Functionality

- Reconcile 1..16 selected durable keys with one shared wall-time allowance, canonical key ordering, complete-selection validation and complete-response reservation before bridge work. Prepared/terminal effects are not dispatched or queried. A failure stops subsequent queries and preserves earlier durable progress.
- Verify new prepare tokens and new live terminal receipts against complete durable identity. A known native prepare or interrupted commit without a terminal receipt remains indeterminate, never proven not-applied. Existing terminal archive records are not silently requalified.
- Keep immutable native prepare identity separate from observed outcome. Same-content prepare replay survives time advancement; setter failure reports the actual observed pause state. Invalid post-set clock evidence or a post-set exception cannot permit a second setter invocation.
- Reject incomplete known-effect replies and incorrect binary lengths. Fence every failed wire call, including oversized frames with unread bytes. TCP connect and native handshake share one timeout.
- Preserve offline Query-only recovery and refusal of live wait, even when clock grants are injected. Retained evidence never claims current game freshness or safe retry of a mutating commit.

## Evidence

Nineteen new Rust scenarios are registered (10 coordinator, seven wire, two response reservation); existing offline MCP-handler tests are extended. They have not compiled or run here because Rust/Cargo/rustfmt are unavailable.

The actual changed native producer passed 75 C++ assertions plus three independent SHA-256 comparisons under each of GCC and Clang, using C++17 and `-Wall -Wextra -Werror -pedantic`, through `scripts/test_live_control_outcomes_native_mock.py`. Producer SHA-256: `80bb0c427ce94ecee41b86c52ea721e979cd15b1f371e0a4a7a27f5aa410a6ff`. Independent Python also verified the Rust test-vector constants.

These are source/mock-interface checks, not a real DFHack/generated-protobuf build, live campaign, Rust qualification, or admission. See `docs/LIVE_CONTROL.md` and `IMPLEMENTATION_STATUS.md` for invocation and exact limitations.
