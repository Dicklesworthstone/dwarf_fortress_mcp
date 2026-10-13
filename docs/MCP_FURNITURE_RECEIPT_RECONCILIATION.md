# MCP furniture receipt reconciliation

The unadmitted furniture/1.19 MCP recovery path now reports when a query resolves a
retained terminal Indeterminate receipt through the exact original positive-proof
path. The existing frozen tool waist is unchanged.

`fortress.wait` / recovery still calls the same BuildSession owner. When the
selected original record is terminal Indeterminate, query recovery may open the
exact original source, obtain one byte-valid Placed receipt, append it under the
journal's explicit reconciliation transition, and return
`receipt_reconciled: true`. No prepare or commit is retried.

Cancellation remains outside this exception. It does not contact native recovery
for a terminal attempted effect and cannot turn uncertainty into nonapplication.

A focused MCP regression drives prepare, one indeterminate commit, then a native
query that returns the original Placed proof. It asserts one total commit, two
total queries (fresh-key preflight plus reconciliation), and two source factories
(initial owner and recovery owner). Rust tests are source-present but unexecuted
in this environment.

Beads: df-dfhack-bridge-plane-c-pic.4/.5. No protocol, top-level tool, production
runner, registry, capability or admission change.
