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

## Evidence and limits

Nine Rust regression groups are registered, including native vectors, every-bit
effect corruption, truncated/oversized input, all five states, recomputed forged
receipts, key/world substitution, eligibility, UTF-8 and sequence exhaustion.
They have not been compiled or executed here: Rust, Cargo and rustfmt are absent.
An independent Python calculation verified the existing native fixture's observation
witness, plan digest, token, controlled readback witness and terminal receipt.
The fixture files' Git blob identities match upstream. This calculation does not
execute Rust and is not Rust, real-plugin, live-game or repository qualification.

At this increment the module supplies evidence and sealed values only. Transport,
a durable authorized external coordinator, MCP exposure and actual DFHack campaigns
remain separate work. In particular, never interpret a native missing record or a
lost reply as proof that a setter did not run.
