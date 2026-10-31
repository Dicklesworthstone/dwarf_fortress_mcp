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
is no assigned worker, the completion timer is -1, the local closed eligibility
policy accepts the job, and its holder is a completed workshop or furnace.
Every prerequisite is rechecked at commit by reproducing the exact witness.
The caller must inspect again after a changed field, tick, job-ID horizon or
local sequence.

### Closed eligibility for the 53.16 SDK

DFHack 53.16 provides `Job::getHolder` and `Job::getWorker`, but no public
`Job::getJob` or `Job::isSupportedJob`. The plugin resolves the selected job by
walking the real `world->jobs.list` completely. The scan is bounded to 65,536
jobs and rejects null entries, duplicate IDs, cycles, broken previous links,
mismatched `job.list_link` back-references, and IDs outside the current
`job_next_id` horizon. Finding the selected ID does not end validation early.

The existing observation's `supported` bit now represents this explicit closed
type/holder policy:

| Native job type | Required holder type |
| --- | --- |
| `ConstructBed`, `ConstructDoor`, `ConstructThrone`, `ConstructTable` | `Workshop` |
| `PrepareMeal` | `Workshop` |
| `SmeltOre`, `MeltMetalObject`, `MakeCharcoal` | `Furnace` |

An eligible job must also have an empty reaction name, no specific references,
no attached items, and a nonempty bounded collection of non-null requirement
filters. Its `working`, `fetching`, `bringing` and `special` flags must be false,
and `posting_index` must be exactly -1. The holder's queue must contain the exact
selected job pointer once, contain no null or duplicate-ID entries, and fit the
ten-job native queue bound. All collection bounds and pointers used in the
selected observation are checked before publication.

This supports only idle **unposted, unclaimed** jobs. A queued job with a live
posting, reserved ingredients, an active worker, a custom reaction or an
unlisted type remains inspectable but cannot be prepared. No brewing claim is
made: `BrewDrink` is not a job enum member in this SDK. The policy does not infer
recipe feasibility, material availability or workshop subtype compatibility.
It accepts only an already existing exact job and controls its suspension bit.

The setter repeats a complete validated observation and eligibility check under
the RPC's existing game suspension, then resolves the exact global-list member
again before writing only `job->flags.bits.suspend`. It does not call
`removeWorker`, `removePostings`, enqueue work, change any other flag, or advance
time. The unposted/unclaimed restriction deliberately avoids needing those
additional effects. It cannot establish that a resumed job will execute or
finish. These checks consume the existing `supported` field; method names,
wire encodings and digest domains do not change.

The inspected upstream source is DFHack `53.16-r1`, commit
`c80747da32aad3bdd5915c60105136d6558aca39`, whose `library/xml` submodule pins
df-structures `1dd01aad64219afa0578f1328cf15bd0c6006d5a`:

- [Public Job API](https://github.com/DFHack/dfhack/blob/c80747da32aad3bdd5915c60105136d6558aca39/library/include/modules/Job.h).
- [Holder/worker resolution, world-list linkage and workshop assignment](https://github.com/DFHack/dfhack/blob/c80747da32aad3bdd5915c60105136d6558aca39/library/modules/Job.cpp).
- [Upstream workflow's private support predicate and posting-aware suspension](https://github.com/DFHack/dfhack/blob/c80747da32aad3bdd5915c60105136d6558aca39/plugins/workflow.cpp).
- [Upstream suspendmanager's flag and worker behavior](https://github.com/DFHack/dfhack/blob/c80747da32aad3bdd5915c60105136d6558aca39/plugins/suspendmanager.cpp).
- [Exact job enum, flags, posting index and reference fields](https://github.com/DFHack/df-structures/blob/1dd01aad64219afa0578f1328cf15bd0c6006d5a/df.job.xml).
- [Exact building types](https://github.com/DFHack/df-structures/blob/1dd01aad64219afa0578f1328cf15bd0c6006d5a/df.d_basics.xml)
  and [building job queue](https://github.com/DFHack/df-structures/blob/1dd01aad64219afa0578f1328cf15bd0c6006d5a/df.building.xml).

Reviewing these sources establishes the intended API/field contract. Actual
matching SDK compilation and a disposable-fortress effect trial remain separate
evidence requirements under `dfmcp-reality-wiring-proof-4bf.8`.

A stable ASCII idempotency key is 1..128 bytes. Prepare is nonmutating and retains
its original token when replayed. It does not refresh its 60-second monotonic
lifetime. Reusing a key with different content conflicts. The bridge retains at
most 4,096 records and refuses new keys at capacity rather than evicting effects.

Commit changes only `job->flags.bits.suspend`. Before that sole setter, it retires
competing older preparations with a monotonically increasing intervention sequence
and records `Unknown`. This also applies to no-op setters. A throwing setter is
never retried: the same immediate readback may still establish applied/not-applied
evidence if the flag and intervention sequence are the only changes. Failed
readback or any other observed-field change leaves the result unknown.
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

## Historical synthetic evidence

An earlier version of `python scripts/test_job_suspension_native.py --ubsan` and
the same command with `--compiler clang++` each passed 397 C++ assertions in nine groups under C++17,
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
That historical boundary double incorrectly supplied `Job::getJob` and
`Job::isSupportedJob`, which the actual 53.16 public SDK does not export. Its
`BrewDrink` type and `Workshop=1` values were synthetic and do not establish real
enum compatibility. The later real-SDK source repair retained all job writes as
unsupported, and the old double no longer compiled against that source. Those
397 historical assertions therefore do not validate the current native policy.
Current focused execution evidence must identify the repaired SDK-shaped
double, exact source and generated vectors independently. No Rust,
native-plugin, live-game, or production admission follows from synthetic tests.
