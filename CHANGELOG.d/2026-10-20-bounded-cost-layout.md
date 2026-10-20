## Complete room layouts with tile-budgeted cost-aware corridors

The existing layout command now accepts bounded-cost-with-original-candidate/1
with a complete cost objective and retains that exact connector in RoomPlan.
An eight-bedroom/two-hall regression retains all 28 furnishings and changes five
charged corridor tiles to one at the original 512-target limit. Default request
bytes remain unchanged. See docs/BOUNDED_COST_LAYOUT.md.

Observed layout now retains the same exact bounded connector while deriving its
costs from raw map bytes. The original padded compiler emits a one-wall handoff
for the complete 512-target/28-furnishing fixture. All 93 component/regression
cases pass; 244 legacy outputs remain identical. Synthetic TCP and CLI evidence
is not a native-game or production-admission claim.
