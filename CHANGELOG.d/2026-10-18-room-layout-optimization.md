## Bounded whole-project layout optimization

Room layout can explicitly compare complete candidates using
`"objective": "minimum-target-tiles/1"`. It minimizes planned target footprint,
then entry distance and deterministic placement ties, retaining one complete
best-so-far plan under the existing shared budgets. Finite-domain optimality
is distinct from a node-limited incumbent; legacy searches keep their bytes.
See `docs/ROOM_LAYOUT_OPTIMIZATION.md`.
