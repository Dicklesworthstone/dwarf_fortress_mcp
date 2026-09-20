### Recoverable one-shot bounded simulation developer client

Add a POSIX, standard-library-only run/1.13 client with observe/start/query/cancel
and offline intent inspection. A fresh private immutable intent capsule and its
parent directory are fsynced before native prepare/commit. Start attempts commit
once; reconnect recovery queries or cancels, never replays an unpause. Validate
fixed method bindings, framing/deadlines/notification budgets, exact native
observation/plan/token/receipt identity, and phase semantics. Expose observed tick
overshoot and historical pause evidence without claiming current pause or goal
completion. Missing records remain unknown, and intent capsules are not terminal
receipts or a durable effect journal.

Twenty executed Python test groups include joined loopback TCP doubles, 480 state
combinations, every-byte corruption, an actual C++ encoder fixture, ambiguous
commit recovery, read-only polling, custody/special-file rejection, and file plus
directory sync failures before preparation. Re-execute native plugin SDK/protobuf
API-double tests: GCC and Clang each pass 341 assertions plus three independent
hash vectors under UBSan and warning denial. No actual DFHack SDK/protobuf ABI,
live-game, Rust/MCP or full repository qualification, production runner, default
MCP tool, or compatibility admission is changed or claimed.

Operator usage and recovery semantics: docs/BOUNDED_SIMULATION_CLIENT.md.
