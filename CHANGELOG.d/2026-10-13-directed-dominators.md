## Directed bottleneck cognition (GA-DOM)

- Add bounded immediate-dominator analysis over explicitly selected observed
  directed relations, with reverse-edge queries, downstream failure-impact counts,
  unavoidable-objective chains, stable ordering, and replay witnesses.
- Reuse the canonical graph index; reject exhausted budgets and propagate
  caller-owned cancellation without returning a partial answer.
- Add 11 Rust regression/differential tests and a separately executable Python
  vertex-removal oracle (75,634 graph/root cases passed).
- Implementation/evidence details: `docs/GRAPH_DOMINANCE.md`. Rust tests are
  source-only in this environment; no native or release qualification is claimed.
- Owning bead: `df-franken-storage-mvcc-54h.4` (remaining integration/flow work stays open).
