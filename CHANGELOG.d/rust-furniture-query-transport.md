## Connect original receipt reconciliation through Rust TCP

- Accept native uncertainty clearing only with a complete original-generation
  positive QueryPlacement receipt, not a flag, cancellation or unrelated reply.
- Permit explicit seen Indeterminate-to-Placed query recovery without changing
  ordinary native successor immutability or creating a dispatch permit.
- Preserve prior validated summaries and records when later validation rejects.
- Add six Rust regression functions with real-codec and joined TCP peer coverage.
  Tests are source-present but uncompiled/unexecuted here; no Rust/native/live
  qualification or production admission is claimed.
