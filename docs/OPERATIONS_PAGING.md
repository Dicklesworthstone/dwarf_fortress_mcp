# Immutable native operations snapshots: profile 1.4

The explicitly unadmitted operations/1.4 development profile acquires one coherent
native jobs/buildings/items/attachments snapshot, retains its serialized bytes,
and transfers those bytes in bounded pages. It supports a larger world than the
existing operations/1.3 single-frame profile without concatenating observations
from different game ticks.

This is source integration, not a claim that the Rust runtime or real DFHack
plugin has passed qualification. The existing native 1.0, 1.1, 1.2 and 1.3 sources,
production admission map, dependency pins and compatibility registry are unchanged.

## Components and entry

- `bridge/common/retained_snapshot.h`: immutable byte cache and whole-payload SHA-256.
- `bridge/dfhack-operations-v1_4/`: separate plugin, protobuf envelope and CMake target.
- `dfmcp_adapter::live_jobs_rpc::operations::paged`: fixed-profile bounded client.
- `dfmcp_adapter::live_operations::OperationsProfile::PagedV1_4`: explicit codec and projection identity.
- `dfmcp_mcp::live_operations_server::paged`: separately gated bootstrap using the existing operations handlers.
- Cargo binary: `dfmcp-live-operations-paged-dev-server`.

Build `dfmcp_operations_v1_4` inside a named DFHack checkout using the supplied
CMake subdirectory. Preserve `bridge/common` as a sibling of the native profile
directory; do not copy the producer alone. This requires real DFHack-generated
headers and protobuf generation/linking, neither established by the mock test.

Configure the same operator-chosen 32..256-byte `DFMCP_OPERATIONS_PAGED_TOKEN` in
DFHack and the MCP process. The token is never an MCP argument. With the token
already in the environment, the development entry is:

```bash
DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_4=1 \
DFMCP_OPERATIONS_PAGED_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-operations-paged-dev-server
```

The public library entry repeats the opt-in check. Other `DFMCP_*` settings,
including old-profile credentials, all admission state, and journal/repair paths,
are rejected. There is no client-selectable protocol, method, plugin, path or
native address. Only Observe, Query and Doctor grants can be issued.

## Bounds

| Dimension | Bound |
|---|---:|
| Jobs per captured world | 4,096 |
| Buildings per captured world | 4,096 |
| Items per captured world | 65,536 |
| Job-item attachments | 65,536 |
| Complete serialized snapshot | 16 MiB |
| Transfer page | 16..256 KiB; default 64 KiB |
| Native pages per acquisition | At most 1,024 |
| Native cache | At most four captures and 32 MiB retained payload bytes |
| Capture validity | Fixed 120 seconds after capture completion |
| Paged MCP sessions | Two per process, also subject to the shared eight-session limit |
| Whole acquisition deadline | 1..60,000 milliseconds; default 5,000 |

Default acquisition limits use the hard roster/payload ceilings. The agent can
request smaller bounds when opening a session. Response-token budgets remain
independent of native page size and default to 8,192 tokens. Existing query,
allocation, graph, watch and baseline bounds are not widened by native paging.

The cache reserves capacity for the requested maximum before serializing. It
retains only bytes and metadata, never native pointers. The 32 MiB number is
retained payload storage, not a bound on total process memory: encoding buffers,
native lookup maps, Rust records and the canonical graph require additional
bounded storage. Real memory use and pause/latency behavior are not benchmarked.

Capture validity is checked lazily on cache operations; no background expiration
worker is introduced. Expired bytes may remain allocated until a subsequent
cache operation, world reset or shutdown, always within the retention ceiling.
MCP sessions currently last until process shutdown; there is no new close-session
operation. Restart is needed after retained session capacity is exhausted.

## Capture, page, verify, release, publish

The native package is `dfmcp.operations.v1_4` and the plugin remains a two-method
service: `Handshake` and `ReadObservation`, both registered with RPC flags zero.

An empty capture token and zero offset request one complete native capture under
DFHack suspension. That first call traverses, validates and serializes the entire
bounded world. Later calls with the returned token read only retained immutable
bytes, not the game. Paging reduces transfer-frame size; it does not make the
initial native capture incremental or eliminate its suspension cost.

Each page carries token, nonce, source generation, DF/DFHack versions, offset,
total byte count, SHA-256 and terminal status. Native token use is bound to the
request nonce, source generation and acquisition limits. Authentication is repeated
on every request. Tokens identify retained reads; they do not grant authority.
Page retries, including the last page, return the same bytes until release or
expiry. A missing/expired token is an error, never an implicit new capture.
World load/unload clears captures and advances the native generation.

The Rust assembler requires exact progress, full nonterminal pages, immutable
manifest fields, bounded allocation, and terminal coverage equal to the announced
total. It verifies SHA-256 over the complete bytes before semantic decoding.
The decoded world must still satisfy count, type, ordering, endpoint, attachment
and containment invariants. A complete byte stream cannot waive those checks.

The client explicitly releases the capture and validates the acknowledgement
before returning the observation. All page reads and release share one absolute
TCP deadline, including fragmented I/O and bounded text notifications. Negotiation
has its own deadline; it is not renewed per page. A failed acquisition fences the
connection and preserves the previously published world. A failed connection may
leave native bytes retained until expiry; it does not trigger unsafe retries on
a desynchronized stream. The new client does not automatically resume after a
connection failure even though native page reads themselves are repeatable.

The snapshot tick is the capture tick, not transfer-completion time. Normal game
simulation may continue between page calls. Returning a coherent snapshot does
not prove the game is still in that state when the agent receives it.

## Existing agent workflows

The new bootstrap registers the existing eleven tool names and reuses the ten
post-bootstrap operations handlers. Entity inspection/filtering, aggregates,
search, graph traversal, production diagnosis, declared inventory allocation,
baselines and foreground condition watches run on the fully published 1.4 graph.
A logical observe/await call performs one capture, potentially many page RPCs,
and one release. Terminal watch retries still skip acquisition.

The selected codec profile is fixed in session construction. Default observation
methods remain operations/1.3 with their existing 32,768-item/2-MiB limits and
`DFMO1300` magic. The new codec requires `DFMO1400` and distinct source/fact
provenance even for a small otherwise identical roster. Semantic entity and edge
keys are shared where their meanings are unchanged; full anchors bind the profile.

Operations schema discovery exposes the eighteen nonhistorical variants for 1.4.
The existing 1.3 entry continues to expose its twenty variants, including history.
The durable archive currently replays 1.3 records only. The paged entry rejects
journal configuration and historical queries instead of silently interpreting
old records as new-profile evidence. Paged observations, watches and baselines
remain process-local. Native snapshot tokens are not archive record identities.

## Validation performed and still required

`python3 scripts/test_retained_snapshot_native.py --compiler g++` and the same
command with `--compiler clang++` compile the actual producer and cache header
against mock interfaces using C++17, `-Wall -Wextra -Werror -pedantic -O2`.
Each run passed 748 checks. A 40,000-item, 2,200,070-byte capture was reassembled
from 135 pages after simulated game changes and matched an independent Python
encoder byte-for-byte. The exact 65,536-item ceiling, over-limit rejection,
owner/limit/token failures, release, world invalidation, cache quota/expiry, and
11 independent SHA-256 vectors were also exercised.

Exact tested producer SHA-256:
`568c56b13a84fb87e9c47affc5156045ceaf29c10d486133a8c370df1e7623e0`.
Exact tested cache header SHA-256:
`4c822e308b0b700df10e38162bfc5e8bdc96fff6956b350a6bc405f3a7bcb887`.
The 40,000-item golden payload digest is
`6fb2e8c93943abc31f4b66c496c4aeabf95ab0ac312596068344950c28f3ef09`.

Fifteen Rust scenarios are registered: five assembler tests, five actual-client
wire tests, three codec/profile tests, and two MCP integration/isolation tests.
They include fragmented native I/O, corrupt/mixed pages, lost release, cross-profile
rejection, 40,000-item queries with an 8,192-byte response budget, baseline/watch
refresh, source-digest propagation and prior-anchor preservation on failure.
They have not been compiled or executed: Rust, Cargo and rustfmt were unavailable.
No Clippy, stdio, real generated DF header build, protobuf linking, plugin loading,
live-game campaign, full repository qualification or production admission is claimed.

Still absent: paging beyond these bounded roster/payload ceilings, incremental
native capture, 1.4 durable archive integration, coherent citizen/announcement or
map coverage, full native requirement/path feasibility, and live game mutations.
