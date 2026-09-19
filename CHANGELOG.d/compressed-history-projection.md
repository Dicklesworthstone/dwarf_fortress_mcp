### Compressed multi-record history projection

- Repair the historical-series reader's reference to a decoder that became
  test-only during the delta-storage increment. In tests, that old helper also
  had no predecessor payload and therefore could not decode a delta record.
- Use the same production compressed/raw decoder and expanded acquisition bound
  as single-record replay. Carry the verified base through every prefix record,
  including records not selected for projection. Keep page-local bases separate
  from the current journal state and future append base.
- Register eight mixed-journal regression tests for sparse selection, separate
  pages/reopen, periodic keyframes, epoch reset, corruption in skipped deltas,
  expanded limits, callback refusal, final custody loss and preflight validation.
  Existing raw projection tests are retained.
- Source inspection only in this environment: no Rust/Cargo/rustfmt is installed.
  These tests have not been compiled or run. This is not Rust or live qualification.
