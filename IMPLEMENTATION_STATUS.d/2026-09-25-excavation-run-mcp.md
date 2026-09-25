# Excavation-run/1.18 MCP source integration

`dfmcp-excavation-run-dev-server` and its registered eleven-tool handlers now
compose the typed session, actual private-file backend, native RPC client and
durable coordinator. Offline is default; Recover queries only; Control requires
separate operator clock permission and current grants. No native designation or
cross-profile blueprint execution is added. Production admission is unchanged.

31 new Rust groups across both session/MCP increments are UNCOMPILED AND
UNEXECUTED: 14 adapter, 12 dispatcher/presentation, four runtime and one binary
argument test. No cargo/rustc/rustfmt or locked dependency graph is available in
this editing environment. This is not Rust, MCP, SDK, live-game, physical
power-loss or full repository qualification.

Actual Python reference checks pass 53 valid/104 invalid schema cases, three
unchanged native fixture identities and plan/token/receipt hashes, inventory
goldens, 288 sampling arithmetic cases and conservative output-size models.
Exact scope and source hashes are in `docs/evidence/excavation-mcp-reference.json`.
Global controller fencing and actual game checkpoint/restore remain absent.
