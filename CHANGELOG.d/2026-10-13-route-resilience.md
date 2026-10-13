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
