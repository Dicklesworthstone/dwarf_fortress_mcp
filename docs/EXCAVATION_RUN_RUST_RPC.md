# Concrete Rust excavation-run/1.18 transport

`dfmcp_adapter::excavation_run::rpc::ExcavationRpc` implements the existing
`ExcavationRunSource`. It communicates directly with the fixed native plugin;
there is no Python subprocess, new runtime, native protocol change or MCP tool.
Use the durable coordinator, not direct trait calls, for the application lifecycle.
Beads: df-dfhack-bridge-plane-c-pic.4/.5 and df-action-coordinator-exec-ero.4.

## Control and recovery

`connect_control(fortress, region, nonce, context, cancellation)` reads the
operator environment, negotiates, and acquires one complete native capture. The
caller can build its typed plan from `initial_capture()`. This capture is a
historical observation; start still performs the coordinator's fresh-capture and
key-absence checks. Public construction requires scoped Query authority. The
existing start checks are repeated at the actual socket send boundary, including
Plan/ControlClock grants and validity through the planned game-tick horizon.

`connect_recovery(binding, region, nonce, context, cancellation)` does not read
terrain. Its handshake must match endpoint/software and may report a later native
generation. Folder/site/map dimensions remain the journal's expected historical
scope, not a new observation of the currently loaded fortress. This mode refuses
Observe/Prepare/Commit. Query verifies the original complete plan and accepts only
terminal history from an older generation. Cancel requires the exact original
generation and current ControlClock authority. Native identity checks remain the
last independent guard against touching a replacement fortress.

Only a successful Prepare on this connection can precede Commit. The source
consumes both that local preparation and the coordinator's non-cloneable dispatch
permit before authorization or I/O. Queried/imported Prepared records cannot
create the local permit; there is no retry, reconnect or resume-commit operation.
A second preparation attempt on the same connection is refused. Cancellation
also consumes local preparation. Any method error fences and shuts down the socket.

## Operator isolation and resource ownership

The only accepted DFMCP environment names are:

    DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18=1
    DFMCP_EXCAVATION_RUN_TOKEN=<32..256 UTF-8 bytes>
    DFMCP_EXCAVATION_RUN_ENDPOINT=127.0.0.1:5000
    DFMCP_EXCAVATION_RUN_ALLOW_CLOCK=1

Endpoint is optional, numeric canonical IPv4 loopback only. Clock permission may
be absent or zero for observation/query/cancellation. Cancellation still needs
its scoped Rust ControlClock grant. Removing clock permission cannot disable
native stop ownership. Other DFMCP names, including production admission state,
fail closed. Endpoint, token, opt-in and current permission are rechecked at every
socket read/write boundary. Credentials have no Debug output or journal fields.
The supervising runtime supplies a fresh 32-byte nonce; it is not authorization.

One connection owns one absolute deadline, at most 64 exchanges including method
binding, and a decreasing byte allowance. Later OperationContexts can narrow but
not refill those allowances. Each exchange reserves 272 KiB before any dispatch,
covering the 2 KiB request, 4 KiB reply, notification payload and headers. At most
eight notifications/256 KiB are admitted. Varints, field sets/types, bindings,
nonce/version, generation/software, ownership, counts and record successors are
checked before returning typed evidence. An absent record cannot replace one
already observed on the same connection.

`ExcavationCancellation` is a one-way shared signal owned by the supervising
caller. It is checked during partial I/O, using socket wait slices of at most
100 ms. Connect makes one loopback attempt with the same short bound. No polling
thread or detached work is created. This is a synchronous, cooperative adapter,
not an Asupersync Cx region integration or a hard real-time guarantee. Signal
cancellation and drop/fence the source when the caller's region is closed; the
native plugin continues to own its previously committed bounded stop.

## Evidence status

    python3 scripts/check_excavation_rust_rpc_vectors.py
    cargo test --locked --offline -p dfmcp-adapter excavation_run

Executed here: six independently constructed Python protobuf request fixtures,
including agreement with the existing native plan/token vectors. The fixture
checker does not execute the Rust encoder or socket client.

Added but UNCOMPILED/UNEXECUTED: 16 Rust tests, including actual loopback TCP test
code joining the concrete source to the real coordinator, lost-commit recovery,
dispatch sync failure, control-mode isolation, clock revocation, source drift,
notification/size/nonce checks, cancellation during stalled I/O and call budgets.
Cargo/rustc/rustfmt and the locked dependency cache are unavailable in this
session. No Rust, real DFHack/protobuf, live-game, full-workspace or production
qualification is claimed. Private storage and runtime/session/MCP integration
remain separate from this transport increment.
