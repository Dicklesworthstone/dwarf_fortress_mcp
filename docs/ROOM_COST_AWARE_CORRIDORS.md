# Cost-aware room corridors

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` and
`df-dfhack-bridge-plane-c-pic.3/.4/.5`.

Room placement can now reuse existing passages instead of choosing corridors
solely by their number of tiles. The new mode compares two complete corridor
candidates. It never removes rooms, substitutes furnishings, excavates protected
cells or raises the original tile, rectangle or work limits.

## Observed map workflow

Use the existing surveyed-layout command with both explicit request fields:

```json
"objective": "minimum-observed-excavation/1",
"connection_strategy": "cost-first-with-original-candidate/1"
```

The request in `examples/room_cost_aware_corridors.json` shows two fixed dining
halls in a 13-by-5 domain. Set its actual save folder, site and coordinates before
using a real source. The example does not describe an installed fortress.
Configure the existing isolated read-only map environment, then run:

```sh
python3 scripts/survey_room_layout.py \
  --request-file examples/room_cost_aware_corridors.json \
  --require-optimal-layout --emit survey > survey.json

python3 scripts/survey_room_layout.py \
  --survey-file survey.json \
  --require-optimal-layout --emit room-plan > room-plan.json
```

The first command uses one existing map/1.5 observation. Its connection closes
before comparison. The second command is offline evidence replay; it does not
reconnect or claim that historical terrain is current. The remaining exports
(`report`, `layout`, `room-request`, `excavation`, `furniture-request`) retain their
existing formats. No new native method, placement command or mutation authority
is introduced.

Costs come from the original decoded map before intersecting the caller's site
mask: eligible existing floor costs zero; wall-shaped targets cost one. Unknown,
liquid, designated, occupied and other excluded cells remain unusable regardless
of cost. Caller `F` permission on an observed wall cannot make that wall free.
The observed path rejects caller-authored `excavation_costs` or `corridor_costs`.
It independently recounts every selected target against the same raw capture.

## Original intent and offline assumptions

Offline `room-layout-request/1` input uses the same new connection strategy with
`"objective": "minimum-excavation-cost/1"` and a complete `excavation_costs` string
of binary characters, in x-fastest/y row order over its exact `region`. Those
costs are declared assumptions, not observations. Supplying the strategy without
that cost objective and complete map is refused.

The selected `RoomPlan` retains a complete `corridor_costs` object in its original
intent. Direct `room-provisioning-request/1` callers can also supply it:

```json
"connect_areas": true,
"corridor_costs": {
  "region": {"origin": [10, 10, 2], "size": [13, 5, 1]},
  "cells": "00000000000000111111111110000011111000000001111100000000111110000"
}
```

This cost string has exactly 65 cells. The domain is in world coordinates; it
neither rotates with an area nor shifts its original geometry. Every original
floor must fit. When an explicit access point is present, its routing region and
the cost region must be identical. The full original bedroom wall rings, room
furniture targets and caller exclusions remain protected.

Cost-aware detours may leave the original room bounding box, but cannot leave
the declared domain. The compiled capture hull and excavation-mask digest are
recomputed from the exact final target union, never from the larger search box.
The complete cost map and selected geometry participate in saved-plan replay.
Changing and rehashing derived coordinates or claims cannot bypass regeneration.

Omitting the new strategy and cost object preserves legacy decisions, digests,
work counts and report bytes. No process-global mode or replay-time migration
exists. Old implementations reject new fields rather than treating the new
geometry as legacy evidence.

## Algorithm and hard limits

The baseline runs first through the existing connector, including its bounded
minimum-tile fallback. A deterministic cost-first greedy pass then minimizes
added cost, added tiles and cardinal steps for each connection. Only complete
feasible candidates are compared by total added cost, added tiles and rectangle
count; the original wins exact ties. A zero-cost baseline is retained directly.
A baseline exceeding the remaining rectangle limit is not an eligible fallback.

A cheaper detour that exceeds either geometric capacity is discarded as a
whole. The baseline can then be retained if it meets both limits. Work exhaustion,
cancellation or other unexpected failure returns no result, even after a valid
baseline. One 100,000-check ceiling spans cost validation, both attempts,
comparison, reconstructed-mask verification and the final witness. The enclosing
layout/observation work budget and deadline continue to shrink during all of it.
There are at most 1,024 region cells, 512 complete target tiles, 32 parts, eight
areas and 32 furnishings; none of these bounds is increased.

This is NOT a globally optimal cost network solver. Baseline failure still
refuses; an overlong free route is not replaced with an exhaustive search for
all intermediate tradeoffs. The original minimum-tile fallback and the new
cost-first pass are different algorithms with explicitly limited claims.
`--require-optimal-layout` proves comparison over the declared layout candidates
and this deterministic connector only, not all possible corridor networks.
A node-limited complete layout can be exported without that flag, but no
optimality claim is made. Work/cancellation/deadline failure exports no incumbent.

## Concrete regression and evidence

In the synthetic two-hall fixture, the original route crosses five wall-shaped
cells and has 29 total floor targets. The new route reuses a longer existing
passage: zero wall targets and 39 total targets. Both halls and all four
furnishings remain identical, and the actual complete allocator accepts all four
or reports a whole shortage. This exchanges a larger footprint for fewer
observed wall targets, not measured mining time or native pathfinding proof.

Executed tests use ordinary complete module imports, real local TCP connections
and CLI subprocesses with synthetic source bytes:

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_cost*.py' -v
python3 -W error -m unittest discover -s tests -p 'test_room_connection_plan.py' -v
python3 -W error -m unittest discover -s tests -p 'test_room_connection_solver.py' -v
```

The 71-test total is 18 connector cases, 17 full workflow cases, nine TCP/CLI
cases and 27 unchanged compiler/solver cases. An independent simple-path oracle
covers all 2,187 cost/barrier assignments of a two-terminal 3-by-3 map. Nineteen
joined synthetic-peer sessions exercise all seven live exports, offline replay,
connection closure before search, lost/malformed replies, changed authorization,
shared budget failures and final output faults without repeat reads.
A differential comparison preserves 32 legacy RoomPlan artifacts, 96 layout
results, 128 observed artifacts and 128 reports byte-for-byte.

Real DFHack, native/Rust builds, room-backed effect execution and full-repository
qualification were not run. Existing effect journals, original-key recovery,
placement permissions and production admission are unchanged. No proof of
mineability, aquifer safety, construction support, room assignment or current
fortress access is inferred.
