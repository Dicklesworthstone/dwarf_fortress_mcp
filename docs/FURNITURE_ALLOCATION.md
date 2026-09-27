# Inventory-driven whole-plan furniture allocation

`scripts/furniture_allocation.py` chooses distinct exact items for 1..32 requested
bed/chair/table targets. It emits the existing `dfmcp.furniture-plan/1` format used
by `furniture_batch.py`, or a shortage with **no partial executable plan**.
This increment is an executed pure allocator; native acquisition is not provided
by the allocator itself. It does not reserve items, prepare/commit a placement,
unpause the game, or qualify a production profile.

## Request and pure API

```json
{
  "schema": "dfmcp.furniture-request/1",
  "world_folder": "region1",
  "site": 2,
  "slots": [
    {"name": "bed", "kind": "bed", "target": [15, 15, 2]},
    {"name": "chair", "kind": "chair", "target": [18, 15, 2],
     "material": [419, -1], "max_distance": 100, "after": ["bed"]},
    {"name": "table", "kind": "table", "target": [20, 15, 2],
     "after": ["chair"]}
  ],
  "excluded_items": [123]
}
```

IDs, material pairs, fortress and coordinates are illustrative, not defaults.
A slot can constrain the exact material type/index pair, exact subtype and maximum
horizontal Manhattan distance (0..65,532). Missing material/subtype means any
candidate value; every candidate must be on the target's z-level. Exclusions only
remove candidates and do not claim a global reservation. The original plan
compiler validates names, unique targets, coordinates and dependency acyclicity.
Dependencies still mean historical placement registration, not finished furniture.
The complete normalized request must fit 16 KiB.

```python
from furniture_allocation import Candidate, Request, allocate
request = Request.decode(request_bytes)
# The pure caller supplies already selected candidate facts, not a trusted world.
candidates = (
    Candidate(42, "bed", (10, 10, 2), (419, -1), -1),
    Candidate(43, "chair", (11, 10, 2), (419, -1), -1),
    Candidate(44, "table", (12, 10, 2), (419, -1), -1),
)
result = allocate(request, candidates)
# Only an allocated result contains result['plan']. Later batch review remains
# mandatory: these values never authorize a placement or establish eligibility.
```

The pure API checks all supplied candidate identities and values, including
excluded/irrelevant candidates. It does not authenticate the caller's facts or
verify that those facts came from the requested fortress. The native read adapter
must establish those associations before calling it. Every result makes its
non-authority and non-reservation scope explicit.

## Global allocation, objective and shortages

A generic slot must not consume the only material-compatible item for a more
constrained slot merely because it appears first. The implementation solves the
whole bipartite assignment, rather than greedily assigning each target.

The objective is lexicographic: first minimize total same-level Manhattan
distance; then minimize the exact item-ID vector in lexical **slot-name** order.
Dependency execution order is separate. Distance is a geometric heuristic, not a
route-length estimate or proof of worker accessibility. Integer costs represent
both objective levels exactly, including maximum-width item IDs, without floats.
Input row order does not affect the result.

For N slots, keep each slot's N best compatible items by distance and ID. This
preserves an optimum: when a slot uses an item outside its best N, at least one of
those N better items is unused by the other N-1 slots and can replace it. Thus the
solver handles at most N*N distinct reduced items, not a 32-by-65,536 dense matrix.
Full candidate counts are retained. A maximum matching is checked before running
the rectangular Hungarian minimum-cost assignment.

When no perfect assignment exists, a Hall-deficiency witness names a set of slots,
all their compatible candidate IDs and their shortage. Every such set has fewer
than N neighbors, so none of its candidate lists was trimmed; its witness covers
the complete supplied candidate graph. Counts are not substituted for matching:
several individually satisfiable slots can still compete for too few items.
The result includes the maximum assignable count, but never an executable subset
that silently drops the other targets or weakens constraints.

## Bounds and validation

At most 65,536 supplied candidate items, 32 slots, 4,096 explicitly excluded IDs,
1,024 reduced item identities, and 49,152 serialized allocation-result bytes are
accepted. The normalized request's 16-KiB bound usually limits large exclusion and
dependency lists sooner. An injected guard runs during scans, matching and cost
optimization; budget refusal returns no partial result. The pure API has finite
structural bounds even when no external deadline guard is supplied.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest test_furniture_allocation -v
```

All 14 test functions pass. Tests exhaust all 4,096 three-slot/four-item bipartite
graphs against an independent cardinality/cost/permutation oracle; compare 150
random full inventories against an unpruned oracle; execute the 65,536-item,
32-slot bound and the 1,024-item reduced union; and check deterministic tie breaks,
material starvation, Hall witnesses, strict parsing, interrupted budgets and
actual existing-plan compiler compatibility. This is pure Python evidence, not
DFHack, TCP, live-game, Rust/MCP or production qualification.
