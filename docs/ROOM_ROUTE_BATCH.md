# Whole-room conditions at a route placement frontier

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (remain open).

`room_route_frontier.gate_frontier` adds the complete original room condition to
an execution owner's already verified route/construction frontier. Its input is
not client-issued evidence: the original owner still authenticates the complete
placed prefix, current inventory and all remaining deliveries.

The gate revalidates the full RoomRouteHandoff, exact original route review and
plan, prefix order, next step, source identity and raw paused map. Every required
floor, corridor, doorway and bedroom wall is assessed on the SAME map used for
routes. Historical placements do not remove room requirements. Floor occupancy
alone is allowed; required walls must still be present, dry, undesignated and
unoccupied. Hidden/missing cells remain unknown.

A matched room cannot turn a blocked route or unfinished predecessor into ready.
When the underlying frontier is ready, contradicted room terrain narrows it to
`room_blocked`, and unknown terrain to `room_unestablished`. Evidence binds the
parent room/handoff identities, prior execution evidence and full room diagnosis.
It excludes computational allowances so narrowing the remaining budget does not
change identical semantic evidence. The original work counter continues; no
new work allowance is created. At most 64 deficit rows are shown with exact
counts for the complete required mask. All checks precede publication.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_room_route_frontier -v
```

Fifteen actual Python tests pass without skips, using the real room compiler,
map decoder, request/plan/review/handoff codecs and this gate. Tests cover every
required cell in a bedroom fixture, all rotations, 32 furnishings, hidden/missing
cells, liquid/designation/occupancy, source changes, changed intent/prefix,
malformed inputs, exact inclusive work limits and every guard interruption.
The prior route/construction report is an injected producer port in this unit
suite; it does not qualify receipt authentication, native acquisition or custody.

This first increment is a pure additional condition. Native dispatch integration
and durable parent retention are separate. No protocol, native authority,
production admission, Rust/MCP or live-game qualification is changed.
