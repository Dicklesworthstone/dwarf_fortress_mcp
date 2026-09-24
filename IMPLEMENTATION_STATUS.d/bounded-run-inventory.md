# Executable bounded-run inventory and query-only reconciliation

The run/1.13 Python recovery workflow now discovers existing intents and retained
terminal outcomes across a bounded private directory, reports complete pending
counts independent of pagination, and performs an exact-inventory-bound foreground
QueryRun pass. One connection, at most sixteen distinct queries, one shrinking
allowance and stop-on-first-failure preserve earlier synced receipts without any
mutation replay. Source-loss evidence remains unresolved operator work.

All 52 actual Python/POSIX/loopback tests pass on exact uploaded source bytes:
20 existing, 18 receipt and 14 new inventory/recovery scenarios. Four deliberately
weakened temporary implementations fail focused regressions. Details and limits
are in docs/BOUNDED_RUN_RECOVERY.md. This is executable developer functionality,
not Rust/MCP integration, real DFHack/live-game qualification, physical power-loss
proof, global controller fencing or production admission. Native wire, dependency
and production-map boundaries are unchanged.
