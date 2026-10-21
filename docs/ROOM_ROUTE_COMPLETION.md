# Joint original-room completion after route-backed placement

Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-10 (remain open).

The query-only `track_room_route_completion.py` workflow retains the entire
original room-backed batch, not a selected receipt subset. All furnishings and
all original floor/corridor/doorway/wall requirements share ONE sampled stability
window. Placement registration and completed furniture alone are not room success.

## Original goal and evidence

The goal embeds the existing complete original route-batch construction goal and
its exact RoomRouteHandoff. Creation requires all original placements to have
registered resolved Placed receipts. Every receipt keeps its original key,
selection, source generation, chronology and material/identity constraints.
The original batch must remain open and rechecked even for terminal inspection.
Furniture-only batches cannot silently acquire a room parent.

Each sample retains two identical paused map endpoints bracketing one complete
receipt-linked operations sample. All original receipts are queried before and
after that bracket. Build, map and operations generations are pinned separately;
software, fortress, map dimensions, full original capture region, game tick and
identity horizons must agree. Existing paged-inventory validation and release
remain owned by the existing construction transport. The connection closes
before CPU-bound sample validation. No placement or clock method is added.

The map must preserve every original required floor, corridor, doorway and wall.
Every original furnishing target additionally needs visible building occupancy;
exact building identity comes from the receipt-linked roster, never the occupancy
tag. No item substitution, dependency rewrite or reallocation occurs.

The unchanged construction_plan reducer owns temporal progress for all conditions
together. Room defects or unknown cells reset the unfinished streak. A changed
map at the same tick resets it even when the inventory did not change. Alternating
room/furniture successes cannot combine into success. Original invalidation,
failure, deadline and lifetime sample limits retain precedence. Interrupted
reads cannot continue an older streak. Monitor cancellation never cancels or
undoes a placement, clears uncertainty, or grants permission to retry an effect.

## Commands and authority

Use the EXISTING isolated room-readiness query environment:

```text
DFMCP_ALLOW_UNADMITTED_ROOM_READINESS=1
DFMCP_ROOM_READINESS_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_TOKEN=<existing build-read credential>
DFMCP_OPERATIONS_PAGED_TOKEN=<existing operations-read credential>
DFMCP_MAP_TOKEN=<existing map-read credential>
```

The endpoint and paths below are illustrative. Other DFMCP environment variables
are refused by the unchanged authority loader. Do not mix this with placement,
time-control or production-launch permissions.

```sh
python3 scripts/track_room_route_completion.py start \
  --batch-directory /absolute/private/original-room-batch \
  --batch-id ORIGINAL_BATCH_ID --journal /absolute/private/monitors/room.monitor \
  --deadline-tick FIXED_GAME_TICK --stable-samples 2 --stable-span-ticks 10

python3 scripts/track_room_route_completion.py sample \
  --batch-directory /absolute/private/original-room-batch \
  --batch-id ORIGINAL_BATCH_ID --journal /absolute/private/monitors/room.monitor \
  --expected-goal ORIGINAL_GOAL_DIGEST
```

`start` creates the original goal and acquires its first sample. `sample` acquires
one new sample. `wait` uses the existing bounded FOREGROUND scheduler and accepts
`--wait-samples 1..32` and `--poll-ms 10..5000`; it never advances game time or
retries a failed acquisition. Five extra map RPCs are included in its admission
floor. All calls, storage, work and wall time share one shrinking budget.

`inspect` and monitor-only `cancel` work offline without query credentials, but
still require both the complete original batch and its separate monitor. On
reopen, `--expected-goal` is mandatory and all deadline/cadence/stability overrides
are refused. The caller must preserve the returned goal digest independently.

## Durability and output

The new journal uses fixed `DFMRQJ01` framing, `DFMRQG01` goal bytes and `DFMRQS01`
samples. Old furniture-only monitor formats are not upgraded or reinterpreted.
The existing private-file owner supplies exclusive creation/locking, no-follow
parent traversal, exact private modes, append synchronization, complete byte
readback and corruption fencing. The monitor must live OUTSIDE the closed batch
directory; its parent must already exist with mode 0700, and its file uses 0600.

Read intent is synchronized before native contact. A sample is rendered and
fully validated before its append. Interrupted reads remain explicit; reopening
never restores read ownership or renews the original timing policy. Torn frames,
changed files and ambiguous writes are not repaired or overwritten. The journal
is bounded at 128 MiB/1030 frames and reserves a complete maximum-size sample
before starting a read. Capacity exhaustion refuses without dropping history.

One bounded JSON/Agent Turn response retains the original room and furniture
plans, goal/parent identities, complete current assessments, room deficits and
historical progress. The limit is 256 KiB. Output space is reserved before read
intent, and authority, original custody, monitor identity and deadline are
rechecked after serialization. A short output write never causes a second object
or another acquisition. Both original owners remain open through that attempt.

`sampled_whole_room_condition: true` is historical sampled evidence only. It is
NOT an atomic cross-profile snapshot, continuous wall preservation, native room
assignment, current usability, worker pathfinding or production admission.
`room_completion_proven` remains false for those stronger game-level claims.

## Executed evidence and remaining boundaries

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_room_route_completion test_room_route_completion_runtime \
  test_room_route_completion_rpc -v
```

38 new tests pass without skips: 14 condition/codec, 17 private-journal/workflow,
and seven acquisition-orchestration tests. Actual source executes the joint
reducer, unchanged construction temporal engine, map/request/plan/review codecs,
private monitor files, append/replay/synchronization, foreground scheduler and
complete CLI main function. Tests cover all 32 original furnishings, same-tick
changes, original deadlines, every goal/sample truncation, every truncation and
byte corruption of a persisted sample frame, lost sync acknowledgment, changed
file/path/mode, read ownership, original-custody loss, source pinning, one-shot
acquisition, authority revocation after rendering and single-attempt output.

The native receipt/assessment, room compiler/terrain diagnosis and original-batch
custody interfaces are injected producer ports. Acquisition tests inject the
transport base and existing map-method implementations. They test the actual new
adapter's order and source checks, NOT TCP framing, inventory paging/release,
real DFHack or the full default native import/execution pipeline. The actual
complete private route-batch directory is not reopened by this test suite.

Four deliberately weakened implementations fail assertion regressions: ignoring
room truth, ignoring same-tick map change, omitting final authority checks and
omitting original-map handshake binding. Reconstructed unchanged sources match
Git blobs for construction_plan, construction_monitor_store and construction_wait.
The original runner's 27 tests also pass; its 12 previously unpublished room-run
regressions are included with this integration. No Rust/MCP, SDK/plugin, live
fortress, broader room/corridor combinations or full-workspace qualification is
claimed. Existing native protocols and production admission remain unchanged.
