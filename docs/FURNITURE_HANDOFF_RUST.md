# Inventory allocation retained through furniture placement

`dfmcp_adapter::furniture_handoff` connects requested furniture slots to an
immutable exact placement batch. Callers can request beds, chairs and tables
with material, subtype, distance, exclusion and dependency constraints. One
complete operations/1.4 observation feeds the existing global allocator. Its
chosen items and original constraints survive placement and restart.

## Request and allocation

`FurnitureRequest::decode` accepts the existing `dfmcp.furniture-request/1`
JSON format, bounded at 16 KiB and 1..32 slots. The schema-specific parser
rejects unknown or duplicate fields, malformed numbers, invalid Unicode,
duplicate targets or exclusions, and invalid dependency graphs. Normalized
ASCII JSON and its domain-separated digest follow the Python allocator.

Supply uses the existing direct-ground, unattached, singleton furniture policy.
The allocator minimizes total same-level Manhattan distance, then the item-ID
vector in lexical slot order. It assigns distinct items to every slot, or
returns a complete shortage witness with no partial executable plan.

`Handoff::allocate` accepts a published `LiveOperationsState` in the paged
operations/1.4 profile and the matching operation context. It derives its own
supply report, rather than accepting a client allocation report or selected
item projection. The full roster must have consistent native type numbers
and semantic keys. Entity, byte, work and wall limits cover the complete result.

## Trusted observation

`furniture_handoff::rpc::acquire_trusted` owns one foreground TCP connection to
the operator's numeric IPv4 loopback endpoint. It binds only the existing
operations/1.4 `Handshake` and `ReadObservation` methods. The common pager
checks every page against the pinned manifest, verifies the complete capture
digest, strictly decodes it and verifies release before publication.

The connection has one shrinking deadline, 20 MiB of network allowance,
bounded notifications and method calls, and current cancellation/operator
checks during I/O. The entity count includes the fortress root, all jobs,
buildings and items: at most 73,729. Query authority is rechecked at the
captured game tick before release and local projection. There is no automatic
reconnect or repeated read.

The pure handoff constructor does not authenticate an endpoint by itself.
An effect-owning host must acquire the source through the trusted boundary
and retain current authority through allocation and durable publication.

## Original evidence and placement checks

The handoff retains the normalized request, endpoint, fortress, software,
operations generation, canonical source anchor, capture and source digests,
native ID horizons, and every chosen item's original entity handle, native
type, material, subtype, position and allocation distance. The exact
`FurniturePlan` is derived from these retained bytes.

`validate_binding` compares endpoint, fortress and software with the furniture
source. Operations/1.4 and furniture/1.19 generation numbers have separate
namespaces and are never substituted for one another. `validate_capture`
checks the existing exact furniture binding and selection, rejects clocks or
building/job ID horizons predating allocation, and requires the selected item's
original type, material and subtype. The item may move while remaining on the
requested level and within the original distance bound.

`BatchDefinition::from_handoff` uses the new `DFMFBD02` representation and a
separate batch digest domain. Its audit and next-step validation enforce the
retained constraints on every child preparation. The legacy constructor still
emits byte-for-byte `DFMFBD01` definitions and original step keys.
The new handoff is at most 24 KiB; the complete definition is at most 42,036
bytes and fits the existing 64 KiB parent store. Existing placement journal
bytes, parent stop frames and completion origin limits are unchanged.

The batch never reallocates automatically after restart. Source custody,
current native eligibility, host lease, explicit review, checkpoint policy and
the original preparation connection remain required for placement. The
historical inventory does not reserve items, establish paths, or prove future
construction completion.

## Evidence limits for this increment

The source and 33 new Rust test groups are present. Those groups have not
compiled or executed. Four attempts to build the furniture MCP tests were
killed while compiling the pinned Asupersync dependency, and the execution
backend disconnected before the adapter-only build could start.

The source was preserved through GitHub from the successful patch context
after that outage. Formatting and syntax checks performed before the outage
do not certify the subsequently reconstructed bytes. The next required check
is a pinned-toolchain adapter build followed by the handoff, batch, placement,
construction and paged-RPC regression filters.

This increment does not change a native wire protocol, admit a live tuple,
or widen the production protocol runner map. Earlier completion test evidence
does not qualify this later adapter source.
