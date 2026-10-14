# Joint delivery feasibility source and component evidence

The new route_furniture_joint API searches alternative exact items and dependency
orders when the initial nearest-items assignment cannot be delivered. It is
bounded to 4096 search nodes and the existing shared work/input/output limits.
Only a complete verified model solution exports a new plan. Exhaustion remains
incomplete; exhaustive blockage is limited to supplied inventory/model.

53 Python tests pass, including 13 new joint-solver cases and five rejected
semantic mutants. Existing tests include explicit inventory/packet doubles.
No native transport, private batch, Rust/MCP or live-game execution is claimed.
This increment is an API; CLI review opt-in is a separate integration increment.
See docs/JOINT_FURNITURE_DELIVERY.md. Beads .3/.4/.5 of df-dfhack-bridge-plane-c-pic
and df-franken-storage-mvcc-54h.4 remain broader open integration/evidence work.
