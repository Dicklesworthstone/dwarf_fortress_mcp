# Exact furniture plans

`scripts/furniture_plan.py` validates the closed `dfmcp.furniture-plan/1` format.
A plan contains 1..32 steps, each with `name`, `kind` (bed/chair/table), exact native
`item`, exact `[x,y,z]` `target`, and optional `after` step names. Dependencies
express placement ordering, not completed construction or native job scheduling.

The compiler preserves every selected item and coordinate. Duplicate items,
duplicate target tiles, unresolved dependencies, cycles, self dependencies,
unsupported furniture and out-of-range context are refused. Deterministic Kahn
ordering chooses the lexically first ready name. Plan identity includes the full
normalized request and dependencies; input ordering does not affect it.

No native read, item substitution, wall clearing, placement authority, atomic
transaction, checkpoint or completion proof is created by compiling a plan.
The prefix reducer requires callers to verify actual receipts: only Placed
unlocks the next step, while uncertainty, cancellation or refusal blocks it.

Seven executed Python test functions cover all 4,096 directed four-node graphs
against a permutation oracle, normalization, exact resources, map bounds,
closed inputs and all progress-prefix outcomes. This increment is a pure model,
not native or Rust/MCP execution. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
