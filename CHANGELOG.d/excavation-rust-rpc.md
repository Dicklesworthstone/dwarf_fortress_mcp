## Concrete excavation-run Rust source

Implement the fixed 1.18 TCP source directly behind the durable coordinator:
control bootstrap, terrain-independent recovery, same-connection one-use commit,
canonical bounded envelopes, source/receipt checks, shrinking budgets, per-I/O
operator checks and caller-signalled cancellation. No Python subprocess or new
native/MCP/dependency surface. Add 16 Rust tests and six independent request
fixtures. Only the Python fixture construction ran; Rust remains uncompiled.
