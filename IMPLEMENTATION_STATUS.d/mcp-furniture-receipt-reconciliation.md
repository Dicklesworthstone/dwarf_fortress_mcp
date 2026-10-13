# MCP furniture receipt reconciliation

Source present: the furniture MCP recovery handler now exposes the Rust
BuildSession exact positive-receipt reconciliation path and marks successful
resolution explicitly. A focused regression proves no second commit. Rust/MCP
tests are source-present but unexecuted; no production admission is implied.
