# Furniture supply across observed floors

The Python route allocator, original review and placement-frontier checker now
support furniture stored on a different z-level from its target. The graph already
modeled complementary stairs, but the endpoint selector previously discarded all
cross-level candidates before matching. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5
and df-franken-storage-mvcc-54h.4.

## Explicit request, unchanged default

Each `dfmcp.furniture-request/1` slot may specify `max_vertical_distance`, an integer
from 0 through 127. Omission and zero retain the old same-level endpoint constraint.
Zero is omitted from canonical output, so existing request and retained-review
identities do not change. Positive values participate in the request digest and
therefore in every allocation/review/frontier commitment. Older closed decoders
reject the additional field; they must not silently ignore it.

```json
{
  "schema": "dfmcp.furniture-request/1",
  "world_folder": "example-fort",
  "site": 1,
  "slots": [
    {
      "name": "bedroom-bed",
      "kind": "bed",
      "target": [20, 20, 7],
      "max_distance": 100,
      "max_vertical_distance": 3
    }
  ]
}
```

This is illustrative intent, not an observed fortress. Pass such a request to
`route_furniture_allocation.allocate_routes` or the existing
`allocate_route_furniture.py` read-only workflow. Its selected map region must
include both endpoints and a complete usable route. The non-route Manhattan
allocator explicitly refuses positive vertical limits, even when it could choose
a same-floor item: it cannot satisfy an observed-route request without terrain.

The limit is the absolute z difference between endpoints, **not** the number of
vertical moves or an envelope restricting intermediate floors. As before, a
same-floor route may detour through another floor and return. `max_distance` is
the Manhattan lower bound: `abs(dx)+abs(dy)+abs(dz)` for cross-level endpoints.
The separate `max_route_steps` bounds the actual shortest route, including detours.
All bounds are inclusive. Material, subtype, kind, exclusions, dependency ordering
and exact item identity remain independent requirements.

Only existing observed dry floor/stair cells qualify. Vertical edges require a
lower up/up-down stair and an upper down/up-down stair. Hidden, unallocated,
liquid-filled, building-occupied, unit-occupied and walkability-unknown cells do
not connect floors. No ramp, diagonal, external-region or inferred stair is added.
These remain candidate routes, not native unit pathfinding or safety evidence.

The whole-plan solver still maximizes cardinality, refuses partial executable
plans on shortage, and minimizes total shortest-route steps followed by the exact
lexical slot/item-ID vector. Each selected full path retains its streaming digest.
Bounds remain 32 slots, 65,536 candidates, 16,384 map cells, 128 cells per axis,
20,000,000 shared work units and 49,152 pure-result bytes.

## Retained review and placement integration

An original `Review` retains the complete normalized request, including each
positive vertical limit. Reopening, fresh route revalidation and native before-
capture binding use the same endpoint rule. Moving the selected item to another
allowed level is permitted only when a fresh complete route still passes. A move
outside the original limit, changed material/type or a broken stair rejects that
selection; a convenient replacement item is never substituted.

The placement frontier validates the exact receipt-backed fixed-order prefix.
A previously placed original item need not remain loose inventory, but every
remaining original selection still undergoes fresh route and endpoint checks.
Unknown, refused, cancelled or unverified predecessor records cannot unlock later
work. Existing placement authority, independent acquisition, native preflight and
private batch custody are unchanged. A raw `FurniturePlan` alone does not retain
route constraints: use the original route-review workflow rather than treating an
unbound plan as route enforcement.

## Executed checks

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_cross_level_furniture.py --mutations
```

All 26 Python test functions pass. They execute the actual selector, route solver,
canonical request/review codecs, native value codec and placement-frontier checker.
Coverage includes all 81 stair-shape pairs, 180 unpruned random assignment oracles,
127-level endpoint separation, 32-slot bounds, exact work limits, legacy canonical
request/result/review equivalence, moved originals, blocked stairs, fixed-prefix
receipts and no implicit substitution. Six weakened implementations fail the
expected regression assertions; syntax/import failures are not counted as kills.
The runner records the exact executed production/test hashes and refuses changes
during execution.

Map captures, inventory candidates and placement receipts are explicit test data.
No native TCP client, private batch parent, real DFHack SDK/game, Rust/MCP or full
workspace qualification was executed for this change. Existing protocol encoding,
mutation permissions and production admission remain unchanged.
