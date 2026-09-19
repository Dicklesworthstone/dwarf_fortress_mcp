# Rust job-control/1.9 implementation status

This isolated development extension connects the existing native job-suspension
contract to Rust evidence, fixed RPC and durable coordination. It does not change
existing native wire generations, read-only runtimes, the production runner map,
the empty compatibility registry, dependencies or the frozen MCP tool surface.
It is not admitted production or live-game functionality.

Native scope and prerequisites remain in `JOB_SUSPENSION_CONTROL.md`: suspend or
resume one existing idle supported job at a completed workshop/furnace while the
game is paused. No arbitrary command, job creation/deletion, worker interruption,
repetition change or clock advancement is added.

## Exact selected-job evidence

`dfmcp_adapter::job_suspension` decodes the complete bounded native `DFMJS019`
observation. It preserves canonical source bytes and hashes them as the witness.
Identity, scalar/string bounds, reserved flags and holder presence are checked.
Unknown native type numbers and negative positions are preserved rather than
inventing canonical enum membership or valid map locations. This is one selected
native observation, not a complete or canonical world snapshot.

`SuspensionPlan` binds the observation, native job, desired Boolean, ASCII key,
plan digest and native prepare token. Only paused, idle, supported jobs at completed
production holders form a plan. Eligibility is not authority or a reservation.

`SuspensionEffect` requires the complete sealed plan. It checks every prepared
identity field and the receipt hash. Applied/not-applied readback must also match
an independently reconstructed exact observation: only the intervention sequence
and suspension bit may differ. A self-consistent receipt for a different job,
later tick or unrelated readback is insufficient. Prepared, Unknown, Applied,
NotApplied and Refused remain distinct. Absent readback and nonterminal receipts
require canonical zero backing bytes. Hashes are checksums, not signatures or
proof that the job later completed.

## Fixed native RPC

`job_suspension::rpc::JobControlRpcClient` binds only Handshake, ReadJob,
PrepareSuspension, CommitSuspension and QuerySuspension in the existing isolated
package. It validates canonical protobuf fields, exact per-method reply shapes,
distinct method IDs and nonce. Generation and software identity remain pinned
throughout the connection. Native refusal code 4 is a general adapter rejection,
not falsely classified as always meaning that the fortress is unloaded.

Each call has an explicit 1..60000 millisecond absolute wall-time bound. Concrete
TCP accepts numeric loopback addresses with a nonzero port, includes connection
and bootstrap in one deadline, and reapplies the remaining timeout to blocking
reads/writes. Frames are bounded to 8 KiB; notifications are limited to eight,
64 KiB each and 256 KiB in total. Injected streams must honor the same contract.

Every failed wire call, including failed evidence validation, fences the stream.
Local invalid arguments write nothing. Commit requires matching native Prepared
evidence. There are no automatic retries. Query returns validated evidence or
absence; absence never proves that a prior setter did not execute. Even a native
error envelope after commit dispatch can represent an ambiguous outcome.

## Durable production-authorized coordinator

`job_suspension::coordinator::JobControlJournal` connects directly to the RPC client
through the fixed `JobSuspensionSource` trait. It stores complete sealed plans,
validated native evidence and source manifests, never authentication tokens or
client nonces. Its fortress lineage uses the existing folder/site identity domain.

Prepare/commit/cancel require `ConfigureProduction` at reversible risk. The current
context must match the selected fortress and observation tick before a new
preparation or dispatch. Native IDs are not fabricated canonical EntityIds:
this development coordinator requires an explicit fortress-wide grant and refuses
entity/map-scoped or limited-use grants it cannot correctly account for.
Query authority suffices for reconciliation; that path has no prepare/commit edge.
No grants are manufactured by the transport or coordinator.

The foreground transaction is:

1. `prepare` validates the plan/source/context, obtains exact native preparation
   evidence and syncs it before acknowledging durable preparation.
2. `commit` syncs `DispatchStarted` BEFORE the only native commit call, rechecks
   authority and remaining budget, then syncs a verified outcome or Indeterminate
   state before acknowledgement.
3. `reconcile` queries retained native evidence without repeating the setter.
   Missing records, changed generation/software, prepared replies after dispatch,
   transport failure and unknown outcomes never restore dispatch authority.
4. `cancel_before_dispatch` durably cancels only a still-prepared local record.
   Cancellation is not a native NotApplied receipt or an undo command.

Exact key replay preserves original content and does not renew native TTL. Native
Unknown records and terminal evidence cannot be rewritten. `unresolved` retrieves
complete pending records after transcript loss or restart; it fails an insufficient
bound instead of silently omitting recovery work. Cached terminal results never
invoke the native setter again.

The separate append-only journal has an identity-bound hash chain, consecutive
transition numbers, bounded records and commit footers. Replay validates complete
native evidence and legal state transitions. Incomplete/corrupt tails are refused
without truncation; append/sync/custody errors fence the journal. Retention is
bounded to 4,096 keys, 16,384 transitions, 2 KiB record bodies and 64 MiB. Capacity
and caller byte/time budgets are reserved before native work. A synchronous
filesystem syscall cannot be forcibly interrupted; elapsed budgets are checked
around I/O, and expiration does not authorize redispatch or a false success.

`open_private_job_journal` uses an operator-selected absolute normalized path, a
private 0700 directory, single-link 0600 regular file, exclusive file lock and
revalidated inode/owner custody. `open_private_job_recovery` opens an existing
journal with Query authority and a genuinely read-only descriptor. It never
creates a missing journal. Private files currently require Unix. No MCP request
may choose the journal path. Generic test stores must honor the storage contract.

## Evidence executed and retained

There are 34 registered Rust regression tests: nine evidence/codec, nine RPC and
16 coordinator groups. They cover native vectors, every-bit effect corruption,
all states, forged rehashed readback, malformed input, eligibility, fragmented I/O,
lost replies, source changes, sync ordering/failures, replay, cancellation,
authority/budget checks, capacity, torn tails and rehashed invalid histories.

**These Rust tests have not been compiled or executed.** Rust, Cargo and rustfmt
were unavailable. No workspace, clippy, real-plugin, real-filesystem crash or live
fortress qualification is claimed. The native fixture provenance is unchanged;
its prior C++ test evidence is not evidence for execution of this Rust increment.

The independently implemented Python stdlib reference checker was executed:

```sh
python scripts/check_job_control_vectors.py
```

It reproduced the native observation witness, plan digest, prepare token,
controlled readback witness and terminal receipt; checked five native states;
rejected 1,608 effect bit corruptions, 1,361 journal byte corruptions, 1,358
incomplete journal prefixes and six rehashed invalid state histories.
`docs/evidence/job-control-rust-reference.json` retains the result and SHA-256 of
the reviewed Rust source. This checks reference format arithmetic/invariants,
not the Rust implementation or filesystem behavior. Source hashes do not turn
reference execution into Rust qualification.

## Remaining functional integration

The selected-job evidence/RPC/durable-coordination path is implemented as library
source. It is not yet exposed through an MCP intent/plan/execution route or a
production runner. Rust compilation and execution, real DFHack build/transport,
real filesystem fault campaigns, live-game verification and compatibility
admission remain outstanding. Do not bypass these gaps by widening a read-only
profile, accepting invented authority or reporting native RPC success as proof
of a completed fortress-level goal.
