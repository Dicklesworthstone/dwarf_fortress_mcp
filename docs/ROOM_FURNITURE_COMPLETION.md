# Room-backed furniture placement and sampled construction completion

The existing furniture CLI and original-batch completion monitor accept the
room-backed batch/3 format. Original room geometry, furnishing constraints,
exclusions, selected items and all original placement keys survive restart.
This is an isolated development workflow, not an admitted MCP/game capability.

## Place the exact original furnishings

Under the existing isolated furniture/1.19 environment, import a canonical
`dfmcp.room-furniture-handoff/1` capsule into an empty private directory:

```sh
mkdir -m 700 "$HOME/room-furnishings"
python3 scripts/furniture_batch.py init --directory "$HOME/room-furnishings" \
  --room-handoff room-furniture-handoff.json
```

The capsule supplies the original fortress selectors; overrides are refused.
Initialization observes but does not place. Each original step still requires
`review`, followed by `advance` with the returned batch ID, expected native plan
and confirmation seal. Keep the returned batch ID. A lost reply requires `query`
or `cancel` on that original step, never a new batch or repeated commit. The
existing placement opt-in remains separate from read authority.

The batch's full RoomPlan is included in every verified inventory. Recover its
exact canonical bytes offline without selecting a smaller successful subset:

```sh
python3 scripts/furniture_batch.py inspect --directory "$HOME/room-furnishings" \
  --batch-id "$BATCH_ID" --emit room-plan > original-room-plan.json
```

## Sample construction after all original placements

Switch to the existing isolated construction-monitor environment (do not mix
placement/allocation profile variables). Configure the original loopback endpoint,
`DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR=1`, `DFMCP_BUILD_TOKEN` and
`DFMCP_OPERATIONS_PAGED_TOKEN`. Place the monitor outside the original batch,
in its own existing owned 0700 parent directory:

```sh
python3 scripts/track_furniture_batch.py start \
  --batch "$HOME/room-furnishings" --batch-id "$BATCH_ID" \
  --journal "$HOME/room-monitor/completion" --deadline-tick "$ABSOLUTE_DEADLINE"

python3 scripts/track_furniture_batch.py sample \
  --journal "$HOME/room-monitor/completion"
```

The deadline is an absolute game tick after the original receipts and must allow
the fixed stability policy. Start requires every original step to have a registered
Placed receipt. Each explicit sample checks all original receipts before and after
one complete released operations capture, on one connection. There is no effect
retry. Repeated paused ticks cannot complete the shared stability window.

`result.requested_room_plan` retains the complete room intention. Every target
names its original plan step, room area/unit, item, building, job and receipt.
`complete_original_room_furnishings_sampled_condition` requires the shared
condition and verified original batch custody during this call. It does not
mean the entire room is complete. Floors, walls, access and room assignments
are not observed by this monitor; the relevant proof flags remain false.

Inspect or terminal sampling can reopen offline. Cancellation opens only the
monitor, so original-file loss does not prevent stopping local monitoring:

```sh
python3 scripts/track_furniture_batch.py cancel \
  --journal "$HOME/room-monitor/completion"
```

This preserves complete historical room intent but withholds verified original
custody and combined construction claims. It cancels no game job and does not
discharge or repeat placements. Keep both original stores even after satisfaction.

## Bounds and compatibility

Room-backed origins/goals use DFMFCO03/DFMFCG03. The two previous origin/goal
versions, native protocols, receipt bytes and private journal framing remain
unchanged. The complete room manifest is bounded to 128 KiB; room-facing results
to 192 KiB. Older output profiles retain 64 KiB. All original targets must fit;
there is no partial-response fallback. Final serialization, original file custody
and live query authority remain checked under the foreground deadline.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests \
  python3 -m unittest test_room_furniture_completion -v
```

Twelve methods passed in separate final groups with actual CLI processes, real
private journals and synthetic joined TCP peers. The maximum-set method placed
32 original items retaining 646 exclusions, then sampled all original receipts
around multipage operations captures to shared satisfaction. Another 30 legacy
origin, goal, digest and response comparisons matched the previous implementations.
This is not live DFHack, Rust/MCP or repository-wide qualification. See the
implementation-status fragment for the precise scope and remaining limitations.
