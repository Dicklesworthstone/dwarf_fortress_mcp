# Reuse existing floors when laying out complete room projects

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd`,
`df-dfhack-bridge-plane-c-pic.3/.4/.5`. Broad planning/execution work remains open.

The existing map-backed layout command accepts a new explicit objective:

```json
"objective": "minimum-observed-excavation/1"
```

It compares complete original room projects by the number of selected target
cells observed as walls, then total target footprint, summed entry distance,
and stable lexical-area y/x/z/rotation ties. Existing floors incur zero cost;
wall-shaped targets incur one. Every original furnishing and required room,
wall, exclusion, entrance and orientation constraint remains binding.

This fixes a practical difference between footprint and excavation: two halls
can each use twelve tiles, but one may already be excavated while the other
occupies twelve walls. The new objective selects the former without deleting
rooms or furnishings. The synthetic regression retains both furnishings and
all twelve floor targets while reducing selected wall targets from twelve to
zero. Another case retains eight areas and all 32 furnishings on existing floors.

## One read, one original request

Start with a `dfmcp.room-layout-request/1` request and this objective; do not add
`excavation_costs`. The command acquires one complete map/1.5 capture through the
existing isolated read client. That connection closes before placement search.
No extra protocol, mutation method, source policy, retry, clock advance or
background task is introduced.

The map must identify the requested paused fortress and complete bounded region.
The decoder checks the original bytes; caller-authored tile records are not
accepted. Hidden, missing, wet, designated, occupied, unsupported and map-edge
cells remain excluded. Required bedroom walls still require observed wall shape.
A selected entrance still requires visible, dry, undesignated, unoccupied floor
in the original observation.

Costs are derived **before intersection with caller site constraints**. An `F`
constraint permits intended floor use; it does not prove current floor. `F` on
an observed wall therefore remains charged. Caller-supplied `excavation_costs`
is rejected before contact, including an all-zero string. The offline assumed
cost objective is a separate contract; see `ROOM_LAYOUT_EXCAVATION_COSTS.md`.

After selecting a complete candidate, the wrapper independently recounts every
selected target against the raw decoded capture. The exact count must equal
the optimizer's score. The evidence records wall/floor counts, the original map
witness and game tick, cost-map digest and finite-domain proof status. A result
without a complete selected plan reports null counts, not a claim of zero work.

## Run and replay

Edit `examples/room_layout_observed_excavation.json` to name the intended save,
site and bounded region. Native acquisition uses only the existing environment:

- `DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT=1`;
- `DFMCP_ROOM_LAYOUT_ENDPOINT` for the existing numeric loopback endpoint;
- `DFMCP_MAP_TOKEN` containing the existing query credential.

Do not combine this isolated read profile with other DFMCP authority settings.
Do not put credentials in request files, source control or output.

```sh
python3 scripts/survey_room_layout.py \
  --request-file examples/room_layout_observed_excavation.json \
  --require-optimal-layout --emit survey > survey.json

python3 scripts/survey_room_layout.py \
  --survey-file survey.json \
  --require-optimal-layout --emit room-plan > room-plan.json
```

The strict flag requires a complete selected layout and proof of optimality
within the declared compiler domain. A node-limited incumbent may be exported
without it, but strict export returns `ROOM_LAYOUT_OPTIMUM_UNPROVEN` and no
artifact. The flag also works for the older footprint objective. It never
changes requests, expands a budget or promotes first-feasible results into
optimality. Work, deadline, cancellation, source or authority refusal exports
no candidate. Short output produces no second object or repeated acquisition.

All seven existing exports remain available: `report`, `survey`, `layout`,
`room-plan`, `room-request`, `excavation`, `furniture-request`. Reports expose
`agent_turn.briefing.excavation_comparison`. Saved-survey replay repeats raw
map decoding and the complete original search without a connection or native
credentials; it never claims a new observation.

The survey retains both the raw map and original observed-objective request.
Its nested layout retains the derived binary cost map and can replay on its
own, but that standalone layout is deliberately classified as assumed geometry
and costs, not authenticated map acquisition. Exporting it cannot transfer the
survey's acquisition claim. Room plans remain geometry, not terrain proof.

## Bounds and scope

Existing bounds are unchanged: one same-level 1,024-cell search domain, at most
512 complete floor targets, eight areas, 32 furnishings, and the existing
node/work/wall/output allowances. Each nested step shares the enclosing budget.
A node limit preserves only a compiler-accepted whole incumbent; it withholds
optimality. Exhausted work, cancelled search or a failed read returns no plan.

Optimality is limited to allowed origins/rotations and the **selected existing
deterministic connection compiler**. Corridors are not rerouted according to
terrain cost. This is not minimum cost over every possible corridor graph,
mining time, labor, structural support or material value. The separately selected
`connection_strategy` remains unchanged.

A wall-shaped map tile is not proof of mineability. Aquifers, adjacent hazards,
native pathfinding, present safety, completed construction and room assignment
are not established. The result cannot grant permission to dig, place or retry
anything. The separate original survey-to-residual-excavation workflow retains
its own validation and native review requirements.

## Executed tests

Forty-three warnings-denied Python tests cover this increment: 15 cost-layout,
15 raw-map observation and 13 TCP/CLI tests. Independent enumeration checks
128 assumed binary maps, 18 two-area cases and 64 raw wall/floor maps. All seven
exports are checked for native request and saved input, including strict
refusal of node-limited results. The CLI tests execute 27 joined synthetic-peer
sessions through the actual MapClient, framing and commands. No replacement
client, backend, compiler, AST extraction or import adapter is used.

Faults include lost replies, source/nonce/profile changes, malformed or wrong
fortress captures, cancellation at every small-case checkpoint, authorization
replacement after solving, work/deadline loss and short output. The peer is not
DFHack: these tests establish Python read/compute/publication behavior only.
A separate exact-byte comparison preserves 108 legacy offline layout outcomes,
108 legacy surveyed layouts and their 108 original CLI reports.

```sh
python3 -W error -m unittest discover -s tests -p 'test_room_layout_costs.py' -v
python3 -W error -m unittest discover -s tests -p 'test_room_layout_excavation_observed.py' -v
python3 -W error -m unittest discover -s tests -p 'test_room_layout_observed_costs_cli.py' -v
```

The CLI suite was executed in two completed bounded invocations. An earlier
aggregate invocation hit the execution environment's wall limit and is not
counted as a completed run. No real DFHack producer, native/Rust build,
live-fortress, full-repository or production qualification is claimed.
