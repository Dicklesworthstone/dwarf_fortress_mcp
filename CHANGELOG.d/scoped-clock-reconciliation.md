## Query-only reconciliation of discovered original clock operations

- Add a bounded pass over at most 16 original unknown clocks using independent
  inventory identity, exact frp-/frf- records and run-only query credentials.
- Retain each verified outcome before the next query; stop on failure without
  restarting effects, and keep SourceLost/absent/prepared/running unresolved.
- Preserve complete prior-file custody and fence ambiguous publication.
- All 46 Python tests pass (19 new); actual clock TCP/private files execute with
  explicit original route-batch/budget ports. No native/live admission claim.
