# Observe construction after the final original placement

Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; df-franken-storage-mvcc-54h.4.

`route_furniture_final.read_frontier(batch, authority)` inspects a complete original
route batch even when there is no next placement. The existing held owner must
prove every original step is registered and Placed, with no pending or stopped
work. Original review bytes, item/material/subtype/target constraints, receipt
keys, chronology, native generation and complete local inventory remain bound.
No caller-selected subset or synthetic temporal goal is accepted.

The read-only client uses the existing construction-monitor framing, paged
operations decoder, capture digest and release protocol. It binds exactly six
native read methods: furniture Handshake/QueryPlacement, operations Handshake/
ReadObservation and map Handshake/ReadObservation. All original receipts are
queried before and after two equal map endpoints around one complete released
operations roster. Every original native source remains pinned. Unresolved
native work, lost final receipts, release errors and changed sources refuse.
The connection closes before map/operations decoding and predicate execution.

All original furnishings are evaluated together by the existing construction
prefix predicate. Pending construction, unknown work, removal and missing or
changed objects remain distinct. `construction_quiescent` establishes only that
the selected original furnishings are installed without held/item-linked work
at this observation. It does not prove a stability window, goal completion,
continuous quiescence, native route safety, causal effects or current usability.
The existing route-furniture completion monitor retains its original authority
for sampled stability and durable goal history. Every result leaves placement
confirmation null and advance_allowed false.

Bounds: 1..32 receipts, unchanged native roster/page/frame limits, 16,384 map
tiles, 96 KiB final-frontier result. Six million analysis work units are reserved
once without refund, with 100,000 units retained for final custody/publication.
Actual transport bytes/calls and other traversals consume the same caller budget.
A complete minimum read bracket is admitted before contact; extra pages can still
exhaust the existing allowance and refuse without another acquisition.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest test_route_furniture_final -v
```

All 14 tests pass. They execute actual TCP framing, all six fixed bindings,
receipt/map/operations codecs and the shared predicate against bounded joined
synthetic peers. The 32-original-furnishing case includes a multi-page roster
and 64 exact receipt queries. Source, nonce, final receipt, release, inventory,
world/tick/region, malformed bytes, revocation and output-budget refusals pass.
All seven unchanged production dependency blobs were matched to GitHub before
execution. Original batch custody is an explicit test port; no real private
batch, DFHack SDK/game, Rust/MCP or full-workspace qualification is claimed.
Clock integration is a separate increment. No native protocol or admission changes.
