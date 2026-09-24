# Bounded-run terminal recovery: implemented and Python-tested

The existing run/1.13 Python CLI now persists exact verified terminal native
records beside immutable intent and can inspect them without a bridge or game.
Lost responses remain unresolved until query evidence is retained; source loss
never becomes a verified stop or retry permission. Directory/file custody,
immutable replay and complete response reservation precede acknowledgement.

All 38 actual Python/POSIX/loopback tests pass (20 existing, 18 new), including
611 byte corruptions and 611 incomplete prefixes, partial writes, independent
file/directory sync faults, restart, cancellation and cross-process locking.
See docs/BOUNDED_RUN_OUTCOMES.md. This supersedes the prior intent-only CLI
limitation, not the separate Rust coordinator. Native protocols, dependencies,
MCP and admission are unchanged. Real DFHack, Rust, physical power-loss and full
repository qualification were not run and are not established by these tests.
