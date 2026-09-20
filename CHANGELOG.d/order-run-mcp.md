### Fortress-bound conditional running through the eleven-tool MCP interface

Connect native order-run/1.14 to its typed Rust adapter and durable coordinator,
without a subprocess wrapper. Add exact observation planning, reviewed confirmed
commit, one-query receipt waits, local/native cancellation, durable discovery and
offline/recover/control modes. Explicitly register all eleven dotted tool names.
Recheck runtime I/O, cancellation, operator selection and clock enablement at
native effects. Retain predicate/pause/production distinctions and scoped Agent
Turns; reserve full responses before work. Existing native profiles and admission
remain unchanged.

Twelve new Rust regression groups (39 across this integration) remain uncompiled
and unexecuted because Rust/Cargo/rustfmt are unavailable. Eight independent MCP
model/static groups pass: 160 accepted/488 rejected condition cases, 2,056
pagination cases and 27 conservative response shapes of 8,028..73,963 bytes.
Rerun six journal-reference groups and canonical fixtures. These are not Rust,
MCP, actual SDK/live-game, power-loss or full repository qualification.
