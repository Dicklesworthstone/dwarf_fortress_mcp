# Reviewed finite clock intervals for a complete original room

`dfmcp_adapter::room_condition::clock` connects the original joint room monitor
to the existing run/1.13 coordinator. It supports separately reviewed finite
intervals while preserving every original floor, wall and furnishing condition.
A run can advance the game; only an ordinary joint observation can establish
sampled room completion. No background loop or automatic interval chaining is
introduced.

The owner shares the native run journal, authority checks, one-attempt dispatch
and recovery engine with the [furniture clock](FURNITURE_CONSTRUCTION_CLOCK.md).
Its public wrapper, goal association and identity domains are separate. Existing
furniture clocks retain their exact encodings, keys, review identities and behavior.
See [the joint monitor](ROOM_CONDITION_RUST.md) for the complete room condition.

## Bind the original room and its first accepted sources

The clock retains the complete immutable `room_condition::Definition`, including
its original room-backed batch, full placement receipt set, private-file identities
and unchanged timing policy. It also retains the first accepted `SourceIdentity`:
furniture generation, operations/1.4 manifest and map/1.5 manifest. The three source
generations remain independent; none is replaced by the run generation.

The target digest covers the full definition, all retained source identities and
the independent run journal incarnation. Each run key and process-local review
seal belongs to that exact target. Changing terrain, furnishings, receipt identity,
timing, map incarnation or operations incarnation cannot adopt an earlier clock
history. An ordinary furniture monitor cannot be promoted into the room objective.

The runtime supplies a verified first accepted joint source. Creating a clock
from a Query-only bootstrap must not manufacture Plan or ControlClock grants.
A new owner captures its current opening authority; later calls must satisfy both
that retained authority and the current request's authority.

## Check the whole room at preparation and commit

`prepare_guarded` synchronizes run intent before its host guard. `commit_guarded`
consumes the current review seal and synchronizes DispatchStarted before its guard.
Each guard independently verifies original custody and obtains one complete
receipt/map/operations/map/receipt bracket through the existing query-only reader.
Both complete map endpoints must match and be paused at the operations and
selected native run tick. The guard acknowledges the raw sample as readiness in
the original room journal before returning `RoomClockSample { progress, sample }`.

The clock rechecks that evidence with a pure joint probe. Every original furnishing
must remain either viable `pending` construction or `condition_met`. Hidden or
missing required terrain, unpaused captures, changed source/identity, removal,
regression, exhausted acquisition allowance or expired goal refuses a new interval.
A clock review never selects a successful subset of the room.

**Known unmet terrain can remain pending during an explicitly reviewed interval.**
The complete counts and bounded deficit list remain visible. This permits a room
whose furnishings are complete but whose declared terrain is not yet complete to
retain its original goal. It does not prove that excavation, wall construction,
hauling or another causal process exists. Global clock control has its own explicit
authority; the room condition supplies a bounded objective and observed blockers.

## Readiness consumes observations without earning completion

`room_condition::readiness::advance` evaluates the same full condition as an
ordinary goal sample. `MonitorStore::publish_readiness` writes new frame kind 5
containing the complete canonical sample. The DFMRCJ01 header and earlier frame
encodings remain unchanged; older readers reject the unknown new frame rather
than reinterpret it as positive evidence.

Every completed readiness acquisition consumes one of the original
`max_observations`, so preparation and commit normally consume two acquisitions
per interval. It never creates `first_tick`, advances `counted_tick`, increments
the shared streak or independently satisfies the room goal. The clock's pure probe
rechecks pre- or post-acknowledgement progress without charging a second acquisition.

A fully qualifying joint readiness sample can preserve an existing candidate.
Known terrain deficits, pending furniture, unknowns, unpaused evidence, changed
same-tick captures and excessive gaps withdraw unfinished stability. Source and
stage regressions, removal, deadline and observation exhaustion retain their
terminal meanings. These results survive journal replay. A readiness capture
that consumes the final allowance expires the goal before unpause.

An interrupted read or failed enclosing publication cannot preserve positive
continuity. Reopening restores original limits and evidence floors, but no live
read permit, review seal or downtime stability. Only a later ordinary joint sample
can earn positive completion credit.

## Review, dispatch and recovery

Each interval requires a current process-local review seal and explicit confirmation.
The seal is consumed before dispatch. Failed publication, abandonment, reconciliation
and reopening cannot restore it. Once DispatchStarted is synchronized, an ambiguous
reply is retained as uncertainty and prevents another unpause attempt for that key.

Reconciliation queries the exact original key and digest. Missing native history
remains unknown and blocks new advancement and completion sampling. Only original
verified outcomes can settle the retained run. Recovery requires the acknowledged
`{journal_id, head}` and accepts only a complete validated descendant; rollback,
substituted definitions or sources, and unrelated histories are refused.

Cancellation before dispatch retires local preparation. After dispatch it can ask
for the original native safety pause. This requires retained and current named
fortress Query/ControlClock plus separately issued native Query/ControlClock. It
requires no new future ticks, Plan/Observe capability or unexpired goal deadline.
Cancellation does not complete the room, undo placements or prove that the game is
currently paused. Full room proof remains an independent sampled obligation.

## Authority and bounded work

New advancement requires named-fortress Query, Observe, Plan and fortress-wide
ControlClock. The native run source separately requires NIL-scoped Query, Plan
and ControlClock. A child context cannot recreate removed grants. Both original
opening and current grants, expiry, session/request identity and the minimum
negotiated game-time budget are checked at the relevant boundaries.

Decoded map, operations and run clocks raise a conservative authority floor before
later identity or semantic rejection. An older caller tick cannot restore expired
authority. Durable raw readiness and native records preserve their floors on replay;
a floor learned from an unjournaled failed read remains process-local until a later
authorized persistence boundary. It is never positive condition evidence.

| Bound | Limit |
| --- | --- |
| Original room | One level, up to 4,096 cells and 1..32 furnishings |
| One requested interval | 1..1,200 game ticks and 1..60,000 wall milliseconds |
| Horizon | Original deadline, maximum sample gap and retained/current tick allowance |
| Retained run intents | `min(64, original max_observations)`; no eviction |
| Clock inspection reservation | 8 MiB |
| Joint core readiness reservation | 192 MiB |
| Complete room clock core operation | 256 MiB |

The host separately reserves original-file checks, complete native acquisition,
monitor journal publication and output. One shrinking foreground wall allowance
covers the entire operation; nested calls do not renew it. Native stop limits are
cooperative triggers, so observed overshoot remains visible and narrows later
requests.

## Proof and qualification limits

Equal paused endpoints provide bounded bracket evidence. They do not prove an
atomic cross-plugin snapshot, common process identity, run/1.13 named-fortress
lineage, global controller exclusion or construction causality. Historical joint
satisfaction does not establish current usability, native room assignment,
pathfinding, access or effect discharge.

This adds no native method, wire generation, dependency, admitted compatibility
entry or production runner. Component execution and exact qualification limits
are recorded in `IMPLEMENTATION_STATUS.md`; a real native SDK, live fortress and
full FastMCP runtime require their own evidence.
