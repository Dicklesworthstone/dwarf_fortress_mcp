# Bounded-run discovery and batch reconciliation

Add offline whole-directory recovery inventory with complete pending counts and
exact-byte-bound pagination. Add an expected-inventory-bound query-only foreground
pass with one connection, sixteen-query maximum, shrinking deadline, complete
output reservation and partial-progress preservation. Refuse duplicate native
identities, orphan/corrupt evidence and authority changes before dispatch.

Add fourteen executed tests; all fifty-two client/receipt/inventory tests pass.
Keep source-lost work unresolved and never retry an unpause. No native wire,
dependency, Rust/MCP or admission changes.
