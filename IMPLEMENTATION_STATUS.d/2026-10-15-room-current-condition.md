# Fresh whole-room condition inspection

Owning beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (remain in progress).
`room_readiness_current.inspect` evaluates every original furnishing and all
terrain/occupancy requirements, plus the retained strict circulation requirement,
without advancing, renewing or clearing the original monitor. It reuses complete
Goal/Sample validation and the existing single-receipt construction evaluator.
Historical source, horizon, clock and building floors remain explicit invalidators.
Missing evidence is unknown; a successful subset is never the complete goal.

Executed: 15 tests of the complete new module with explicitly injected validated
native producers, including all 27 three-valued combinations, every query guard
interruption, identity/source regressions, all terminal phases and 32 targets.
These do not execute the upstream codecs, real journal, TCP, Rust/MCP or DFHack.
The public native-codec boundary and CLI integration require separate validation.
No native protocol, mutation, production admission or journal format is changed.
