# Joint original-room completion through MCP

`dfmcp-build-placement-dev-server` monitors the complete declared room terrain and
every original furnishing through the existing tools and [Rust monitor](ROOM_CONDITION_RUST.md).
One complete acquisition must meet both sets of conditions to advance their single
shared stability streak. Satisfaction is historical evidence under a fixed policy.

This unadmitted development composition uses furniture/1.19, operations/1.4 and
map/1.5 without adding a mutation method. See [implementation status](../IMPLEMENTATION_STATUS.md)
for exact executed evidence and remaining runtime/native/live qualification limits.

## Start with the complete original room-backed batch

Create the room through [room construction intake](ROOM_CONSTRUCTION_MCP.md),
then finish the original [furniture batch](FURNITURE_BATCH_MCP.md). Every step needs
a synchronized, resolved `Placed` receipt. The monitor retains the full `DFMFBD03`
batch, original file identities and receipt mapping; any unresolved, missing,
refused or substituted step prevents creation.

The original request declares one level with at most 4,096 cells, exact floor
and wall masks, and 1..32 furnishings on required floors. Unselected cells make
no terrain claim. Older furniture-only batches cannot acquire geometry later;
reopening cannot select a successful subset or replace the room request.

## Configure independent monitor custody and map access

Keep the original [placement configuration](BUILD_PLACEMENT_MCP.md), including
`DFMCP_BUILD_JOURNAL`, `DFMCP_BUILD_BATCH`, fortress identity, endpoint and scope.
Configure the room journal path before opening the session; supply the map
credential when sampling:

```sh
export DFMCP_BUILD_ROOM_COMPLETION=/private/furniture/room-completion
export DFMCP_MAP_TOKEN='<operator-provided map/1.5 credential, 32..256 bytes>'
```

The path is normalized and absolute, distinct from the placement, parent,
ordinary completion and optional clock files. Use real owned `0700` directories
and private `0600` files. Opening checks locks, current file/directory custody,
sealed original identities and complete bytes; it never repairs damaged history.

Explicit sampling also needs the original `DFMCP_BUILD_TOKEN` and independent
`DFMCP_OPERATIONS_PAGED_TOKEN`. All three credentials are 32..256 bytes. Local start,
inspection, cancellation and reopening need no native connection. Credentials,
paths, source selectors and policy exceptions are operator settings, not MCP arguments.

The server's closed environment now contains **24 allowed `DFMCP_` names**.
The three room additions are `DFMCP_BUILD_ROOM_COMPLETION`,
`DFMCP_BUILD_ROOM_COMPLETION_RECOVERY` and `DFMCP_MAP_TOKEN`; the remaining settings
are documented in the placement, [furniture clock](FURNITURE_CONSTRUCTION_CLOCK_MCP.md)
and [room clock](ROOM_CONSTRUCTION_CLOCK_MCP.md) guides. The room clock adds its
independent configuration, token and opt-in names.
Unknown `DFMCP_` names, including native-plugin opt-ins and production admission
settings, are refused. Native process configuration remains separate.

For a new monitor, its path must be absent and no recovery pin may be supplied.
For **every reopen of an existing room journal**, retain the returned
`result.room_completion.recovery` and configure that exact acknowledged identity:

```sh
export DFMCP_BUILD_ROOM_COMPLETION_RECOVERY='{"monitor_id":"<original room monitor SHA-256>","head":"<acknowledged room journal SHA-256>"}'
```

Both digests must be nonzero lowercase 64-character hexadecimal values. The closed
object contains exactly `monitor_id` and `head`. A pin requires the room path;
an existing file requires its room pin, distinct from ordinary monitor/clock pins.

## Seal the goal, then acquire explicit samples

Use the completed batch's session if its room journal path was already configured
when that session opened. Session configuration is immutable: adding or changing
the room path later requires releasing/restarting and reopening the same original
custody with the updated operator configuration. Reopen with
`fortress.open_session()` and omit `selection`, `furniture_plan`, `furniture_request`
and `room_request`. Existing room journals additionally require their retained
recovery pin. `fortress.query` takes a JSON **string**. Create the fixed joint goal
with:

```json
{"mode":"room_completion_start","deadline":403200,"interval":120,"stable_samples":3,"stable_span":240,"max_gap":1200,"max_observations":128}
```

The illustrative deadline must leave enough time after the original receipts for
the entire policy. Creation performs local custody checks and no native acquisition.

| Field | Meaning and accepted range | Default |
| --- | --- | --- |
| `deadline` | Required absolute game tick; the policy must fit before it | None |
| `interval` | Minimum advancing-tick sampling interval, 1..403200 | 1 |
| `stable_samples` | Required shared qualifying samples, 2..64 | 2 |
| `stable_span` | Required span in game ticks, 1..4032000 | 1 |
| `max_gap` | Maximum gap in game ticks, `interval`..4032000 | 1200 |
| `max_observations` | Original completed-acquisition limit, `stable_samples`..512 | 512 |

The closed 2 KiB query rejects duplicate/unknown fields, nonintegral numbers and
null optionals; query `{"mode":"schema"}` for discovery. Repeating the identical
goal is idempotent; different timing or intent cannot replace or renew it.
Control and normal Recover mode can create Query-only custody; Offline cannot.

Inspect locally or request one complete foreground acquisition:

```text
fortress.query(session_id, query='{"mode":"room_completion"}')
fortress.observe(session_id, selection="room_completion")
fortress.wait(session_id, scope="room_completion", max_wall_millis=5000)
```

The two acquisition forms have the same meaning: **one sample**, not a polling
loop. Omit effect keys, plan digests and `monitor_id` for room `wait`. An already
terminal monitor returns locally. The default `wait` scope remains original
placement-outcome reconciliation. These room calls clear local placement and
clock review permission; they never commit, cancel or retry native work.

Durable read intent precedes contact. One connection binds only Handshake and
QueryPlacement for furniture, and Handshake and ReadObservation for operations
and map. It queries every original receipt, reads map-before, acquires/releases
one complete operations capture, reads map-after, then queries every receipt
again. Both paused map endpoints must have identical complete bytes and manifests
at the paused operations tick. Exact original region, fortress, dimensions and
software must agree. The three profile generations remain independent.

Every selected floor must be visible, dry, undesignated and floor-shaped. Walls
must be visible, dry, undesignated, wall-shaped and free of observed building/unit
occupancy. Every furnishing target also needs building occupancy and the exact
receipt-linked completed construction condition. Hidden or unallocated cells are
unknown; terrain defects and pending furniture clear unfinished stability.
Furniture success at one time and terrain success at another cannot combine.

## Read the whole result

`result.batch.room` retains the complete request, region and compact floor/wall
masks. `result.room_completion` uses schema `dfmcp.room-completion-mcp/1` and includes
the room/monitor identities, fixed timing, every original furnishing assessment,
complete terrain category counts and up to 64 ordered deficits. `deficits_complete`
distinguishes a full list from its bounded prefix; no furnishing is dropped.

The projection includes observations, one streak, first/last/counted ticks,
capture digest, journal head/size, recovery pin and `authority_tick_floor`;
`source` associates independent furniture, operations and map generations.
`fresh_joint_sample_acquired` distinguishes a newly accepted acquisition from
local historical inspection; it is not a completion claim.

Phases are `active`, `candidate`, `satisfied`, `failed`, `invalidated`, `expired`
and `cancelled`. An unverified projection reports `phase="unverified"` with its
historical phase retained. `monitor_inventory_verified` and `monitor_terminal`
describe local monitor custody independently of `origin_verified` and
`sampled_condition_satisfied`. The common Agent Turn retains the complete result,
outstanding work and next action; it does not invent a canonical world anchor.

Repeated ticks earn no credit. Same-tick changes anywhere in the full joint
capture, excessive gaps, interrupted reads and reopening reset unfinished
stability. Identity/source or clock/stage regressions invalidate the goal;
removal fails it; the fixed deadline or acquisition limit expires it. Terminal
outcomes, including satisfaction, are historical and cannot be renewed.

## Game time and the separately configured construction clock

Room monitoring never changes game time or automatically polls/reconnects.
Ordinary gameplay or a separately authorized controller must supply advancing ticks.

An optional [furniture construction clock](FURNITURE_CONSTRUCTION_CLOCK_MCP.md)
continues to belong to its **ordinary furniture completion monitor**. It requires
that monitor's first accepted sample, independent run-source configuration and
separate preparation/review/commit authority. A joint room goal cannot replace
that clock goal, and furniture readiness cannot prove terrain readiness.
`construction_clock_goal_promoted=false` makes this boundary explicit.

A separate [original-room clock](ROOM_CONSTRUCTION_CLOCK_MCP.md) now supports
explicit finite intervals bound to the joint goal and its first accepted sources.
Both preparation and commit acquire full joint readiness, charge the original
observation allowance and earn no positive completion credit. A new absent clock
may permit one first-source sample; explicit room-clock planning must then open
its exact custody before another sample. Each interval requires its own review
and commit; the ordinary monitor alone still never changes game time.

Prepared, pending, unresolved, missing or unverified configured clock custody
in either configured clock blocks fresh room sampling. Reconcile or safely
cancel that original clock, then
request another room sample. Keep its configuration and all recovery references
on combined-session reopen: omitting optional clock configuration does not prove
that no earlier clock exists. The room monitor neither settles nor discharges it.

## Lost replies, restart, cancellation and closing

Retain recovery references from successes **and failures**. Reopening verifies
the room definition and acknowledged ancestry. A validated descendant can recover
a lost response; rollback, divergence, substituted original files and torn tails
are refused. Writable reopening re-synchronizes complete surviving frames and
withdraws unfinished continuity without restoring a read or commit permit.
The original deadline, acquisition allowance and first accepted source stay fixed.

Even a failed bracket can reveal a higher game tick. That floor governs authority;
an older caller tick cannot revive expired Query. Retain its failure handoff when
disclosure is authorized. The next authorized read intent or local cancellation
persists the floor. Until then it is process-local, not recoverable from older
journal bytes. Preserve the handoff and original files.

Normal `DFMCP_BUILD_MODE=recover` reopens the same original placement, parent and
room histories and permits explicit Query-only sampling with valid credentials.
Normal `offline` mode inspects without native calls or writes; it refuses active
sampling and active cancellation. Neither mode renews the original goal.
Room configuration is refused in `completion-recover`, `completion-offline`,
`clock-recover` and `clock-offline`; those modes own different monitor formats.

```text
fortress.cancel(session_id, scope="room_completion")
fortress.cancel(session_id, scope="session")
fortress.cancel(session_id, scope="session", release_for_recovery=true)
```

Room cancellation stops only this local monitor under current Query and intact
monitor custody. It remains available after original parent/placement loss in an
established session, with original verification withheld. It makes no native call
or building/job/clock change. Restart requires the original files for normal
opening; this slice has no separate room-only recovery mode.

Normal close requires a terminal joint monitor, settled clock and resolved original
effects. Cancel an abandoned local goal first. Explicit recovery release preserves
uncertainty and identities; it does not pause the game or prove native work stopped.

## Work bounds and proof scope

The complete packet must fit **64 KiB**. Creation and progress publication admit
the whole proposed result; insufficient output is refused instead of truncating
the objective. Placement/clock effects retain their own pre-dispatch reservations.
Room calls dispatch no game mutation. Native acquisition, original-file guards,
monitor replay, synchronization and final publication share the original shrinking
wall/byte allowance: default 10 seconds, maximum 60 seconds and at most **1 GiB**
of conservative work accounting per request. Large journals can exceed that shared
allowance and be refused even below the core's 128 MiB retention ceiling.

Identical paused endpoints establish a bounded bracket, not an atomic snapshot
or shared-process proof. Historical joint satisfaction does not prove present
room usability, native room assignment, access, circulation, pathfinding,
structural safety, continuous stability, causality or placement-effect discharge.
This source and its component tests confer no native SDK/live fortress qualification
or production admission.
