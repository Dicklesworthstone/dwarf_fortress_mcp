# Rust excavation-run coordinator

- Add append-only journal replay, complete source binding and pending-work fencing.
- Require exact confirmed fresh capture, native key preflight and synced dispatch.
- Issue non-cloneable dispatch permits only after publication; recovery cannot resume.
- Preserve uncertain effects, immutable terminal evidence and durable cancellation.
- Share deadline/byte budgets and reserve room for stop finalization.
- Add 13 Rust coordinator tests and an independent complete-journal fixture.
- Seven Python fixtures checked; all 22 Rust tests remain uncompiled/unexecuted.
- Native socket, private backend, Cx/lease and MCP integration remain separate.
