# Whole-room constraints in route-backed placement

- Add an exact-parent, same-map room gate after route and current predecessor
  checks. Preserve every original floor/wall and all earlier blockers.
- Retain a complete RoomRouteHandoff in an explicitly new private batch format;
  old readers reject it, while ordinary route-review roots remain unchanged.
- Extend initialization with one mutually exclusive room-origin artifact pair;
  later calls cannot attach, replace, or downgrade the original parent.
- Require full room evidence before intent creation and before commit, preserving
  pending original-key recovery when the final gate refuses.
- Keep original child journals, native protocols, permissions and work/output
  bounds. Recovery stays build-only; no additional map read or retry is added.
- 41 scoped Python tests and five assertion-based mutation checks pass. Actual
  private files and original coordinator run; native I/O, acquisition/operations
  projection and current construction evidence are injected test boundaries.
- No native/live, Rust/MCP, full-workspace or production-admission claim.

See `docs/ROOM_ROUTE_BATCH.md`. Beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
