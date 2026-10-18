# Compare complete room layouts before selecting one

Bead: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` (WP-05/WP-10).

The room-layout request accepts one optional objective:

```json
"objective": "minimum-target-tiles/1"
```

This selects a bounded whole-project search instead of returning its first
feasible candidate. The objective and distinct result policy
`dfmcp.mrv-ranked-room-layout/1` are retained in the canonical request/artifact.
Omitting the field preserves every original first-feasible choice, trace,
work count and saved-layout byte. Unknown objectives are refused.

## What is minimized

The lexicographic score is: complete target-tile count; summed same-level
Manhattan distance from each area entry to the declared focus; then each
area's `(y, x, z, rotation)` tuple in lexical area-name order. Lower is better.
The first quantity includes all room floors, doors, local corridors, project
corridors and an external entrance when required. The second is geometric
entry distance, not native pathfinding or worker travel time.

This is **planned target footprint**, not observed new mining work. Caller
site-mask `F` permissions do not prove that a floor already exists and cannot
be used as a free excavation cost. No terrain, completion or mutation claim
is strengthened. All original areas, furnishings, item selectors, exclusions,
wall rings, fixed origins, origin regions and allowed rotations remain binding.

The existing connection strategy is independently retained. Without
`connection_strategy`, comparison uses the original deterministic greedy
connector. Supplying `"connection_strategy": "greedy-then-minimum-tiles/1"`
enables its bounded minimum-corridor fallback. The optimizer does not search
all possible corridor trees or change the connection algorithm.

## Bounded search and honest optimality

The original node, aggregate work and caller deadline allowances cover
normalization, candidate generation, complete compilation, ranking and result
publication. No allowance is renewed per candidate. Only one complete best
plan is retained; the first feasible plan retains only its digest and score.

A conservative branch bound uses the sum of disjoint area tile counts and
independent minimum entry distances and placement tuples. It ignores conflicts
and corridor costs only in the mathematical lower bound, never in a returned
plan. A branch is pruned only when it cannot beat the complete incumbent.

When the finite search finishes, `optimization.search_complete` is true.
`minimum_in_compiler_domain_proven` is true only when a complete incumbent
exists and every branch was evaluated or excluded by that bound. The scope is
the declared positions/angles and chosen deterministic compiler, not arbitrary
fortress designs, minimum native mining, or global corridor optimality.
`global_optimum_proven` remains false.

A node limit after a complete candidate returns `layout_found` with that
best-so-far **complete** plan and explicit unproven optimality. A node limit
before any candidate returns `search_limit_reached` without a plan. Work
exhaustion, cancellation, caller deadline failure, or an inconclusive corridor
solver returns no plan, even when an incumbent was previously found.

The `optimization` record contains the first and selected scores/digests,
feasible-candidate count, improvement count, bound-pruned branch count, objective,
area ordering and exact completion status. Replay repeats the original bounded
search and rejects altered scores, policies, selected plans or optimality flags.

## Example

`examples/room_layout_minimum_targets.json` keeps one complete eight-furnishing
hall and its fixed entrance. First-feasible placement needs 49 target tiles;
comparison selects a 39-tile placement without an added corridor. A separate
two-hall regression reduces 85 targets to 78 while retaining all 16 furnishings.
These are synthetic geometric examples, not measurements of a live fortress.

```sh
python3 scripts/layout_rooms.py --request-file examples/room_layout_minimum_targets.json --emit layout > layout.json
python3 scripts/layout_rooms.py --layout-file layout.json --emit room-plan > room-plan.json
```

The existing exports accept the selected complete RoomPlan; they do not reroute
or reallocate it. Subsequent survey, allocation, native review and one-shot
execution keep their own original authority and validation requirements.

## Component validation

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_layout_ranking.py' -v
```

Nineteen tests execute full ordinary compiler and allocator imports. Independent
absolute-origin/angle enumeration checks nine two-area domains and every one
of 32 obstacle masks. Other cases cover 32 furnishings across eight areas,
fixed positions, shared work, every checkpoint in a small cancellation fixture,
post-incumbent revocation, monotone node-budget improvements, exact replay,
constraints and fallback interaction. A differential corpus of 112 legacy
outcomes retains byte-identical results. Native acquisition, DFHack, Rust,
full-repository and production qualification are not implied.


## Require a proven finite-domain optimum before export

The existing CLI accepts `--require-optimal-layout` for both request input and
saved-layout input. It verifies the regenerated `layout_found` result and requires
`optimization.minimum_in_compiler_domain_proven` to be exactly true before any
export. This is an output requirement, not permission to add an objective, change
a saved request, enlarge the search budget, or silently restart optimization.

```sh
python3 scripts/layout_rooms.py --request-file examples/room_layout_minimum_targets.json --require-optimal-layout --emit layout > optimal-layout.json
python3 scripts/layout_rooms.py --layout-file optimal-layout.json --require-optimal-layout --emit room-plan > room-plan.json
```

A node-limited incumbent, a no-layout result, and a legacy first-feasible layout
cannot satisfy this requirement. The command exits 2 with
`ROOM_LAYOUT_OPTIMUM_UNPROVEN` and `result: null`; it emits no room artifact.
Without the flag, a complete best-so-far plan remains exportable under the
explicit objective's normal contract. Existing work/deadline failures still
return their original refusal codes, never an incumbent. A re-signed forged
optimality flag cannot pass: saved input is replayed before export checks.

The `report` export adds `agent_turn.briefing.layout_optimization`, which states
the first-feasible and selected target counts, search completion, optimality
scope and stopping reason. Successful strict exports preserve the selected
artifact's exact bytes. All six export kinds retain their existing formats;
legacy reports without an objective remain byte-identical.

## Executed CLI and regression evidence

Thirteen CLI tests use real subprocesses, full normal module imports and
bounded temporary input files. Both requests and saved layouts exercise all six
exports, with successful exact downstream RoomPlan/Request decoding and complete
eight-item allocation. The strict path rejects every node-limited export kind.
Other tests cover re-signed altered scores/optimality/requests, no incumbent,
work failure after an incumbent, process hash-seed equality, original entrance
retention, final-render deadline failure and short/failed output without another
search or response. Deadline/output fault hooks target the actual offline CLI;
no solver replacement, native peer or source attestation is used.

The complete final component run passes 59 warnings-denied tests: 19 optimizer,
13 CLI and 27 unchanged existing corridor/room-compiler regressions. The latter
include the 512-tile fallback project and existing 6,561-world corridor oracle.
The 112-result legacy search corpus and four legacy report cases retain their
pre-change bytes. These do not establish observed-map acquisition, native DFHack,
Rust/workspace, full-repository, live-game or production qualification.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_layout_ranking*.py' -v
python3 -W error -m unittest discover -s tests -p 'test_room_connection_*.py' -v
```
