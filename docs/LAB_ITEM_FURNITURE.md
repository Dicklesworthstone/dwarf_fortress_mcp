# Item-backed furniture in the laboratory

`place_furniture` connects manufactured furniture to an installed building in
the deterministic laboratory. It uses the existing eleven-tool plan, commit,
wait and prove workflow. These rules describe the reference simulator; they
grant no native Dwarf Fortress mutation or compatibility admission.

## Provision a complete furnished room

Pass this JSON as the existing `fortress.plan` tool's `blueprint` argument:

```json
{
  "template": "provisioned_room",
  "layout": {
    "template": "bedroom_cluster",
    "origin": [2, 6, 10],
    "rooms": 2,
    "room_size": [3, 3]
  }
}
```

The planner reuses eligible loose beds in canonical item order. If two beds are
missing, it requests two carpentry units from the observed finite WOOD supply.
Ordinary excavation steps establish every original room, doorway and corridor.
Their sealed deadlines scale with the number of tiles using the reference
model's bounded timing contract, including for long, narrow rooms.
Each placement waits for its room excavation and all requested furniture
production to verify, then allocates its exact observed or future produced item.
The normal reviewed `plan → commit → wait` sequence executes these dependencies.

The planning response includes required quantities, eligible existing items,
manufacturing deficits, procurement targets, exact item allocations and their
step dependencies. Future item identities derive from the original production
step key and completion ordinal; they are predictions until those items exist.
Planning and forecasting do not change the source fortress.

The final goal requires **every original excavation region and every exact
installed furnishing**. Procurement stock targets are intermediate conditions:
installation spends loose furniture inventory, so those stock minima are not
retained as the final room goal. Completed production alone cannot satisfy it.
An already-excavated room can still be furnished. If eligible items already
cover the layout, no new carpentry or WOOD evidence is needed.

For a dining room, use this layout inside the same wrapper:

```json
{
  "template": "provisioned_room",
  "layout": {
    "template": "dining_hall",
    "origin": [2, 6, 10],
    "width": 5,
    "height": 5
  }
}
```

It installs one table and one adjacent chair on distinct interior tiles.
Bedroom clusters install one bed per room. Doors remain available through
explicit `place_furniture` actions; the room compiler does not add them.

Optional top-level `prerequisites` uses the closed production setup grammar,
including `assign_labor` and explicitly supplied `workshop:Carpenters` sites.
Required setup becomes ordinary authorized labor/build actions. Ready capacity
is reused; no arbitrary workshop location is chosen. The original request
retains even currently unused setup choices for exact replay and recovery.

Aggregate stock without eligible exact items cannot satisfy a furnishing slot.
When legacy stock is present, the procurement target is the current aggregate
plus the missing eligible item count. This creates real new items without
claiming that old stock establishes their identity or availability.

The wrapper supports at most sixteen furnishing slots and sixty-four combined
setup, production, excavation and placement steps. Existing occupied footprints,
overlapping workshop sites, unknown supply or terrain, insufficient material,
unsafe excavation and unsupported layouts refuse explicitly. Valid cancelled
placements release their old target and exact item for a newly reviewed plan.

New intake seals `planner="provisioned_room_v1"` and the entire canonical
request. Archived sources must retain that exact marker; replay never silently
upgrades an older room blueprint. Changed anchors require a reviewed replay
from current inventory and occupancy. Existing direct `bedroom_cluster` and
`dining_hall` templates preserve their earlier abstract Build interpretation.

This wrapper establishes excavation and furniture installation. It does not
create DF room ownership/assignments or prove connectivity to the existing
fortress beyond the existing layout's bounded geometry and hazard checks.

## Request and authority

Submit a semantic action with one exact canonical item and one target tile:

```json
[
  {
    "action": {
      "kind": "place_furniture",
      "furniture": "bed",
      "item": "123456789",
      "location": [2, 4, 10]
    }
  }
]
```

Replace the example item ID with an eligible item observed through the normal
query surface. The closed furniture kinds are `bed`, `door`, `table` and
`chair`. IDs use canonical decimal strings. The reviewed steps expose the
selected item, kind, tile and exact completion predicates. Planning and
forecasts do not allocate inventory.

The action is Guarded and requires Construct authority over both the exact
item and the target tile. Ordinary checkpoint, plan validation, spatial leases
and execution rules remain applicable. Another item cannot be substituted
after review. A changed source anchor requires explicit reviewed replay,
because current occupancy and inventory depend on complete domain scans.

## Manufacturing, allocation and installation

New `carpentry/1` output is a generation-one singleton item with its original
work-order identity, completed production ordinal, closed item token and WOOD
material. It starts with explicit `inventory_state="loose"` and
`allocated_to=null`. The aggregate furniture stock includes this item.

At dispatch, the reference model validates the original producer and item,
the complete attachment census, a qualified stock count and an observed,
unoccupied floor tile. In the same transaction it:

1. Decrements the appropriate loose furniture stock by one.
2. Creates the key-derived planned building with `placement_model="furniture/1"`,
   exact `source_item`, source generation and single-tile footprint.
3. Marks that same item `allocated` with a reciprocal building reference.

Construction uses the existing reference construction duration. Each advance
revalidates reciprocal custody and occupancy. Completion changes the item to
`installed` at the building's observed position. It does not consume another
stock unit. No hauling route, worker construction capacity or room ownership
is inferred by this model.

The default proof includes the complete building, exact kind and footprint,
placement model, source item and generation, and the installed item's identity,
position and backlink. Physical-work inspection independently revalidates
original identity and custody before reporting quiescence. A matching
`construction_stage` word alone cannot establish placement completion.

A later goal can retain an existing completed installation without placing it
again. Mandatory completed-furniture claims still undergo the same original
generation, production provenance and reciprocal-custody checks, even when the
new plan contains no placement action for that building. One bounded attachment
census serves at most sixty-four mandatory installations; optional `Any` and
`Not` branches do not become unconditional requirements. Reusable predicates
preserve the original exact placement proof bytes.

## Cancellation, conflicts and recovery

Cancelling unfinished construction returns the same item to loose inventory,
clears its backlink and refunds one stock unit. The building retains a
cancelled/released historical record; that record no longer occupies its tile.
Repeated cancellation has no further effect, including after another action
has reused the item. Completed furniture is not undone or refunded by cancel.

An item already allocated or installed cannot serve a second building.
Unknown, stale, malformed or contradictory custody refuses the operation.
Unqualified occupancy, a taken tile, identity collision, arithmetic overflow
or an exhausted bound also refuses. The adapter publishes no partial world on
failure. Durable snapshots retain allocation and item identity, and original
plan archives preserve the typed action and original key across restart.

Inventory and occupancy inspection admit at most 65,536 canonical entities.
Construction scans consume the bounded effect-advancement work allowance.
No unbounded search or arbitrary script is introduced.

## Compatibility

The original `build` action remains abstract in the laboratory. Its material
selector, canonical bytes and archived interpretation are unchanged. Exact
placement is a distinct action tag appended to the existing canonical and
archive codecs.

Historical carpentry items without both custody fields are ineligible for
placement. They are not silently upgraded to loose inventory. Partial or
unqualified custody is an explicit refusal. Aggregate stock may contain those
legacy items, so the eligible item census is a lower bound on stock rather
than a substitute for the stock ledger.

Native semantic routing explicitly refuses this laboratory action. Canonical
laboratory item custody does not establish native DF item identity or eligibility.
