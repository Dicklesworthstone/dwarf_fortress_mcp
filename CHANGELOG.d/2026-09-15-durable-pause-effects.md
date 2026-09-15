## Durable pause-effect coordination

- Require a private synced control-effect journal for the unadmitted control/1.7 development runtime; process-local-only live pause coordination now refuses startup.
- Persist `Prepared`, sync `CommitStarted` before exactly one native mutation dispatch, and sync terminal applied/not-applied evidence before acknowledging a commit result.
- Recover unresolved commit attempts across Rust-process restart as reconciliation-required states. Read-only reconciliation may reconnect, but mutating commits are never automatically retried.
- Keep generation loss or an unknown native effect indeterminate; the same durable effect is never reported safe to retry after a commit attempt has started.
- Bind native prepare tokens to bridge generation and replace implementation-defined hashes with domain-separated SHA-256 tokens/receipts. Add the missing CMake target and a reproducible C++17 mock-native control campaign.
- Give control/1.7 a session namespace distinct from spatial/1.6 and correct Agent Turn metadata so unadmitted development mutation is never labeled production mutation admissibility.
- Preserve the frozen eleven-tool MCP waist and continue to refuse every live mutation family except pause/resume. Production admission, registry, floor, and runner maps remain unchanged.
