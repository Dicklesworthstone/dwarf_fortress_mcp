# Observed independent routes and minimum separators

- Add bounded GA-FLOW route packing and minimum cuts for the existing observed
  floor/stair model, with explicit edge-disjoint and internally tile-disjoint modes.
- Independently check path validity/disjointness and equal-cardinality separating
  cuts; distinguish excluded endpoints and adjacent-endpoint mixed separators.
- Add residual rerouting, cooperative cancellation, exact work limits and fifteen
  actual-source Rust regressions, including exhaustive obstacle-map cut oracles.
- Python design oracle passed 42,856 graph/map/model cases plus adversarial cases.
  Rust compilation/tests/formatting/Clippy and native qualification remain unrun.
- No native protocol, game mutation, compatibility or production admission change.

See `docs/MAP_ROUTE_RESILIENCE.md`; bead `df-franken-storage-mvcc-54h.4` stays open.

## Shared spatial query

- Wire `map_route_resilience` into the existing spatial query dispatcher and
  discovered schema. Default to a compact minimum-cut view; page long route
  witnesses by individual position with complete-analysis/source commitments.
- Preserve current query authority, source fences, full anchors, foreground
  deadlines and Agent Turn/watch output reservations. Excluded endpoints keep
  null counts; adjacent-endpoint cuts identify their direct-edge exception.
- Add 13 Rust query regressions using strict spatial and citizen-inclusive
  capture fixtures. Bound caller-supplied certificate paths before comparisons.
- Python JSON Schema checks pass 15 accepted and 32 rejected request shapes;
  the graph oracle still passes all 42,856 graph/map/model cases. All 28 Rust
  regression tests remain unexecuted; no native or release evidence is implied.
