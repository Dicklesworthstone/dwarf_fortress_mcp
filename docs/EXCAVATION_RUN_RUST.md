# Rust excavation-run evidence and durable coordinator

The adapter's `excavation_run` module implements the existing native 1.18
capture/plan/receipt semantics and an append-only coordinator over
`EffectJournalStorage`. It does not call the Python implementation, change the
native wire, expose another MCP tool, or grant production admission.

## Evidence

`ExcavationCapture::decode` retains exact bounded native bytes, an eligible clock,
folder/site/generation/map-size identity, one 8x8-or-smaller region and tagged
cells. Hidden/missing cells have no attribute payload. `ExcavationRunSpec::new`
requires a feasible sampled stability window strictly before the tick deadline.
`ExcavationRunPlan::new` requires a paused, visible, dry, not-yet-satisfied goal.
Native plan/token digests match the existing C++ and Python domains. The separate
DFMEP018 journal encoding includes the key and reconstructs the complete plan.

`ExcavationRunRecord::decode` validates the whole receipt, not just its checksum:
phase/reason/effect flags, source/region/sequence, sampled ticks, streak count,
trigger-specific evidence and stopping semantics. `validate_successor` refuses
phase regression, terminal changes, changed triggers and same-tick sample inflation.
`sampled_floor_reported`, `historical_pause_verified` and `resolved` are distinct.
SourceLost is terminal history but remains unresolved work. No value proves current
pause, continuous stability, mining causality, safety or permission to retry.

## Coordinator API and ordering

The trusted runtime supplies an `ExcavationRunSource` and an exclusively owned
`EffectJournalStorage`. `create` requires EXCLUSIVELY NEW storage; never pass an
old empty or truncated file as new storage. The backend must pin private file and
parent identity, hold its lock, reject unsafe path/link/mode changes, and sync
BOTH file and parent in `sync`. The generic storage trait's default identity hook
is not sufficient for deployment. No new filesystem backend is provided here.

`ExcavationCoordinator::open` accepts only an existing, nonempty, intact journal
and current Query authorization for its expected fortress. It recovers the exact
endpoint/software/generation binding from the journal without contacting native
code. Neither open nor receipt replay reconstructs dispatch permission.

`start(source, plan, confirmed_digest, context)` requires fresh explicit plan
confirmation, scoped Query/Plan/ControlClock authorization, enough complete-run
budget, and grant validity through the planned tick horizon. It checks source
binding, an exact fresh capture, and native key absence. Local key reuse and ANY
unresolved local intent block new starts. Its ordering is:

    durable intent -> native prepare -> durable prepared receipt
    -> durable dispatch intent -> one commit -> durable returned evidence

Every publication rechecks existing bytes, writes fully, flushes/syncs and rereads
exact bytes before publishing the new in-memory state. Failed writes/syncs fence
the owner without truncating or repairing storage. A failed commit remains
indeterminate; its recorded intent blocks both the old key and a new-key bypass.
`ExcavationDispatch` is non-cloneable with private fields. Only the coordinator
constructs it, after dispatch sync; the source consumes it by value. This proves
local ordering only, not authority, a global lease or native nonapplication.

`recover(source, key, cancel, context)` performs one keyed query or one
cancellation request. Cancel requires current clock authority and exact original
source binding, with a durable cancellation marker first. Query may recover old
terminal history from a later generation of the same endpoint/software/fortress.
Old-source active records, substituted sources and inconsistent successors are
rejected. An absent native record writes nothing and remains unresolved. Terminal
recovery is offline and immutable. Cancellation never calls commit.

## Storage and budgets

Rust journal magic is DFMEJ018. Every frame contains payload length u32, sequence
u32, previous digest32, payload and checksum32. All integers are big-endian.
Checksum is SHA256(`dfmcp-excavation-coordinator/1` + NUL + all preceding frame
bytes). The first previous digest is zero. Payload tags: 0 binding (first only),
1 keyed plan intent, 2 prepared receipt, 3 dispatch digest, 4 native evidence,
5 cancellation request. Text/nested fields are u16 length-prefixed. Replay repeats
both integrity and transition checks; bytes are not a signature or anti-rollback
root. The format is deliberately distinct from the existing Python JSON journals;
no import/migration or interchangeable file claim is made.

Bounds are 2 MiB, 2048 frames, 4096 payload bytes and 256 unique intents. New work
reserves complete-start capacity; nonterminal progress leaves capacity for a
cancellation marker and terminal evidence. Whole-file reads and native-call
allowances are charged to one per-operation byte budget. One shrinking cooperative
wall deadline covers all steps, and cancellation is checked at each storage/native
boundary. The source must enforce it during actual transport I/O; storage syscalls
are not forcibly interruptible. There is no internal polling or detached work.

## Validation and remaining integration

    python3 scripts/check_excavation_run_rust_vectors.py
    cargo test --locked --offline -p dfmcp-adapter excavation_run

Executed here: seven independently constructed Python byte fixtures, including a
complete five-frame journal. Exact local/uploaded Git blob hashes were checked.
Added: 22 Rust tests covering native vectors, malformed/rehashed evidence, phase
matrices, lost commit replies, sync/partial-write failures, exact confirmation,
authority/horizon budgets, pending-work fences, cancellation and offline replay.
Those Rust tests are UNCOMPILED AND UNEXECUTED. Cargo, rustc, rustfmt and the locked
dependency cache are unavailable here. Do not call this Rust-qualified or infer
that the independent Python fixture check executed the Rust implementation.

Still required: a concrete 1.18 Rust RPC/source implementation, a qualified private
storage backend, supervised Cx region/cancellation and global clock-lease
integration, session policy, MCP wiring, real DFHack SDK/live campaigns and full
qualification. The existing native and Python workflows remain unchanged.
Beads df-dfhack-bridge-plane-c-pic.4/.5 and df-action-coordinator-exec-ero.4 remain open.
