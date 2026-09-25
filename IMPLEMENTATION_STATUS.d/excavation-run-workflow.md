# Excavation-run host coordinator: executed development workflow

The transport-only limitation in excavation-run-client.md is now addressed by
`scripts/excavation_run_store.py` and `scripts/excavation_run_client.py` for this
POSIX developer workflow. They implement exact-confirmation start, append-only
intent/preparation/dispatch/terminal journals, complete-directory pending-work
fencing, offline inventory/inspection and bounded query-only recovery. Existing
journal state never grants permission to resume a commit. Source loss remains
operator work, not a successful stop or a reason to delete a journal.

All 48 codec/RPC/custody/lifecycle tests execute successfully, including independent
reference bytes, actual private files, process locking, hard-kill/restart recovery
and joined fragmented TCP doubles. Four weakened source copies are rejected by
focused tests. See `docs/EXCAVATION_RUN_WORKFLOW.md` for commands and boundaries.

This is host-local durable coordination, not a production capability or clock
lease. No real SDK/plugin, generated protobuf, C++, Rust/MCP, live fortress,
physical power-loss or complete repository qualification was executed. Native,
manifest dependencies, production runners and compatibility registry are unchanged.
Beads df-dfhack-bridge-plane-c-pic.4/.5 and df-action-coordinator-exec-ero.4 remain open.
