# Excavation-run/1.18 host evidence and transport

`scripts/excavation_run_wire.py` and `scripts/excavation_run_rpc.py` implement the
existing native profile described by `EXCAVATION_RUN_NATIVE.md`. They add no native
method, MCP tool, dependency, production runner or admission. Work advances
`df-dfhack-bridge-plane-c-pic.4/.5` and `df-action-coordinator-exec-ero.4`.

The wire module decodes complete bounded captures and effects into immutable
records, binding region, fortress identity, clock, goal limits, plan and token.
Hidden/missing cells contain no attribute payload. Rehashed contradictory phase,
trigger, source or stability-window fields are rejected. Public capture fields
are checked against their exact canonical bytes before plan construction.
Reported sampled floor evidence never implies continuous stability, mining
causality, structural safety, present pause, or permission to retry unpause.

The transport binds only Handshake, ObserveRun, PrepareRun, CommitRun, QueryRun
and CancelRun under the fixed 1.18 package/plugin. Typed methods fix field sets;
no caller chooses a plugin, command, pointer, method ID or arbitrary payload.
A successful Prepare on this connection can enable one Commit attempt. Imported
or queried Prepared records cannot enable it. The permit is consumed before I/O;
Cancel consumes it too. This does not prove that native preparation was newly
created: the native protocol has no fresh/replay bit. The durable owner must
prevent identity reuse and unresolved-work bypass. The transport alone is not a
coordinator and must not be used to bypass durable dispatch intent.

Each connection pins one region and exact generation/software manifest. Source
changes fence the connection; a fresh recovery connection may inspect historical
terminal records. An absent record is unknown, not proof of nonapplication.
Seen terminal records are immutable; phase/sample regressions fail closed.

`Authority` requires the exact excavation opt-in, a 32..256-byte environment
credential, and a canonical numeric IPv4 loopback endpoint. Prepare/Commit require
clock permission. Each call rechecks current settings; clock revocation alone
leaves query/cancel available. Other DFMCP-prefixed environment state is refused.
These developer gates do not create a production capability or global clock lease.

One cooperative deadline covers connection, greeting, all bindings and calls;
partial reads do not renew it. A connection permits at most 64 native calls,
including seven negotiation calls, with no reconnect or retry. Requests/replies
are bounded to 2/4 KiB; each call admits at most eight notification frames and
256 KiB of notification payload. Duplicate/unknown fields, nonminimal/overflowing
varints, wrong wire types, alias bindings and noncanonical refusals are rejected.

## Executed evidence

`PYTHONPATH=scripts python3 -m unittest -v test_excavation_run_client` passes all
20 Python test groups, including joined fragmented loopback TCP doubles. Tests
cover exact native reference inputs, maximum 1991-byte records, every incomplete
prefix and single-byte corruption of a stopped record, rehashed false goal
claims, phase/reason combinations, hidden redaction, authority revocation,
source drift, lost commit replies, query-only recovery and nonrenewable budgets.
The independent Python vectors reconstruct inputs already used by the existing
C++ bridge test; this increment did not execute C++ or a real SDK/protobuf runtime.
No Rust/MCP, live fortress, power-loss or full-repository qualification is claimed.
