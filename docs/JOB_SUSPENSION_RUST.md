# Rust job-control/1.9 implementation status

This is an isolated unadmitted development extension of the native contract in
`JOB_SUSPENSION_CONTROL.md`. It does not change any existing bridge generation,
read-only runtime, production runner map, compatibility entry or dependency.

## Exact selected-job evidence

`dfmcp_adapter::job_suspension` decodes the complete bounded native `DFMJS019`
observation without assuming a native type registry or turning negative positions
into valid map locations. It preserves canonical source bytes and hashes those
bytes as the witness. Identity, scalar/string bounds, reserved flags and holder
presence are checked before a value is exposed.

An immutable `SuspensionPlan` binds the observation, native job, desired boolean,
ASCII idempotency key, deterministic plan digest and native prepare token. Only
paused, idle, supported jobs at completed production holders can form this plan.
Eligibility is not capability authority, a reservation or proof of future validity.

`SuspensionEffect` requires the caller's complete sealed plan. Decoding rejects
identity substitutions and verifies the native receipt hash. Applied/not-applied
readback is additionally checked against an independently reconstructed exact
observation: only the local intervention sequence and suspension bit may differ.
A self-consistent receipt for another job, later tick or unrelated readback is
not sufficient. Unknown, prepared, refused, applied and not-applied remain distinct;
absent observations and nonterminal receipts require canonical zero backing data.
These hashes are checksums, not signatures or proof of later job completion.

## Fixed native transport

`job_suspension::rpc::JobControlRpcClient` binds only Handshake, ReadJob,
PrepareSuspension, CommitSuspension and QuerySuspension in the existing isolated
package. There is no public command, plugin or method selector. The client checks
canonical protobuf scalars, required fields and exact per-method optional-field
shapes; duplicate/reserved method IDs and mismatched nonces are refused.
Generation and software identity stay pinned for the lifetime of a connection.

Each call has an explicit 1..60000 millisecond absolute wall-time budget. Concrete
TCP connections accept only numeric loopback addresses with a nonzero port and
include connection establishment and all bootstrap methods in the same deadline.
Every blocking read/write reapplies the remaining absolute timeout. Injected
streams must obey the same contract. Frames are bounded to 8 KiB, with at most
eight notifications, 64 KiB each and 256 KiB in aggregate.

Every failed wire call, including failed evidence projection, fences the connection.
Local invalid input writes nothing and leaves a healthy stream usable. Commit
requires a matching native Prepared record, not merely an invented token. The
client never retries automatically. Query returns either validated evidence or
absence; absence is not proof that an earlier setter did not execute. Even a
native error envelope after commit dispatch is potentially ambiguous.

This is a low-level operator-credential transport, not a capability-granting MCP
facade. A durable authorized coordinator must record dispatch intent before
calling commit and must pin the complete manifest across reconnects. Native
preparation replay does not extend its original 60-second lifetime.

## Evidence and limits

Eighteen Rust regression groups are registered: nine evidence/codec groups and
nine transport groups. They include native vectors, every-bit effect corruption,
truncation, all five states, rehashed forged readback, key/world substitution,
eligibility, UTF-8, sequence exhaustion, fragmented I/O, all five request shapes,
method aliases, lost replies, query-only recovery, local validation, changed
identity, malformed envelopes and notification/allocation bounds.

They have not been compiled or executed here: Rust, Cargo and rustfmt are absent.
An independent Python calculation verified the existing native fixture's observation
witness, plan digest, token, controlled readback witness and terminal receipt.
The fixture files' Git blob identities match upstream. This calculation does not
execute Rust and is not Rust, real-plugin, live-game or repository qualification.

Durable authorized external coordination, MCP exposure and actual DFHack campaigns
remain separate work at this increment. Never interpret a native missing record
or a lost reply as proof that a setter did not run.
