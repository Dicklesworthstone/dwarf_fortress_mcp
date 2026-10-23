# Item-backed furniture in the laboratory

`place_furniture` connects manufactured furniture to an installed building in
the deterministic laboratory. It uses the existing eleven-tool plan, commit,
wait and prove workflow. These rules describe the reference simulator; they
grant no native Dwarf Fortress mutation or compatibility admission.

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
