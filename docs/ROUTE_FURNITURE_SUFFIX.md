# Fresh verification of the original delivery suffix

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`;
`df-franken-storage-mvcc-54h.4`.

`route_furniture_suffix.assess_remaining` checks a retained furniture plan's
exact remaining canonical order on fresh map/1.5 bytes. Individually reachable
items do not imply a deliverable sequence: placing an earlier target can remove
a later route or cover an item which has not yet been hauled. The checker reuses
the existing sequence compiler's target-removal model, BFS and full-path
commitments, but never calls its allocator or order optimizer.

The caller supplies the immutable original `FurniturePlan`, its complete
receipt-verified prefix as a tuple of slot names, and current positions of
exactly the remaining original items. Prefix shape is checked here, but native
receipts, current source authority and original material/subtype/distance/level
selectors must be checked by the execution owner. Names, hashes and positions
alone cannot establish custody or authorize a placement.

Every prior prefix target remains removed even when its current terrain appears
open. Each remaining original delivery is replayed, adding its target to the
removed set before testing the next delivery. The original per-route limit is
not relaxed for detours. Targets and sources must be established by the strict
existing map decoder; hidden, occupied or outside-region endpoints do not turn
into an absence proof. Only compatible observed stairs connect levels.

Results use `dfmcp.furniture-delivery-suffix/1` and contain the original plan
identity, map identity, prefix, exact remaining order and current item positions.
`ready` includes every delivery witness. `blocked` identifies the first failed
slot but publishes no partial route certificate; it says nothing about alternate
orders or assignments. `unestablished` names unavailable endpoints without a
success/absence claim. No result is a native pathfinding or future-route promise.

The evidence digest covers all semantic inputs and output, excluding only work
accounting. Equal observations can therefore bind the same execution seal under
a smaller remaining allowance. One bounded work counter covers decoding, graph
construction, every BFS and commitment; no stage replenishes it. Cancellation,
malformed input, work exhaustion and output overflow refuse without publication.
The output is limited to 49,152 bytes and the plan to 32 original steps.

## Component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s scripts/tests -p 'route_furniture_suffix_tests.py' -v
```

Fourteen actual Python tests pass, including 600 independently encoded
coordinate-BFS cases, every unfinished prefix of a 32-step plan, changed source
positions, detour limits, covered source items, stairs, unavailable endpoints,
strict identity/prefix checks, malformed captures, exact work limits and every
cooperative cancellation callback. Four deliberately weakened implementations
(forgetting the prefix, forgetting future target removals, relaxing the limit,
and publishing a partial certificate) fail assertion-based regressions.

The four unchanged production dependencies were reconstructed from GitHub and
matched to their exact Git blob identities before execution. These are component
tests with synthetic captures, not native I/O, DFHack gameplay, Rust or full
workspace qualification. The pure checker alone is not an execution gate;
placement-frontier integration is the subsequent boundary. No native protocol,
mutation authority, compatibility entry or production runner is changed.
