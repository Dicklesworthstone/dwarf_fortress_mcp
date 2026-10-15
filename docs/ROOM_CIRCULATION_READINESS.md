# Observe access as part of the original room goal

`track_room_readiness.py start --require-circulation` adds a strict, versioned
condition to the existing durable room workflow: every original room entry and
every furnishing approach must have a clear cardinal route in the sampled map.
Terrain, installed furnishings and access must hold together in the same shared
stability window. This is a conservative selected-footprint model, not DFHack
unit pathfinding, structural safety, native room assignment or current usability.

## Start once; retain the same requirement on every reopen

Use the existing isolated room-readiness environment and complete room-backed
furniture batch described in `ROOM_READINESS.md`:

```sh
python3 scripts/track_room_readiness.py start \
  --batch /private/room-furnishings --batch-id "$BATCH_ID" \
  --journal /private/room-monitor/readiness \
  --deadline-tick "$ABSOLUTE_DEADLINE" \
  --interval-ticks 10 --stable-samples 2 --stable-span-ticks 10 \
  --require-circulation

python3 scripts/track_room_readiness.py wait \
  --journal /private/room-monitor/readiness \
  --wait-samples 8 --poll-ms 100 --timeout-ms 10000
```

Only `start` accepts the new flag. Sampling, waiting, inspection and cancellation
recover it from the original goal; passing the flag on a reopen refuses before
opening the journal or original batch. There is no automatic upgrade, downgrade,
renewed deadline or extra game-time control. The original Query authority,
file custody, output reservation, source rechecks and foreground scheduler remain.

Omitting the flag creates the original version-1 terrain/furnishing goal.
Its bytes, digest domains, serialized sample format and normal semantics remain
unchanged. It does not acquire a new access claim. The strict inner goal uses
`DFMRDG02`; its original-batch wrapper uses `DFMRDC02`. Both use the existing
length framing and hash domains, with the version bytes inside the digest.
The journal container stays `DFMRDJ01`; its goal record explicitly retains the
new generation. Old readers reject an unknown goal generation instead of
silently treating it as version 1. No native protocol or production runner changes.

## One complete observation, two conservative graphs

The existing source owner still obtains matching paused map endpoints around
one complete receipt-linked operations capture. Circulation makes no additional
native call. Its input is the decoded map from that bracket and a regenerated
original RoomPlan, including every connecting corridor from `connect_areas`.
No caller-supplied route, abbreviated furnishing selection or cached successful
approach can replace those inputs.

The clear graph contains selected, visible, dry, undesignated floor cells with
zero building and unit occupancy. The potential graph additionally permits
unobserved cells and otherwise eligible unit-occupied cells. Walls, liquids,
active designations and building-occupied circulation cells are excluded from
both. Every planned furnishing target is excluded even when map occupancy is
absent; that absence remains a separate terrain/furnishing failure.

A nonzero building occupancy is conservatively excluded, not classified as an
impassable native building: this profile does not prove the movement semantics
of doors or other structures. Unit occupancy is transient uncertainty, not a
permanent obstruction. Neither an optimistic route nor an omitted tile proves
that a dwarf can reach the destination.

The diagnosis uses three-valued conjunction across the COMPLETE original goal:

| Route result | Meaning |
|---|---|
| `true` / `clear_route` | A clear model route reaches the entry or a cardinal furnishing approach. |
| `unknown` / `potential_route_only` | Only the potential graph supplies a route. |
| `false` / `no_route_in_selected_model` | Neither graph supplies a route inside the declared footprint. |

False here is NOT global unreachability. Outside-footprint routes, diagonals,
ramps, stairs, unit movement abilities and native pathfinding are not evaluated.
The report preserves all endpoint rows and group counts even after a decisive
failure. It never publishes only the accessible subset.

For `connect_areas: true`, the canonical first area's entry roots all routes,
and every other original entry must be reachable from it. Independent legacy
area recipes retain per-area roots; strict readiness does not quietly promise
that independent areas connect. The router searches the complete original floor
union and chooses an observed alternative when a formerly planned approach is
blocked. It does not enlarge the planned excavation footprint.

## Shared progress, not three latched successes

A blocked or unresolved required route erases the joint streak. Clearing the
obstruction begins a fresh shared window; an earlier furniture-only success
cannot supply access credit. Removal, changed identities, regressing source
horizons, map-source changes, deadlines, sample limits and cancellation retain
their existing precedence. The diagnostic reason is
`original_room_circulation_not_ready` when terrain/furniture occupancy is ready
but required access is not. More fundamental terrain failures keep their reason.

The composition now also defensively prevents a child-only terminal success
from becoming joint success after a failed joint gate, while preserving sample
exhaustion. The current native construction codec ALREADY requires at least two
samples and a positive span; the isolated one-sample/zero-span adversarial test
is defense in depth, not a claim that such a public request was admitted.
Those native timing minima are unchanged.

Strict results identify policy
`dfmcp.original-room-terrain-furnishings-and-circulation/2` and
`circulation_required: true`. The existing terrain diagnosis contains the full
`circulation` report: selected areas/furnishings, clear/potential/failed routes,
chosen approaches, distances and deterministic route/model/evidence digests.
A separate `source_bound_digest` binds that evidence to the exact map witness
and original room plan. These hashes are comparison evidence, not signatures
or mutation permissions. Historical satisfaction still requires original batch
custody for the CLI's combined success flag and never asserts present access.

## Bounds and validation scope

The engine admits at most 512 floor cells, eight areas and 32 furnishings, with
100,000 work units and 32 KiB of core diagnostic output. Traversal, validation,
canonicalization and route reconstruction call the enclosing authority/deadline
guard. The original monitor's full-output and native acquisition limits are not
increased. Every endpoint must fit before journal publication.

Executed here: all 12 core tests and ten isolated source-body tests pass. Core
coverage includes all 2,187 ternary 3x3 masks against an independent closure
oracle, unknown/clear/blocked distinctions, detours, full group truth, all guard
interruption checkpoints, exact work limits, hash-seed determinism and the
512-cell/eight-area/32-furnishing case. The isolated tests execute the actual
construction and room reducer bodies, projection, goal framing and reopen
argument checks with injected validated producers. They do NOT execute native
codecs, full module imports, disk custody or the network CLI.

Nine additional production-codec/private-journal regressions are checked in at
`tests/test_room_readiness_circulation.py` but were NOT executed in this editing
workspace. Run them in a complete checkout alongside the existing room suites:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_circulation test_room_readiness_reducer_isolated \
  test_room_readiness_circulation test_room_readiness test_room_readiness_store -v
```

Rust/MCP, DFHack SDK, live-fort and full-repository qualification remain separate.
Owning beads: `df-dfhack-bridge-plane-c-pic.3`, `.4`, `.5` (WP-05/07/10; open).
