# Original project access in sampled room readiness

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/07/10).

A connected room layout can carry an original `intent.access.point`, but
room-local circulation alone does not prove a route from that point. In
particular, blocked or unknown corridor cells before the first room must not
be ignored when every room and furnishing is locally reachable.

`room_circulation.assess(..., connected=True, access_point=point)` evaluates
all entries and furnishing approaches from that exact point in the complete
original floor mask. It uses separate clear and potential cardinal graphs;
unknown or unit-occupied passages can establish only a potential route.
A blocked point cannot be silently replaced by an internal room entry. All
furnishing targets remain blocked in both graphs.

Evidence policy `dfmcp.observed-room-access-circulation/2` binds the point,
complete selection and observed partitions in the model and route digests.
The point is an independent root, not a ninth room or extra furnishing.
At maximum selection it returns 41 records: the root, eight entries and
32 furnishing approaches. Existing 512-tile and work/output limits remain.

Calls without the new point retain their previous policy, canonical bytes,
digests, work counts and room-local interpretation. An entrance model does
not prove membership in the existing fortress, native unit pathfinding,
continuous access, current safety or authority to mutate the game.

Test command:

```sh
python3 -m unittest discover -s tests -p 'test_room_access_circulation.py' -v
```

## Persist the complete access requirement

For an original entrance-backed furniture batch, start a **new** monitor with:

```sh
python3 scripts/track_room_readiness.py start \
  --batch /private/room-furnishings --batch-id "$BATCH_ID" \
  --journal /private/room-monitor/readiness-access \
  --deadline-tick "$ABSOLUTE_DEADLINE" \
  --require-project-access \
  --interval-ticks 10 --stable-samples 2 --stable-span-ticks 10
```

Use the existing isolated query-only readiness environment documented in
`ROOM_READINESS.md`; this adds neither credentials nor mutation methods. The
new option includes circulation. It fails if the original RoomPlan lacks a
connected `intent.access` requirement; it never selects or invents an entrance.

The inner goal uses `DFMRDG03`, the original-custody goal uses `DFMRDC03`, and
both seal the stronger scope into their existing goal digests. The outer
`DFMRDJ01` journal frame format and all raw sample bounds remain unchanged.
The policy is `dfmcp.original-room-terrain-furnishings-and-project-access/3`.

Every original terrain, furnishing, and entrance route shares one stability
window. A known building obstruction before the first room breaks access even
when every room-local route and furniture condition is true. Unit occupancy
or unobserved passages cannot earn a positive access sample. The existing
reducer resets unfinished stability and retains its original deadline,
cadence, lifetime allowance and source floors. Physical effects are not retried.

`sample`, `wait`, `inspect`, `check` and `cancel` recover the selected scope from
the original journal. Passing either policy option while reopening is refused.
A fresh `check` uses the same versioned Goal/Sample validation and can report
failed current access separately from a previous sampled success. It neither
rewrites that success nor renews authority or stability credit.

Versions 1 and 2 continue to decode and replay with their original meaning.
The presence of `intent.access` in a saved plan does not upgrade an old
room-local monitor. `--require-circulation` without the new option still starts
a version-2 monitor; do not interpret its success as entrance reachability.
No stored journal is repaired, relabeled or migrated in place.

### Validation scope

43 Python tests pass: 12 circulation tests (including 6,561 three-state graph
configurations), 9 policy tests, 6 source-projection seam tests and 16 full
raw-codec readiness cases. The seam tests inject selection/tile records; the
raw-codec cases instead run the actual original-room, allocation handoff,
placement receipt, map and operations decoders using synthetic source bytes.
They cover maximum eight-area/32-furnishing selection, blocked and unknown
entrances, same-tick changes, interruptions, source regressions, fixed deadlines
and lifetime budgets, and fresh checks without rewriting terminal history.

```sh
PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_access_circulation test_room_readiness_policy \
  test_room_readiness_access_seam test_room_project_access_readiness -v
```

The complete private-journal and CLI transport workflow was not executed in the
component checkout. Store and CLI changes were source-reviewed and syntax
checked. These tests do not qualify native DFHack, Rust, full-repository or
live-fortress operation, and no production admission gate was changed.
