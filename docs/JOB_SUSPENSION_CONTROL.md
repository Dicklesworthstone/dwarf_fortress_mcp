# Guarded job suspension: development protocol 1.9

This separate profile adds a first narrow production intervention: set the
suspension flag of one existing, idle, supported workshop/furnace job. It never
unpauses the fortress, deletes a job, detaches its worker, changes repetition,
creates a work order or runs an arbitrary DFHack command. Read-only profiles,
pause-control/1.7, the production runner map and the empty compatibility registry
are unchanged. This is unadmitted development source, not live qualification.

## Native boundary

`bridge/dfhack-job-control-v1_9` builds `dfmcp_job_control_v1_9`, package
`dfmcp.job_control.v1_9`. Its five fixed RPC methods are `Handshake`, `ReadJob`,
`PrepareSuspension`, `CommitSuspension`, and `QuerySuspension`. All are registered
with flags zero for DFHack's game-suspension boundary. There is no onUpdate
callback, timer, detached work, shell/Lua surface, or native pointer in a record.
The separate `DFMCP_JOB_CONTROL_TOKEN` must contain 32..256 bytes. Credentials and
16..64-byte client nonces are bounded; protocol 1.9 is checked exactly.

ReadJob returns one bounded `DFMJS019` observation containing native job ID,
job-ID horizon, bridge incarnation, local intervention sequence, game tick,
world folder/site identity, holder/worker, type/reaction, position, suspension,
repeat, completion timer, attachment/filter counts, paused state, and explicit
eligibility inputs. It is a selected native observation, not a complete world
snapshot or an observation borrowed from spatial/1.8. Its witness is SHA-256 over
those exact bytes. No count of requirements proves their content or feasibility.

Prepare requires an exact witness and the deterministic plan digest for the
selected job and desired Boolean. It refuses unless the game is paused, there
is no assigned worker, the completion timer is -1, DFHack reports a supported job,
and its holder is a completed workshop or furnace. Every prerequisite is
rechecked at commit by reproducing the exact witness. The caller must inspect
again after a changed field, tick, job-ID horizon or local sequence.

A stable ASCII idempotency key is 1..128 bytes. Prepare is nonmutating and retains
its original token when replayed. It does not refresh its 60-second monotonic
lifetime. Reusing a key with different content conflicts. The bridge retains at
most 4,096 records and refuses new keys at capacity rather than evicting effects.

Commit changes only `job->flags.bits.suspend`. Before that sole setter, it retires
competing older preparations with a monotonically increasing intervention sequence
and records `Unknown`. This also applies to no-op setters. If the setter throws,
readback fails, or any other observed field changes, the result stays unknown.
Only complete same-identity readback can create applied/not-applied evidence.
A changed/expired precondition produces a retained refusal without dispatch.
Neither unknown nor refused keys can later be dispatched again.

Terminal receipt hashes bind incarnation, key, plan, token, outcome, and exact
post-observation identity. They are checksums, not signatures. A lost response
can be queried without repeating the effect. Map/world load/unload clears the
native records and changes incarnation; pause/unpause events fence preparations.
Incarnation/sequence exhaustion never wraps. Native records are process-local,
so restart-safe dispatch additionally requires a durable external coordinator.
This native component alone must not be mistaken for that coordinator.

This does not fence malicious plugins, guarantee atomicity against arbitrary
external controllers, prove subsequent job execution, or override another
manager's later decisions. Paused, idle-only scope deliberately avoids automatic
worker interruption and keeps the first intervention smaller than arbitrary
job cancellation. Changing the setting back requires a new observation/key;
replaying a previous key is never an undo command.

## Wire identities

All integers in the observation/effect byte codecs are big-endian. Strings have
unsigned 16-bit byte lengths and validated UTF-8 without NUL. The observation is
bounded to 1,024 bytes; native effects are bounded to 322 bytes. Native IDs fit
positive-range signed 32-bit DF IDs (zero is allowed).

The plan digest is SHA-256 of `dfmcp-job-suspension-plan/1` + NUL, job u32,
desired u8, and the 32-byte observation witness. The prepare token is the first
16 bytes of SHA-256 over `dfmcp-job-suspension-token/1` + NUL, generation u64,
length-prefixed key, and plan digest. The receipt hashes
`dfmcp-job-suspension-receipt/1` + NUL, generation u64, length-prefixed key,
plan digest, token, state u8, after-known u8, after-suspended u8, after-tick u64,
and after-witness 32 bytes. Absent observation fields have canonical zero backing
bytes. States are Prepared=0, Unknown=1, Applied=2, NotApplied=3, Refused=4.
Nonterminal receipt backing bytes are zero and never terminal evidence.

The checked-in `.hex` fixtures were emitted by the executed native handler and
independently checked with Python hashlib. They are synthetic data with explicit
DFHack/protobuf doubles, not a captured live fortress.

## Executed evidence

`python scripts/test_job_suspension_native.py --ubsan` and the same command with
`--compiler clang++` each passed 397 C++ assertions in nine groups under C++17,
`-Wall -Wextra -Werror -pedantic -O1`, and nonrecovering UBSan. The tests compile
the actual complete native handler and engine with explicit boundary doubles.
They cover eligibility, changed witnesses, competing/no-op preparations, errors
before/after dispatch, lost success responses, idempotence, TTL boundaries,
record capacity, malformed input, authentication, and map/clock events.

Three independent mutations removing witness revalidation, dispatch replay
fencing, or prepare eligibility were rejected by those tests. These runs do not
execute real DFHack field acquisition, generated protobuf serialization, real
CoreSuspender/event delivery, a Rust client, or a game. The upstream references
used for API inspection are DFHack's `library/include/modules/Job.h`,
`plugins/suspendmanager.cpp`, and the existing native spatial field codec.
No Rust, native-plugin, live-game, or production admission is established.
