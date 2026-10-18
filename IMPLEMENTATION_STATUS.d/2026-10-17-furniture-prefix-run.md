## Original furnishing prefix policy

Beads: df-dfhack-bridge-plane-c-pic.4/.5; broad execution remains open.

Implemented a sealed 1..8-step original furnishing prefix using the actual
existing FurniturePlan/progress model. The complete checked original inventory,
exact keys/items/targets/dependencies, and room/terrain metadata bind the seal.
Only one next original child can change at each transition; prior receipts and
all future selections remain immutable. No retry or completion authority exists.

Executed 20 Python tests with warnings denied, including 3,384 count/offset
selections in 1..32-step plans, eight-step continuation, all non-Placed states,
cancellation and altered history. Inputs are synthetic inventory presentations,
not authenticated native effects or filesystem custody. The unchanged dependency
furniture_plan.py was verified against Git blob
52fe43ce2d6514f3f0d1a3d983cc53c31fb39143. Both posted new Python blobs match
the tested local bytes. No full-repository/Rust/native/live qualification claimed.
