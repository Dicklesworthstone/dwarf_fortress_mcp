## Tile-budgeted cost-aware corridors

Add opt-in `bounded-cost-with-original-candidate/1` corridor routing. Retain
cost/tile tradeoffs during path search instead of discarding every alternative
when the unconstrained cheapest route is too long. Original masks, project
limits, shared work/cancellation and legacy strategy bytes remain unchanged.
See `docs/ROOM_BOUNDED_COST_CORRIDORS.md`.
