## Complete room layouts with tile-budgeted cost-aware corridors

The existing layout command now accepts bounded-cost-with-original-candidate/1
with a complete cost objective and retains that exact connector in RoomPlan.
An eight-bedroom/two-hall regression retains all 28 furnishings and changes five
charged corridor tiles to one at the original 512-target limit. Default request
bytes remain unchanged. See docs/BOUNDED_COST_LAYOUT.md.
