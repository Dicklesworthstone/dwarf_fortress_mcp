# Complete-delivery cost optimization

- Add an opt-in joint assignment/order optimizer for summed routes after preceding
  target removal, not merely initial item distance or first feasible delivery.
- Use dependency-aware distinct-item matching bounds, bounded branch-and-bound,
  explicit cost gaps and strict proven-optimum export.
- Wire cost optimization into allocation and NEW review creation via explicit
  flags. Preserve artifact-export work, source closure and final authority checks.
- Keep basic, sequencing and first-feasible modes unchanged; retained-review
  checking rejects optimization and never substitutes original selections.
- Thirty-nine actual Python tests pass: 16 core, 12 allocation/runtime and 11
  review cases. The core includes 512 obstacle and 200 random exhaustive cost
  oracles, 32 slots and 65,536 items. Three weakened variants are rejected.
- Five legacy fixture modes produce byte-identical predecessor packets. Runtime
  tests inject acquisition/roster/transport-budget boundaries; no native TCP,
  live DFHack, Rust/MCP or full-workspace qualification is claimed.
