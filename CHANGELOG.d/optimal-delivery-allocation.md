# Complete-delivery cost optimization

- Add an opt-in joint assignment/order optimizer for summed routes after preceding
  target removal, not merely initial item distance or first feasible delivery.
- Use dependency-aware distinct-item matching bounds, bounded branch-and-bound,
  explicit cost gaps and strict proven-optimum export.
- Preserve original selectors and independently replay every complete result;
  existing joint/sequence/review implementations are unchanged.
- Sixteen actual Python tests pass, including 512 exhaustive obstacle domains,
  200 random numeric-cost oracles, 32 slots and 65,536 candidates. Three weakened
  implementations are rejected. No native/live/Rust/full-suite admission claim.
