## Joint furniture selection and delivery feasibility

- Add bounded alternative-item/order search for NEW proposals after the initial
  exact assignment is delivery-blocked. Preserve every original constraint,
  backtrack material-critical choices, and verify a complete forward sequence.
- Distinguish incomplete search from model-proven blockage; neither exports a
  partial plan. Reuse existing codecs/route policy and one cumulative work budget.
- 53 actual Python tests pass (13 new); five semantic mutants are rejected.
  Synthetic core inputs and explicit workflow doubles only, not native/Rust/live
  qualification. No existing review, native effect or permission is changed.
