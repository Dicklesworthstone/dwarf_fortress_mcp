# Delivery-aware route-furniture review creation

The existing read-only review CLI now connects the shared
`route_furniture_sequence.allocate_sequenced_routes` compiler to native-capture
acquisition, effective-request retention and final review publication. The
compiler landed separately in e0430a35; this integration reuses it unchanged.
No second sequencing engine, allocation pass, native method or mutation permit
is introduced. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5;
df-franken-storage-mvcc-54h.4.

## Opt in before a new review is created

```sh
python3 scripts/review_route_furniture.py create \
  --request-file furnishings-request.json \
  --origin 0 0 5 --size 64 64 1 \
  --max-route-steps 1000 --sequence-deliveries --timeout-ms 10000
```

Coordinates and paths are illustrative. Use the existing isolated map/inventory
read credentials and environment documented in ROUTE_FURNITURE_REVIEW.md. The
command obtains the same single map-before/inventory/map-after bracket as before;
sequencing needs no further native calls. Sources close before computation.

Without the flag, the existing allocator and packet remain unchanged. With the
flag, allocation selects each item once, and the compiler checks delivery routes
with earlier target tiles removed. It preserves a workable canonical order or
adds dependencies to encode a complete viable ordering, retaining every original
constraint and selected item. This is a static candidate model, not a native
hauling or safety prediction. See ROUTE_FURNITURE_SEQUENCE.md for its exact model.

The returned effective request, result and review must stay together. The saved
review contains the strengthened dependency graph, so the existing fixed-order
batch follows it. The result's delivery_sequence retains the original request
and identities, ordered path witnesses and scoped outcome. Changed dependencies
change request/plan/review identity; no existing review or batch is rewritten.
The CLI rejects --sequence-deliveries with check before reading files or making
native contact. Check continues to verify the exact saved selections and graph.

A delivery_order_blocked result is an established diagnostic (exit zero), not a
usable plan: review and result.plan are null. It says no permitted order of the
chosen assignment works in this model, not that all furniture supply is absent.
Callers must check result.status == allocated before retaining a new review.
Unknown targets and shortages preserve their existing outcomes. All failures
remain bounded, sanitized and non-effectful; no partial executable subset is
exported. Fresh per-step native review and original receipt custody still apply.

## Budgets and publication

The existing 100,000-unit export reservation remains separate from the solver.
One remaining solver allowance spans allocation, sequencing and forward path
verification; no stage replenishes it. The opt-in accepts an actual Boolean in
the Python API and is validated before acquisition. Complete output must fit the
existing 96-KiB packet bound. Authority and deadline are checked after rendering;
a cached valid review cannot be published after read permission is revoked.

## Executed checks and limitations

Forty Python test functions pass: 26 unchanged cross-floor component tests and
14 new sequencing/workflow cases. New cases include 512 obstacle maps and 240
seeded dependency graphs checked against independent coordinate BFS/permutation
oracles, 32-slot review export, exact shared work limits, unchanged defaults,
blocked-plan refusal, effective request/review identity, retained-review checking,
cross-floor limits, original file preservation and final publication refusal.
Four deliberately weakened implementations fail regression assertions.

The workflow tests load complete production runtime/CLI source without source
rewriting, with an explicitly injected inventory dependency. Acquisition, budget
and the outer packet are doubles; selector/sequence/review codecs, create/check/
finish and CLI/file-read logic execute. This is NOT a native TCP test, a complete
runtime-import/serialization qualification, private batch-parent execution,
Rust/MCP, real DFHack SDK/game, or full-workspace qualification. No production
admission is changed. The runner records exact production/test hashes:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_furniture_delivery_workflow.py --mutations
```
