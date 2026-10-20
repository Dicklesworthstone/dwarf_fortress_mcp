## Stop an original construction-clock effect without reopening its batch

Add explicit cancellation with exact scoped key/plan/token and native incarnation,
using the existing CancelRun wire method. Retain original private clock custody
through one cancel attempt and receipt synchronization; never start, prepare,
commit, automatically poll or retry. Stopping remains unresolved; immutable
terminal history remains offline. Query transport stays read-only.

Combined suite: 34 tests pass (11 new cancellation scenarios); ten weakened
implementations fail assertions. Actual files/TCP/CLI execute against test peers,
not a live fortress, plugin build, Rust/MCP or full-workspace qualification.
