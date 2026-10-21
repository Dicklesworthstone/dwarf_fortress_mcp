# Joint original-room completion after route-backed placement

Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-10 (remain open).

`room_route_completion` binds the complete original route-batch construction
goal to its retained RoomRouteHandoff. It rejects furniture-only origins,
substituted parents, incomplete original selections and changed native source
identities. The original deadline, cadence, sample allowance and stability
requirements are retained rather than renewed during replay.

Every sample contains two identical paused map endpoints bracketing the complete
receipt-linked operations sample. All original receipts, map/build/operations
incarnations, fortress, dimensions, capture region, tick and identity horizons
must agree. The map must preserve every original floor/corridor/doorway/wall.
Every original furnishing target additionally needs visible building occupancy;
the exact building identity still comes from the receipt-linked roster, not
from the map occupancy tag. No item substitution or route reallocation occurs.

The existing construction_plan reducer owns ONE stability window for all of
these conditions together. A room failure resets the entire unfinished streak.
An otherwise acceptable map change at the same tick also resets it, even when
the operations roster did not change. Alternating successful room and furniture
observations cannot combine into success. Original invalidation, failure,
deadline and lifetime sample-limit outcomes retain precedence. Interrupted
reads cannot continue an older streak. Cancellation concerns this monitor only.

The result is historical sampled evidence, not an atomic cross-profile snapshot,
continuous wall preservation, native room assignment, current usability,
pathfinding, permission to retry placement or production admission. The complete
original batch must remain open and rechecked by the enclosing workflow.

## Executed component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_room_route_completion -v
```

14 actual Python component tests pass, zero skips. They execute the new goal,
sample and joint reducer, the unchanged construction_plan temporal engine and
real route-map, request, plan and review codecs. The existing construction_plan
source was reconstructed byte-exact and verified against Git blob
`ee921d44a5b2f9e9dbe2f672774928874fd3652a` before execution. Cases cover all 32
original furnishings, joint versus alternating success, hidden/wrong terrain,
missing map occupancy, same-tick changes, advancing ticks, sample gaps, original
deadlines, every sample/goal truncation, source mismatch and cooperative aborts.

Native receipt decoding/assessment, room compiler/terrain diagnosis and original
batch custody are explicitly injected producer ports in these focused tests.
They do not establish those producers' correctness, filesystem custody, native
acquisition, DFHack, Rust/MCP or full-workspace qualification. This first increment
is the reusable completion condition; executable acquisition and durable monitor
ownership are separate integrations. Existing monitor formats remain unchanged.
