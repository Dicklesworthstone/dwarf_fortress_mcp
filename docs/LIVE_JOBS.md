# Live jobs: isolated development profile 1.2

The jobs profile supplies a missing gameplay input: the current native job roster.
It connects a separate DFHack plugin to the safe-Rust decoder, canonical world
projection, and the existing structured MCP query engines. Agents can inspect
suspended jobs, group work by job type or holder, search reaction names, compare
selected job facts across observations, and wait for an observed condition.

**Status:** end-to-end source integration and native-source mock checks. Rust
compilation, Rust tests, actual DFHack compilation, stdio execution, and live-game
behavior have not been verified in the editing environment. No compatibility tuple
or production runtime is admitted by this addition.

## Architecture and scope

```text
dfmcp-live-jobs-dev-server
  -> shared query / aggregate / search / baseline / condition-watch engines
  -> LiveJobsState canonical snapshot
  -> JobsRpcClient<DeadlineStream>
  -> authenticated DFHack native RPC
  -> dfmcp_jobs_v1_2: Handshake + ReadObservation
  -> bounded current world.jobs.list
```

The name `1.2` identifies a **jobs-only profile**, not a superset of the citizen
and announcement profiles. It has its own plugin, protobuf package, token setting,
opt-in, and process-scoped session identities. It does not modify the existing 1.0
or 1.1 native bridges, codecs, production runner map, or admission rules.

Do not merge its snapshot with a separately timed citizen/announcement snapshot.
Native worker and holder IDs are facts about this job read, not generation-safe
handles into another runtime. No fictitious unit or building entities or edges are
created to make such a merge look coherent.

## Native build and development entry

The self-contained plugin source is `bridge/dfhack-jobs-v1_2/`. Its CMake entry
uses DFHack's own `dfhack_plugin(... PROTOBUFS DfmcpJobsV1_2)` mechanism. Add that
directory as an external plugin in a compatible DFHack source checkout, through
`plugins/external/CMakeLists.txt` and `add_subdirectory(dfmcp_jobs_v1_2)`. The copied
external directory must retain its `proto/` subdirectory. Build it within that
DFHack build, not with standalone CMake: generated DF types, DFHack headers, and
protobuf Lite integration come from the host project. The build target is
`dfmcp_jobs_v1_2`.

The native RPC listener must be available on numeric loopback with this plugin
loaded. Configure `DFMCP_JOBS_TOKEN` in both the DFHack process and the MCP process
with the same operator-controlled 32..256-byte secret. A token is never an MCP
argument or a returned observation field.

From the repository root, with that environment already configured:

```bash
DFMCP_ALLOW_UNADMITTED_JOBS_V1_2=1 \
DFMCP_JOBS_ENDPOINT=127.0.0.1:5000 \
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-jobs-dev-server
```

The jobs process rejects other `DFMCP_*` settings, including all production
admission markers and settings inherited from the 1.1 runtime. Both the public
runtime entry and `fortress_open_session` enforce the development gate.

Open a session with `fortress_open_session`. Optional arguments are `max_jobs`
(default 1024, maximum 4096), `max_output_tokens` (default 8192), `max_bytes`
(default 2 MiB), `max_wall_millis` (default 5000), and `requested_capabilities`
(a list drawn only from `observe`, `query`, and `doctor`). Keep the returned
`session_id`; the profile cannot grant mutation authority.

Sixteen sessions, including concurrent bootstrap attempts, is the process ceiling.
There is currently no session-close operation; restarting the development process
releases sessions and its process-local monitoring state. Failed bootstrap
releases its reserved slot and never publishes a session handle.

## Useful queries

Supply the session ID alongside one of the following objects as the `query`
argument to `fortress_query`. `mode="schema"` returns the shared envelope schema.
Omit `mode` when supplying a structured `query`.

Find suspended jobs and inspect their observed assignment and reaction:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "entities",
    "kinds": ["job"],
    "fields": ["type_key", "reaction", "suspended", "worker_assigned", "holder_native_id"],
    "where": {
      "op": "compare", "field": "suspended", "comparison": "eq",
      "value": {"type": "bool", "value": true}
    },
    "limit": 4
  }
}
```

Count current jobs by their native job-type key:

```json
{"schema":"dfmcp.query/1","query":{"kind":"aggregate","kinds":["job"],"group_by":{"kind":"field","field":"type_key"}}}
```

Search declared reaction names without executing any reaction or command:

```json
{"schema":"dfmcp.query/1","query":{"kind":"search","text":"STEEL","kinds":["job"],"text_fields":["reaction"],"include_labels":false,"limit":8}}
```

Capture a bounded selection, refresh once through `fortress_observe` or
`fortress_wait`, then request `changes` with the exact returned
`captured.baseline` handle:

```json
{"schema":"dfmcp.query/1","query":{"kind":"capture","key":"job-assignments","max_game_ticks":1200,"select":{"kind":"entities","kinds":["job"],"fields":["suspended","worker_assigned","holder_native_id"]}}}
```

Baseline capture requires the complete selected result to fit its existing
256-row/256-KiB retention bounds. This is smaller than the live roster ceiling;
use filters to select a relevant subset. A job leaving the result is not proof
of completion: it may have completed, been cancelled, or left the filter.

Condition watches also use the existing schema and `record.watch` handles. For
example, after inspecting a job's canonical ID and generation, register a field
condition for `suspended == false` with an absolute `deadline_tick`, sampling
cadence, and required stable observations. `await_watch` validates the handle
before I/O, requires both Query and Observe grants, refreshes at most one job
observation, and evaluates the published target. A terminal watch skips that read.
See [condition watches](CONDITION_WATCHES.md) for the exact condition grammar.

## What a job observation establishes

Each complete read carries a nonzero source generation, exact DF/DFHack version
strings, world folder/site identity, game year/tick, pause state, next-job-ID
horizon, and strictly increasing native job IDs. Per-job fields include:

- `native_job_id`, numeric `job_type`, `type_key`, and `reaction`;
- `suspended`, `repeating`, and `worker_assigned`;
- `worker_native_id`, `holder_native_id`, and `position`;
- `attached_item_count`, `required_item_filter_count`, and `completion_timer_raw`.

The item counts count references/filters, **not missing materials or quantities
still needed**. An unassigned job is not automatically blocked. A suspended job
does not explain why it was suspended. The raw completion timer is not a reliable
remaining-time estimate. `blocking_reason` is explicitly unsupported. A negative
native map coordinate is represented as unknown position, not a usable tile.

The canonical fortress entity is ID 1. Job entity IDs are native job ID + 2,
within this profile only. Every projected fact carries the source digest. Observed
retirement and reappearance advance the job generation; this does not claim to
detect invisible deletion/reuse between two identical observations. The bounded
generation history refuses new identities when full rather than silently erasing
ABA protection.

Exact repeated observations are heartbeats. Ordinary progress advances sequence.
Bridge generation changes, clock regressions, and next-job-ID regressions create
a new epoch. Changing fortress or software version refuses the observation and
requires a new session. The native plugin advances its source generation on world
load/unload. Candidate snapshots are built before the visible root is replaced.

## Transport and publication

Both RPC methods are registered without remote permission. Every request includes
fixed protocol 1.2, bounded nonce/token, and a maximum job count. The server
collects the entire bounded linked list in one native RPC under DFHack's normal
RPC suspension. More jobs than requested, malformed records, duplicates, cyclic
lists, bad identity, invalid UTF-8, or excessive fields return rejection with no
partial observation payload.

The Rust TCP wrapper applies one absolute 1..60000 ms deadline across all reads,
writes, fragments, and text notifications in a call. Bootstrap negotiation and the
initial observation are separate bounded calls. It rejects non-loopback endpoints,
method-binding aliasing, wrong nonce/profile, software drift, regressed source
generation, duplicate/overlong protobuf values, excessive text notifications, and
invalid roster data. A failed stream is permanently fenced and never retried
implicitly. It does not echo DFHack text or credentials into errors.

MCP queries reserve the complete Agent Turn before allocating result bytes.
Baseline/watch state publishes only after the final response can be constructed.
A failed query render does not undo a preceding successful job observation; that
new anchor remains authoritative. Source-fenced sessions can still list and cancel
local watches. No tool changes jobs, pause state, saves, or the filesystem.

## Binary payload contract

`Reply.observation` uses big-endian scalars; native RPC framing remains DFHack's
little-endian transport. Prefix: eight ASCII bytes `DFMJ1200`, u32 year, u32 year
tick, canonical boolean pause byte, i32 site, u32 next-job-ID, u16-length UTF-8
world folder, u32 job count. Each record is:

```text
u32 native_id; i32 job_type
u16-length UTF-8 type_key; u16-length UTF-8 reaction
bool suspended; bool repeating
i32 x; i32 y; i32 z
bool worker_present; [u32 worker_id]
bool holder_present; [u32 holder_id]
i32 completion_timer; u32 attached_items; u32 requirement_filters
```

Booleans are exactly 0/1. The complete payload is bounded to 2 MiB and 4096 jobs;
trailing bytes and every truncated record are rejected. The authenticated outer
reply supplies the generation and version manifest. Payload bytes plus that
manifest are committed by the source digest.

## Validation evidence

Run the retained native-source test without a game installation:

```bash
python3 scripts/test_live_jobs_native_mock.py
CXX=clang++ python3 scripts/test_live_jobs_native_mock.py
```

Both GCC and Clang compiled the exact checked-in native source using C++17,
`-Wall -Wextra -Werror`, and **mock** DFHack/protobuf types. Each run passed 95
assertions and produced the same 153-byte payload, independently reproduced by a
Python encoder. The source SHA-256 for those runs was
`0458b5548e5bb9891083d29f192bccd3be510107722795f96257855d0c3a7960`.

The golden payload is retained in `crates/dfmcp-adapter/tests/fixtures/jobs_v1_2.hex`.
Nineteen added Rust tests cover payload decoding, every golden truncation, invalid
publication, generation reuse, resets, RPC negotiation, poisoned-stream no-retry,
real job projection into typed queries, and MCP baseline/watch workflows.
**Those Rust tests have not been executed here.**

Mock compilation verifies this source against the supplied mock interfaces; it
does not verify the real generated DF types, actual protobuf generation/linking,
DFHack loader, native RPC suspension, game behavior, or production admission.
A real DFHack build and disposable-fort campaign remain required.
