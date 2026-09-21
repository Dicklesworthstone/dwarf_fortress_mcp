### Retain verified mining outcomes across client restarts

The existing dig/1.16 developer client now persists exact native terminal proof
in immutable, intent-bound sidecars. Start/query/cancel acknowledge known outcomes
only after receipt file and parent-directory synchronization and custody checks.
Inspect/query/cancel use retained historical proof without credentials or native
calls. Lost replies can be reconciled by query without reissuing CommitDesignation.
Unknown/missing/prepared evidence never becomes a terminal receipt; conflicts,
partial writes and corruption fail closed without repair or overwrite.

37 actual Python/loopback/POSIX/CLI regression groups pass. Native wire, Rust/MCP,
production admission and mining-completion semantics are unchanged. Real DFHack,
power-loss durability and full repository qualification remain unverified.
See `docs/DIG_TERMINAL_RECOVERY.md` for the updated recovery contract and commands.
