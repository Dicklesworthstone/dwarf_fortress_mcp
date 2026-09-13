# Observed production diagnosis and declared inventory allocation

- Add bounded integral inventory allocation with residual rerouting, deterministic
  stack assignments, flow/min-cut equality and joint shortage witnesses. The same
  observed supply cannot be counted independently by overlapping demands.
- Add ancestry-aware conservative supply exclusions and exact caller-declared raw
  type/subtype/material selectors. No native recipe, access, reservation, or effect
  plan is inferred from the declared stack-unit model.
- Join jobs, holders, attachments and containers into bounded production diagnosis.
  Deduplicate item roles and expose observed flags, stage values, shared references
  and unindexed filters without asserting causal blockers or readiness.
- Integrate production_diagnosis, inventory_plan and mode=production through the
  actual operations/1.3 fortress.query handler. Preserve active watches, complete
  response budgets, session/snapshot-bound pages and the sixteen existing variants.
- Register 22 Rust tests. Rust execution, rustfmt, Clippy, stdio, full repository
  gates and live-game behavior remain unverified in this environment.
- An independent Python design oracle passed 1,568 exhaustive and 1,000 seeded
  allocation models; 90 schema checks and three documentation examples passed.
  These checks are not execution of the Rust or production qualification.
- Preserve native bridge bytes, dependency pins, read-only authority, eleven tool
  names, current migration bead ownership and the empty production registry.
