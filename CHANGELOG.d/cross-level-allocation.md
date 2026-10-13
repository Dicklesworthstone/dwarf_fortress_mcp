## Furniture allocation across observed floors

- Add explicit per-slot max_vertical_distance (0..127), defaulting to unchanged
  same-floor endpoint semantics and canonical bytes. Non-route allocation refuses
  positive opt-in instead of silently discarding the requested route model.
- Reuse bounded observed floor/stair routes and exact whole-plan assignment with
  a three-dimensional geometric lower bound plus the independent route cap.
- Thirteen actual Python allocator tests pass, including 81 stair-shape pairs,
  180 unpruned assignment oracles, maximum endpoint/slot cases and old result bytes.
- Retained route-review/frontier integration is a separate increment. No native
  transport, Rust/MCP, private batch execution or live-game qualification claimed.
