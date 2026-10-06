# Bind completed room terrain to exact furnishing intent

The new `bind_room_terrain.py` command joins an existing room allocation to the
**original whole-room terrain journal** and one fresh native map capture. It
neither reallocates items nor creates effects. A floor-only goal, residual
blueprint, dig designation receipt or incomplete room monitor cannot substitute.
The original room plan, all slots/constraints/exclusions and the original fixed
stability policy remain intact. Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5`.

## Binding

First satisfy the original `track_excavation.py start-rooms` monitor. Allocate
its exact original RoomPlan using `plan_rooms.py allocate --emit room-handoff`.
The inventory capture must be at or after the retained terrain completion tick.
Switch back to the existing isolated map-read environment (remove allocation and
mutation profile variables):

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<map query credential>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000
python3 scripts/bind_room_terrain.py --journal "$HOME/room-monitor/terrain" \
  --room-handoff room-furniture-handoff.json > terrain-furniture-handoff.json
```

Exit zero emits exact canonical `dfmcp.terrain-furniture-handoff/1` bytes. Check
exit status before consumption; exit two is a refusal, not a partial handoff.
`--emit report` adds a bounded Agent Turn Packet. The complete report is reserved
even for the narrow export. Broken/short output never retries or emits a second
object. The command uses one shared deadline/work allowance and at most four
native frames (two bindings, handshake, map observation), without reconnects.

The original private journal is opened read-only, locked and replayed in full.
Its exact path, parent/file identities, full byte SHA-256, head, incarnation,
goal digest, source and completion policy are retained as `TerrainOrigin`.
The new handoff embeds the **raw fresh map capture**, not just an assertion that
terrain was clear. It regenerates the room plan and re-decodes the entire map.
All original floors and required bedroom walls must hold; all furniture targets
must be visible, dry, undesignated and unoccupied. Non-target floor occupancy
is not invented as a furnishing blocker.

Endpoint, fortress, dimensions, software and map generation must match the
original terrain history. Inventory and map generations remain independent.
The inventory tick must fall between historical terrain completion and the
fresh map; the original maximum sample gap bounds completion-to-map age.
No goal deadline is renewed. No digest is a signature or acquisition attestation.

The reference alone proves neither custody nor history: a consumer must call
`TerrainOrigin.open` to replay and hold the original journal. Missing, copied,
replaced, rehashed or rewritten original custody fails verification. Keep that
journal; deleting the temporary allocation input after binding is safe.

This first increment adds the closed binding artifact and its executable producer.
Legacy furniture-batch consumers deliberately reject it rather than ignoring its
extra evidence. Integration into placement and construction owners is separate.
A sampled clear room is not current/continuous terrain, pathfinding, structural
safety, item reservation, native room assignment or completed-room proof.

## Executed evidence

Sixteen methods passed on Python 3.13.5 using actual private journals/reducers,
raw independent map fixtures, a joined fragmented TCP peer and a real CLI process.
The CLI case preserves 32 slots and 646 exclusions. Negative tests cover wrong
profiles, incomplete/cancelled goals, different room geometry with identical
furniture, stale allocation, source/clock changes, lost replies, changed custody,
revocation, output bounds, short writes, raw-data revalidation and shared budgets.
Allocation input is generated locally with the real allocator, not acquired from
a native inventory in these tests. No placement, Rust/MCP, live DFHack/SDK or
full-repository qualification is claimed. No production admission is changed.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests \
  python3 -m unittest test_terrain_furniture_handoff -v
```
