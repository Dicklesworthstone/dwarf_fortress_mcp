### Bounded simulation clock ownership (native foundation)

Add a bounded-run engine with sealed-input preparation, one active native clock
owner, 1..1200 game-tick and 1..60000 millisecond limits, cancellation, and
fail-closed shutdown. Retain ownership while a safety pause is unverified. Never
repeat an unpause after a setter attempt, including ambiguous exceptions; fence
competing preparations and reject replacement-fortress setters. Preserve stop
reasons and actual observed ticks rather than claiming exact-tick execution or
production completion.

The deterministic C++ callback-double suite passes 351 assertions with both GCC
and Clang, warnings as errors and undefined-behavior sanitization. This initial
foundation is not a DFHack RPC endpoint, MCP runtime, SDK build, or live-game
qualification. No existing profile, production runner, or admission is changed.
