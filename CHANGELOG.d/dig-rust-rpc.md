### Capability-scoped Rust RPC for native mining

Add the fixed six-method dig/1.16 Rust client and typed source/preparation API.
Pin each connection to one region and exact native software/incarnation, enforce
current fortress and full-halo/whole-block capability scopes, and reject renewed
byte/deadline allowances. Only the same connection's fresh preparation can enter
one commit attempt. Query/replayed evidence never grants dispatch; ambiguous or
malformed native replies fence the stream without reconnect or retry.

Twelve RPC regression groups join ten codec groups; all 22 are registered but
UNCOMPILED AND UNEXECUTED because Rust/Cargo/rustfmt are unavailable. Existing
fixture reconstruction is independent Python evidence only. This adds no native
wire change, dependency, Rust durable coordinator, MCP route or admission. The
remaining confirmation/lease/checkpoint/journal integration is explicit in
`docs/DIG_RUST.md`; this adapter is not a replacement for that coordinator.
