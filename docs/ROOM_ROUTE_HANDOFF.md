# Original rooms with route-aware furnishings

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` and
`dfmcp-wp-pln01-spatial-blueprint-layout-pwd` (remain open).

`room_route_handoff.RoomRouteHandoff` retains the complete original `RoomPlan`
and immutable route-furniture `Review` together. This closes the data/diagnostic
seam between room intent and route-aware selection; it does not extend a native
placement journal or establish private batch custody.

The existing codecs are unchanged. Decoding regenerates the room plan, validates
the complete route review and checks byte-exact furnishing-request equality,
including dependencies, selectors, exclusions and vertical-distance constraints.
An optimizer-strengthened request cannot be silently attached to a different
original room plan. The fixed-order allocator is the appropriate new-selection
mode when exact parent request preservation is required.

The route capture must cover every original floor, doorway, corridor and required
bedroom wall, not merely the furnishing targets. Region bounds are checked before
expansion; existing tile, plan and review limits remain. The combined canonical
JSON is bounded to 100 KiB and selected by a domain-separated expected digest.
Keep that digest separately. A digest identifies bytes, not authority or native
acquisition. The original room and child review bytes are preserved exactly.

## One combined current-model check

`revalidate(handoff, candidates, native_types, raw_map, source, guard)` performs
three checks against the same supplied map/source:

1. Revalidate every original selected item with the unchanged route-review code.
2. Assess every original room floor and separating wall in the raw map/1.5 bytes.
3. Replay the complete original delivery order with the unchanged suffix checker,
   treating earlier targets as blocked. No item or dependency is replaced.

Terrain classification matches the existing whole-room condition: floors require
shape 3, walls shape 2; liquid/magma and designations contradict the condition;
occupied walls contradict it; hidden and unallocated cells are unknown. Floor
occupancy is reported, not interpreted as missing terrain. Routing separately
applies its conservative occupancy policy. At most 64 deficit rows are displayed,
but all required cells are evaluated and omitted counts remain explicit.

The top-level `plan` is nonnull only for `ready_in_captured_model`. A known
contradiction produces `blocked_in_captured_model`; otherwise missing evidence
produces `unestablished`. The nested route-review report retains its own narrower
individual-route result and original-plan history. It is not a replacement for
the combined status. Room terrain can block a result even when every individual
route exists; sequential target removal can block it even when room terrain and
all current individual routes match.

This is for an unstarted original plan. It does not authenticate a placed prefix,
resolve outstanding effects, create room assignments, prove construction complete,
reserve items or enforce a native commit gate. Current source acquisition and
authority are the caller's responsibility. No temporal stability or live hauling
safety follows from one capture. Existing receipt-backed batch checks remain
necessary; no production admission is widened.

One structural allowance covers original decoding, region checks, all individual
routes, terrain assessment, complete delivery replay and publication. Work
exhaustion or caller cancellation raises without returning partial success.
Complete results are bounded to 128 KiB rather than omitting an original room.

## Executed component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error \
  -m unittest test_room_route_handoff -v
```

15 actual Python tests pass, zero skips. These load the complete existing room
compiler, route allocator, review codec, sequencer and suffix implementation.
All 11 reused modules were reconstructed and verified against their upstream
Git blob IDs before execution; they are not modified by this increment.
Cases include four rotations, all 32 furnishings, omitted geometry, changed
constraints, sequential blockage, missing original items, source changes,
every prefix and single-byte corruption of a complete handoff, rehashed false
claims, exact work limits and sampled callback interruption points. Inputs are
independently encoded synthetic maps and supplied candidate facts, not a real
DFHack inventory. No native TCP, game, SDK/plugin, Rust/MCP, private-parent batch
or full-workspace qualification is established by these checks.
