# Joint original-room conditions in Rust

`dfmcp_adapter::room_condition` monitors the complete declared room terrain and
every original furnishing together. One accepted sample must establish all
conditions before it can advance their single shared stability streak. Historical
placement registration or completed furniture alone cannot establish this result.

This is an unadmitted development adapter, private journal and query-only native
reader. The [existing MCP intake](ROOM_CONSTRUCTION_MCP.md) already retains a room
request through allocation and placement. Public joint-monitor startup, sampling
and recovery routing are a separate integration step, outside this core checkpoint.
Owning beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` remain in progress.

## Original intent and definition

`room_intent::RoomIntent` binds the complete furniture request to explicit floor
and wall masks in one declared, single-level region. Every furniture target is
a required floor. Unselected cells impose no terrain requirement. This narrow
Rust format is independent of Python `RoomPlan`: it neither compiles a layout
nor imports that format's circulation, access or routing claims.

The room-backed `DFMFBD03` batch retains the unchanged allocation handoff and
canonical terrain. After every original step has a resolved `Placed` receipt,
`construction_plan::origin::Origin` retains the complete batch, all receipts and
the original parent/placement file and directory identities. `MonitorDefinition`
adds the fixed receipt goal and timing policy. `room_condition::Definition::new`
embeds that complete definition and refuses roomless origins. Neither an older
furniture-only batch nor a successful subset can acquire replacement room intent.

`Definition` exposes `original`, `origin`, `goal`, `room`, `id`, `canonical_bytes`
and `decode`. The absolute deadline, cadence, stability count/span, maximum gap
and completed-acquisition allowance remain sealed through replay and reopening.

## Acquire one complete joint sample

`construction_plan::rpc::room::acquire_trusted_with_floor` owns one bounded TCP
connection to the original numeric IPv4 loopback endpoint. It binds only these
existing methods; no native wire generation changes:

| Profile | Fixed methods |
| --- | --- |
| furniture/1.19 | `Handshake`, `QueryPlacement` |
| operations/1.4 | `Handshake`, `ReadObservation` |
| map/1.5 | `Handshake`, `ReadObservation` |

After the handshakes, acquisition queries every original receipt, reads map-before,
acquires and releases one complete paged operations roster, reads map-after, then
queries every original receipt again. Release acknowledgement, full-capture digest,
page identity and exact receipt bytes retain their existing validation. There is
no reconnect, retry loop, effect method, background worker or game-time advance.

The returned `Capture` supplies the fields of `room_condition::Sample`: `before`
and `after` map manifests, both raw map payloads, and `furnishings: LinkedSample`.
Map endpoints must have identical complete bytes and manifests, be paused, cover
the exact original region, and share the operations capture's paused game tick.
Fortress identity, dimensions and software must match the original placement
binding. Furniture, operations and map generations retain separate namespaces.
Equal paused endpoints establish a bounded bracket, not an atomic cross-profile
snapshot or shared-process identity. A serialized sample alone does not attest
that the reader contacted a native peer.

## Conditions and temporal meaning

Every selected floor must be visible, dry, without magma or a dig designation,
and have the exact floor shape. Every selected wall must additionally have wall
shape and no observed building or unit occupancy. Every furnishing target needs
visible building occupancy; its exact identity comes from the receipt-linked
operations roster, not the map occupancy tag.

The unchanged [construction condition](FURNITURE_COMPLETION_RUST.md) checks every
original building footprint, maximum stage, exact installed singleton item and
material, and absence of construction/removal jobs or conflicting attachments.
`TerrainAssessment` retains complete category counts and up to 64 ordered deficit
rows. Hidden or unallocated required cells remain unknown. Known shape, liquid,
designation or occupancy defects remain unmet conditions. Both clear unfinished
stability; they never become a successful subset or global absence claim.

The shared construction reducer evaluates furniture and terrain before awarding
any positive sample. Alternating successful room and furniture observations cannot
combine. Repeated ticks earn no credit. A changed complete joint sample at the same
tick, excessive gap, interruption or reopen resets unfinished stability. Even a
change in unselected map padding affects this continuity digest. Unpaused evidence
cannot earn positive credit; the native reader refuses it before publication.

Changed source identity/software/world/dimensions, native ID horizons, regressing
clock or construction stage invalidate the goal. Removal fails it. The original
deadline and completed-acquisition limit expire it. Malformed or incomplete
brackets are refused rather than acknowledged as samples. `satisfied`, `failed`,
`invalidated`, `expired` and `cancelled` are terminal historical outcomes.

## Durable owner and recovery

`store::open_private_monitor(path, context, mode, expected)` uses existing Linux
private-file custody: no-follow opens, exact private modes, exclusive locking,
file/directory identity checks, synchronized appends and complete byte readback.
An explicit `Definition` permits creation with Query authority in Control or
Recover mode. Offline mode only opens existing history and never appends.

| Object or frame | Encoding / meaning |
| --- | --- |
| Joint definition | `DFMRCG01`, complete original `MonitorDefinition` |
| Joint sample | `DFMRCS01`, both map endpoints plus complete `LinkedSample` |
| Journal | `DFMRCJ01`, sequence/previous-head hash chain |
| Frame 1 | Immutable joint definition |
| Frame 2 | Read intent plus big-endian `u64` monotonic authority floor |
| Frame 3 | Complete raw joint sample; replay recomputes all conditions |
| Frame 4 | Local cancellation plus big-endian `u64` authority floor |

The host reserves output and original-source work separately, then calls
`begin_read` before native contact. Only that live owner may `publish_sample`.
Admission and original-origin callbacks surround publication, including a source
check after synchronization. Failures consume publication permission and withdraw
unfinished continuity; ambiguous writes fence the owner. The next read records
an interrupted-read reset. Cancellation requires current Query and intact monitor
custody, but no original-file guard. It stops only this monitor.

`Recovery { monitor_id, head }` must be retained independently. `initial_recovery`
provides the exact initial pin before creation. `verify_recovery` accepts a fully
replayed descendant, including a complete lost acknowledgement; it rejects rollback,
unrelated or divergent history. Writable reopening re-synchronizes surviving
frames. No truncation, repair, rebinding or restored read permit occurs.
`first_source` preserves the first accepted furniture/operations/map association
even when later diagnostic evidence invalidates the monitor. Hashes and private
custody are not signatures or protection against a complete operator rewrite.

## Failed reads still carry their clock

`acquire_trusted_with_floor` updates its caller's floor on success and failure.
The host must retain it in its request/session context and call
`MonitorStore::retain_observed_tick_floor` before propagating an acquisition error.
`Sample::retain_observed_tick` similarly latches each decoded component before a
later component can fail. The latch grants no evidence, source identity, streak
or read permission. The next authorized read intent or local cancellation persists
the floor; until then the caller must preserve the failure handoff across restart.
A lower later capture cannot earn credit. Authority expiry is checked at the
highest learned floor, not a stale caller tick.

## Bounds and evidence scope

The original region has at most 4,096 cells on one level, with at most 128 cells
per axis and 1..32 furnishings. Each retained map payload is bounded at 82,944
bytes; the operations capture at 16 MiB. Both map endpoints and the complete
operations roster count against the entity allowance. One reader invocation has
at most 332 RPC calls, 22 MiB wire bytes, 2 MiB notifications and 60 seconds.

The journal retains at most 128 MiB and 1,030 frames; the original goal permits
at most 512 completed samples. Replay shares 20,000,000 semantic work units and
one shrinking wall/byte allowance. `operation_reserve` accounts for monitor work;
native acquisition, source guards and output use separate prepaid portions of the
same host request. Store work is capped at 2 GiB; this is accounting, not eager
allocation. Synchronous work has cooperative bounds, not hard real-time preemption.

Focused tests cover 32 targets, alternating truth, wall loss, unknown terrain,
same-tick changes, budgets, native brackets, failed-read clock floors, corruption,
ancestry and real private-file recovery. Exact executed commands and counts belong
to [implementation status](../IMPLEMENTATION_STATUS.md). No native SDK/live fortress,
full-workspace qualification or production admission follows from these tests.
Historical sampled satisfaction proves neither a current usable room nor native
room assignment, circulation, pathfinding, structural safety, continuous wall
preservation, causality or discharge of any original placement effect.
