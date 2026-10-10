# Durable whole-room readiness monitoring

`scripts/track_room_readiness.py` checks the complete original room terrain and
every original furnishing together. It connects the existing room-readiness
condition to a private journal and an executable development workflow, so a new
process can continue the same room goal without reconstructing its receipts or
combining separately completed terrain and furniture monitors.

The input is an original room-backed furniture batch whose every step already
has its registered `Placed` receipt. The journal retains the full room request,
geometry, constraints, allocation evidence, selected items, placement records,
and original file identities. A successful subset cannot become the room goal.
See [the room furniture pipeline](ROOM_FURNITURE_PIPELINE.md) for creation and
placement of that batch.

## Start and sample the original room goal

Use an isolated query-only environment with the following settings. Tokens are
the existing plugin credentials; they belong in the operator environment, not
command arguments or saved room plans.

| Variable | Value |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_ROOM_READINESS` | Exactly `1` |
| `DFMCP_ROOM_READINESS_ENDPOINT` | Original numeric IPv4 loopback endpoint; defaults to `127.0.0.1:5000` |
| `DFMCP_BUILD_TOKEN` | Original furniture plugin query credential |
| `DFMCP_OPERATIONS_PAGED_TOKEN` | Original operations/1.4 read credential |
| `DFMCP_MAP_TOKEN` | Map/1.5 read credential |

Other `DFMCP_*` settings are refused. In particular, switch out of the placement
or inventory-allocation environment before running this monitor.

Create an owned private parent directory outside the original furniture batch.
Use a future absolute game tick for the deadline, leaving enough time for the
chosen stability window:

```sh
mkdir -m 700 /private/room-monitor

python3 scripts/track_room_readiness.py start \
  --batch /private/room-furnishings --batch-id "$BATCH_ID" \
  --journal /private/room-monitor/readiness \
  --deadline-tick "$ABSOLUTE_DEADLINE" \
  --interval-ticks 10 --stable-samples 2 --stable-span-ticks 10

python3 scripts/track_room_readiness.py sample \
  --journal /private/room-monitor/readiness

python3 scripts/track_room_readiness.py inspect \
  --journal /private/room-monitor/readiness
```

`start` creates the immutable goal and acquires its first sample. Each
nonterminal `sample` performs one complete acquisition. Reopening takes only
the journal path; it cannot replace the original batch, receipt set, deadline,
cadence or observation allowance. `inspect` and terminal sampling are offline.
Successful original-room reports still recheck the original batch custody.

## What one sample establishes

One connection performs a fixed sequence:

1. Query every original placement receipt.
2. Read the complete original room map selection.
3. Acquire all pages of one operations capture and verify its release.
4. Read the identical room map selection again.
5. Query every original placement receipt again.

Both map captures must be byte-identical, paused, and at the same game tick as
the operations capture. Original receipt bytes must match on both sides. Each
profile keeps its own generation identity; software versions, fortress, map
dimensions and source horizons must agree with the original evidence.

The common condition requires every original floor and corridor to be visible,
dry, floor-shaped and free of digging designations, every required bedroom
boundary wall to remain intact and unoccupied, and every original furnishing
to meet its receipt-linked completed-construction condition. Map occupancy must
also show a building at each furnishing target. Exact building and item identity
comes from the original receipts and operations roster.

All these requirements share one stability streak over advancing game ticks.
Losing a wall or one furnishing resets the complete streak. Hidden or unavailable
terrain remains unknown. Repeated paused ticks add no stability; a long gap or
changed sample at the same tick resets the streak. Source, identity or clock
regression invalidates the goal. Deadlines and observation limits stay fixed.

Responses retain the full original RoomPlan, every target's room and step
association, furnishing assessments, terrain deficits, source witnesses and
active monitoring work. Joint success is exposed only when the sampled
condition is satisfied and original batch custody is verified in that call.
Historical journal evidence remains distinguishable from current observations.
The result schema is `dfmcp.room-readiness-monitor-result/1`, with
`room_readiness_sampled_condition` as the joint success flag. Complete responses
are limited to 262,144 bytes, including the original plans and all assessments.

## Restart, failed reads and cancellation

A synchronized read-start record precedes native contact. Complete raw sample
evidence is validated and its entire response reserved before appending. File
and parent synchronization, full-byte verification and original-source rechecks
precede acknowledgment. Replay derives progress from the original evidence;
stored labels alone cannot claim completion.

An interrupted acquisition remains visibly unresolved. A later explicit sample
begins a new read and resets unfinished stability, retaining the original
deadline and observations already consumed. A clean process reopening preserves
completed samples under the same maximum-gap rules. No unsampled interval is
claimed to be continuously safe.
While `read_outcome_unknown` is true, `effective_streak` is zero; `streak` still
records the last completed sample's historical count.

Cancellation works with only the monitor journal, even if original placement
files have become unavailable:

```sh
python3 scripts/track_room_readiness.py cancel \
  --journal /private/room-monitor/readiness
```

It stops local monitoring and preserves history. It does not cancel a game job,
retry placement, or discharge an original effect. Without original custody the
response withholds verified original-room success. Keep the original batch and
monitor after satisfaction or cancellation.

Private custody uses owned `0700` directories and single-link regular `0600`
files, descriptor-pinned no-follow access, exclusive local locks, strict bounded
hash chaining and no automatic truncation, repair or eviction. A failed write,
sync, output reservation or custody recheck fences the owner. The new journal
has a separate fixed format; existing furniture and terrain journals are not
migrated.
The `DFMRDJ01` journal retains at most 128 MiB and 1,030 frames. Its
`DFMRDC01` goal binds the complete original batch and the fixed joint condition.

## Scope and limits

This is a Python development workflow using existing map/1.5, operations/1.4
and furniture/1.19 query methods. It creates no native mutation method or
production MCP runner. A satisfied result is historical sampled readiness of
the declared terrain and furnishings. It does not establish native room zoning
or assignments, structural safety, pathfinding, present usability, continuous
preservation, an atomic cross-profile snapshot, or production admission.

The complete selection is bounded to 32 furnishings and the existing 4,096-cell
room map. One shrinking timeout and work/I/O budget cover original-file replay,
acquisition, validation, publication and final output. Capacity refusal preserves
the prior journal and leaves offline inspection and cancellation available.

## Executed development tests

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_readiness test_room_readiness_rpc \
  test_room_readiness_store test_track_room_readiness \
  test_room_readiness_cli_authority -v
```

The suites passed all 60 methods in scoped runs: 27 existing condition/TCP
tests, 15 new private-file/codec tests, 16 new CLI process tests and two final
publication-authority regressions. The maximum
case retains all 32 furnishings and 646 exclusions, reads a paged roster with
2,000 extra items, and reaches joint readiness across separate CLI processes.
The process suite kills a real monitor process at the operations-release
boundary, then verifies unresolved read intent and a fresh shared streak on
recovery. Tests also cover lost trailing reads, changed map incarnations, source
loss during acquisition, full original receipt identity, walls lost while
furnishings finish, immutable deadlines, corruption and publication failures.
Revocation during final source verification or owner close withholds the response,
while the already synchronized historical sample remains inspectable offline.

These tests use actual private files, Python clients and CLI subprocesses with
synthetic TCP peers. They do not establish native DFHack SDK/ABI, live-game,
Rust/MCP or full-repository qualification.

Owning beads: `df-dfhack-bridge-plane-c-pic.3`,
`df-dfhack-bridge-plane-c-pic.4`, and `df-dfhack-bridge-plane-c-pic.5`.
The wider native-dispatch and live-qualification work remains open.
