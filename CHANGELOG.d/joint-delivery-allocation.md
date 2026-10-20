# Joint furniture assignment and delivery feasibility

- Add opt-in bounded joint item/order search when the cheapest fixed assignment
  cannot be delivered under original dependencies.
- Search the full candidate inventory without the unsound static cheapest-N
  reduction; backtrack constrained-item choices and replay complete solutions.
- Preserve successful legacy results and all original selectors. Search limits
  remain inconclusive with no partial plan; fallback does not claim cost optimality.
- Fifteen actual Python component tests pass, including exhaustive/random
  independent forward oracles, 32 slots and 65,536 candidates.
- No retained-intent substitution, native effect, protocol or admission change.
