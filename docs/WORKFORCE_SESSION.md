# Session-owned workforce control

`dfmcp_adapter::workforce_session::WorkforceSession` integrates selected workforce
observations with the existing workforce/1.17 Rust journal. It is a typed adapter
API, not a new wire generation or production runner. The MCP entry is a separate
integration; neither this layer nor its stored evidence creates capabilities.

One session owns one journal, exact source binding, monotonic observed-tick floor
and optional native capture. Opening returns the already verified journal view;
it does not contact DFHack or choose citizens. Native connections are supplied
by an explicit per-operation factory and never retained in the session.

Observe invalidates the old capture before validation, checks custody before
connection, rejects reordered/duplicate citizen IDs and source-clock regression,
then retains the exact native capture. Prepare requires its witness, current
Plan and guarded ConfigureLabor authority. An exact duplicate key returns its
retained record without a native connection or preparation-lifetime renewal.
A new key cannot bypass unsettled work.

Commit requires the reviewed digest and explicit confirmation. The existing
journal re-observes the original paused capture and synchronizes DispatchStarted
before assignment. Once attempted, commit remains non-retryable. Wait makes at
most one receipt query; prepared, settled and permanent Unknown evidence return
locally. Local cancellation never creates a connection. Native cancellation only
retires eligible native preparation; it does not undo membership or repair Unknown.

Control, Recover and Offline modes remain fixed even with broader injected
grants. Offline never calls a source factory. Lost preparation can be recovered
by querying, not re-preparing. Missing native evidence remains unknown. Fresh
session and current grants are required after reopening. A native failure does
not erase healthy journal history. Current authority is checked against the
highest tick observed or retained, never a caller-supplied older tick.

View verification/copies, connection bootstrap and the final journal operation
share one narrowing wall-time and byte allowance. A connection is charged before
its factory is called. Complete response reservation remains the presentation
layer's responsibility. Filesystem latency is cooperative, not hard real-time.

## Evidence

Twelve Rust regression groups are registered against the actual session/journal
path with injected native and storage implementations. They cover lifecycle,
offline reopen, exact retries, failed refresh, scope/expiry/modes, lost replies,
no redispatch, permanent Unknown, local/native retirement, custody, failed intent
writes, budgets and clock regression. **They are uncompiled and unexecuted here:
Rust, Cargo and rustfmt are unavailable.**

`python3 scripts/check_workforce_session_reference.py` executes six independent
routing/selection/budget/lexical groups, including 147 mode/action/state rows
with both Unknown values (294 combinations), 32 refresh-failure combinations,
and view/bootstrap/work partition boundaries. These models do not execute Rust,
its allocator, native RPC, real file custody, MCP or a live game. No admission or
full qualification is established.
