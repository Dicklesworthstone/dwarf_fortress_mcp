# Typed bounded-run Rust integration

`dfmcp_adapter::bounded_run` implements the existing native `run/1.13`
observation, sealed plan, receipt decoder and fixed-method RPC client. This is
explicitly unadmitted development source. The native protocol, Python developer
client, production map, dependency graph and compatibility registry are unchanged.

The decoder checks canonical widths, bools, key bounds, game clocks, plan and
token commitments, receipt hashes, and the complete phase/reason/flag matrix.
Pending/prepared evidence is not a terminal receipt. Source loss is not verified
pause. Known overshoot is exposed; unknown ticks remain absent. The maximum
canonical record is 274 bytes (146 fixed bytes plus a 1..128-byte key); the native
374-byte ceiling is deliberately looser, not another wire format.

The client binds only Handshake, ObserveRun, PrepareRun, CommitRun, QueryRun and
CancelRun. Bind IDs must be distinct and non-core. Reply presence is checked by
operation, including absent Query records. Nonces, protocol, software identities,
generations and plan identities must agree. An active record requires a matching
native owner. Errors fence the connection; no method reconnects or retries.
TCP connect, negotiation, bindings and later requests share one absolute deadline
and one byte allowance. Notification count and aggregate bytes are bounded.

Every call checks the supplied OperationContext. Queries require Query; prepare,
commit and cancellation require ControlClock at Guarded risk. Starting additionally
checks the complete requested game-tick horizon and budget. Limited-use grants
are refused rather than silently replayed. Cancellation is an independently
authorized safety pause, not renewed unpause authority.

**Source identity limitation:** run/1.13 carries no world folder/site identity.
The adapter therefore requires the explicit NIL fortress control domain; grants
scoped to a named fortress are rejected, not treated as evidence of the target.
An integrating coordinator must bind the exact operator-selected endpoint,
software and generation, and must journal dispatch before calling CommitRun.
This low-level API is not itself durable coordination or production admission.

Thirteen Rust test groups are registered, covering 480 phase/reason/flag cases,
corruption and truncation, a frozen independent wire vector, authority/horizon
refusals, fragmented scripted I/O, missing records and failed-commit fencing.
They have **not been compiled or executed** in this environment: Rust, Cargo and
rustfmt are unavailable. The fixture was generated with Python's independent
SHA-256/struct implementation. That is not execution of the Rust decoder, RPC,
MCP, actual DFHack SDK or a live fortress. Full qualification remains outstanding.
