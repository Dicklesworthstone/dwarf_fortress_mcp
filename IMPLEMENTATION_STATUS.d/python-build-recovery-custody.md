# Original receipt recovery custody and source-history preservation

Standalone Python recovery now refuses read-only custody before native contact,
checks the last persisted receipt's generation before query/retention, and keeps
known indeterminate evidence visible when a native lookup is absent. A complete
positive frame is reserved before retaining uncertainty, within existing bounds.

The concurrent explicit reconciled-frame format, exact-original source proof,
immutable generic successor/normal receipt handling, route-batch routing and
publication checks remain unchanged. No dispatch permit or retry is introduced.

Executed evidence: Python 3.13.5 passes all 53 wire/store/receipt/real-TCP tests,
including 19 new recovery cases, and six weakened variants fail regressions.
Four new custody/history gaps reproduce on predecessor 5271476. Sources are
hash-checked before/after execution. Native replies are explicit golden test
peers, not a real DFHack process. Rust/MCP, real SDK, live fortress, full route
parent and full-workspace qualification are not established by this run.
See docs/BUILD_PLACEMENT_RECOVERY_CUSTODY.md.
