## Cost-aware complete room corridors

Opt-in cost-first-with-original-candidate/1 layout routing compares the existing
connector with a cost-first whole-project candidate under unchanged tile, part,
protected-cell and shared-work limits. RoomPlan retains its complete costs and
exact routed geometry. Observed layout derives costs from original raw map
bytes, never caller floor claims, and preserves single-read/strict replay rules.

The synthetic two-hall case reuses a longer open passage instead of five wall
targets while retaining every furnishing. Seventy-one Python tests and 384
legacy output comparisons pass; real game/native/Rust qualification is not
claimed. See docs/ROOM_COST_AWARE_CORRIDORS.md for limits and usage.
