# Expose exact furniture receipt reconciliation through MCP

- Mark successful terminal Indeterminate-to-Placed query recovery with
  `receipt_reconciled`.
- Add an MCP regression proving recovery queries the original key without a
  second commit.
- Preserve the frozen eleven-tool surface and offline cancellation semantics.
- Rust tests remain unexecuted in this environment.
