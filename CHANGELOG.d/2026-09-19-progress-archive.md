# Progress archive and publication barrier

Added a separate progress/1.12 archive with complete native capture/source
retention, exact-record retrieval, bounded metadata discovery and same-segment
historical comparison. Reopening starts a new comparison segment; uncertain
writes fence publication and torn histories are refused without repair. Offline
custody is non-promotable and requires no mutation authority. The session's new
publication callback completes before replacing current evidence.

Thirteen Rust scenarios are registered but uncompiled/unexecuted. The independent
Python reference passes 120 positive cases and rejects 604 byte corruptions,
602 torn prefixes and 11 rehashed illegal histories. This does not establish Rust,
filesystem, MCP, live-game or admission qualification. See
`docs/WORK_ORDER_PROGRESS_HISTORY.md` for exact limits and evidence scope.

The same progress MCP server now supports operator-configured append-before-
publication history and offline Query-only recovery without endpoint/token reads
or a native source. Query history lists complete metadata, fetches exact records
and compares same-segment endpoints. Historical navigation preserves live current
state, native failure does not disable healthy history, and custody/output failures
cannot publish a bootstrap session. Eleven additional Rust groups remain unrun;
the independent Python model passes 134/32 accepted/rejected request cases and
7,263 pagination cases, with modeled maximal output below its reservation.
