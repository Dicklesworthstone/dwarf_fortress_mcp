## Exact-footprint terrain monitoring

- Add `terrain_count` to shared foreground watch and condition inspection execution.
  Track up to 16,384 exact coordinates across 64 disjoint cuboids, not just returned
  entities; reuse bounded predicates and sound count-interval comparisons.
- Require coherent native provenance, visibility, position and identity. Missing,
  hidden, unallocated and uncaptured tiles remain unknown, including under negation.
- Preserve watch stability, deadline, epoch, publication and persistence machinery.
  Add bounded evidence and source-only lifecycle/provenance regression tests.
- Independent Python mask/count reference: 12,482 cases passed. Rust compilation,
  Rust tests, native/live execution and qualification were unavailable/not performed.
  No game effects, native protocol change or admission expansion.
