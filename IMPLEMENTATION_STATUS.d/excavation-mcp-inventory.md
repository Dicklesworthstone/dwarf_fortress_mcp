# Excavation inventory increment: source present, not qualified

The standalone floor-goal monitor remains the sampling/persistence owner.
The Rust adapter now contains its typed goal evaluator, and the existing mining
recovery MCP server can optionally replay operator-pinned progress journals into
an independently scoped active-work inventory. This supersedes the earlier
statement that no MCP progress inventory integration exists; it does not claim
control-server integration, MCP sampling, automatic obligations or mining causality.

Ten adapter and nineteen MCP/archive/custody tests are registered but uncompiled
and unexecuted. Python fixture encoding/syntax checks and the unchanged Python
monitor replay/evaluator passed 18 positive and 12 negative examples, with 60 frame
reconstructions and four canonical strings. This is not execution of the new Rust
parser, file custody or MCP integration; no live-game/power-loss qualification is implied.
The normative increment contract and evidence limits are in
`docs/EXCAVATION_MCP_INVENTORY.md` and `architecture/excavation_mcp_inventory.json`.
