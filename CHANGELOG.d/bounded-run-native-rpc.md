### Isolated native bounded simulation RPC (run/1.13)

Connect the bounded-run engine to six authenticated fixed DFHack RPC operations,
canonical SHA-256-bound observations/plans/records, and native update servicing.
Connection loss or reply allocation failure does not abandon the stop obligation.
Cancellation retries only safety pauses. An atomic unload gate vetoes at
SC_BEGIN_UNLOAD until callback-driven drain proves quiescence, including unload
racing with the unpause setter. No existing wire generation or production runner
is changed; this is an explicitly unadmitted developer-native surface, not MCP
integration or a durable coordinator.

Execute the actual plugin translation unit with SDK/protobuf API doubles: GCC and
Clang each pass 341 assertions, warnings as errors and UBSan, plus three Python-
verified canonical vectors. These tests do not establish real protobuf, SDK/ABI,
DFHack lifecycle execution, live-game, Rust/MCP, or repository qualification.
See docs/BOUNDED_SIMULATION_RUN.md for exact stop, uncertainty, and usage contracts.
