# Rust session receipt reconciliation

`BuildSession::recover` now reaches the explicit exact-source positive-receipt
reconciliation path when the retained original placement is terminal
`Indeterminate`. Previously the session returned that entry before opening a
query source, even though the journal already supported safe
`Indeterminate -> Placed` proof append.

Query recovery requires the original source generation exactly, uses current
Query authority, never observes/prepares/commits, and retires the connection
after the attempt. Cancellation deliberately keeps terminal attempted
uncertainty offline; it cannot reinterpret an attempted effect as
nonapplication.

Two session-level Rust regressions drive the real journal/session state machine
from observe through prepare and an indeterminate commit, then verify that
query-only recovery appends the exact Placed proof without another prepare or
commit, while cancellation opens no source. These Rust tests are source-present
but were not compiled or executed in this environment.

Beads: df-dfhack-bridge-plane-c-pic.4/.5. No native wire, capability, dependency,
production map, compatibility registry, or admission state changes.
