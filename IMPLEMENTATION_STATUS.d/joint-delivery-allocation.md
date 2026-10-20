# Joint furniture selection/order planner

Implemented opt-in pre-review planning over the existing captured route model.
An undeliverable cheapest assignment can now be replaced by a complete feasible
assignment/order under the unchanged original constraints. Existing deliverable
results remain unchanged. Node exhaustion is inconclusive, never a partial plan.

Executed: 15 actual Python tests with all 512 obstacle masks, 150 random domains,
independent exhaustive forward search, 32 targets, 65,536 candidates, budgets and
cancellation. Four imported runtime modules match original Git blob identities.
Native acquisition, real DFHack, Rust/MCP, full workspace and production admission
are not established. See docs/JOINT_DELIVERY_ALLOCATION.md. Broader beads remain open.
