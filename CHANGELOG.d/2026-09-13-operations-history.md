# Durable operations observation history

- Added an optional synced observation journal to operations/1.3 session startup
  and refresh. Canonical sources are recorded before live root publication and
  replayed against exact stored anchors after restart.
- Added private Unix file custody and exclusive writer locking, bounded hash-chain
  frames with independently checksummed lengths, conservative crash-tail refusal,
  explicit operator-only tail repair and uncertain-write fencing.
- Added operations-only `history` and `historical_query` through `fortress.query`.
  Historical queries use current authority, return past anchors/source evidence,
  preserve current active work without advancing it, and perform no bridge read.
- Added fifteen registered Rust scenarios spanning recovery/storage and actual
  MCP handlers, including 8192-byte paging and source-failure behavior.
- Independent Python framing checks passed 5,833 cases. Rust compilation, Rust
  tests, filesystem crash qualification, stdio and live execution were unavailable
  and are not claimed. The journal is not a full MVCC backend, effect ledger,
  durable watcher, checkpoint, complete history or anti-rollback mechanism.
- Native wire/producer, dependencies, production admission and migration bead
  status are unchanged. Usage and limits: `docs/OPERATIONS_HISTORY.md`.
