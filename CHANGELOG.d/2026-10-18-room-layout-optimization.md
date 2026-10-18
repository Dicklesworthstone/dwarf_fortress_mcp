## Bounded whole-project layout optimization

Room layout can explicitly compare complete candidates using
`"objective": "minimum-target-tiles/1"`. It minimizes planned target footprint,
then entry distance and deterministic placement ties, retaining one complete
best-so-far plan under the existing shared budgets. Finite-domain optimality
is distinct from a node-limited incumbent; legacy searches keep their bytes.
See `docs/ROOM_LAYOUT_OPTIMIZATION.md`.

The existing layout CLI adds `--require-optimal-layout`, refusing artifacts
unless finite-domain optimality survives complete replay. Reports distinguish
selected/first-feasible target counts and node-limited incumbents. All six
exports preserve exact complete downstream artifacts; no default behavior or
legacy report bytes change. Fifty-nine component tests pass, including 13 real
CLI tests and 27 unchanged corridor/compiler regressions.
