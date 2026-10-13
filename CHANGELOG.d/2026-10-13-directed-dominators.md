## Directed bottleneck cognition (GA-DOM)

- Add bounded immediate-dominator analysis over explicitly selected observed
  directed relations, with reverse-edge queries, downstream failure-impact counts,
  unavoidable-objective chains, stable ordering, and replay witnesses.
- Expose generation-fenced `dominators` through the existing structured query
  dispatcher and schema, with full-tree or compact objective-chain views.
- Reuse the canonical graph index; reject exhausted budgets and propagate
  caller-owned cancellation without returning a partial answer.
- Add 21 Rust core/dispatcher regression and differential tests, a separately
  executable Python vertex-removal oracle (75,634 graph/root cases passed), and
  schema validation (six valid and sixteen invalid cases passed).
- Implementation/evidence details: `docs/GRAPH_DOMINANCE.md`. Rust tests are
  source-only in this environment; no native or release qualification is claimed.
- Owning bead: `df-franken-storage-mvcc-54h.4` (remaining integration/flow work stays open).
