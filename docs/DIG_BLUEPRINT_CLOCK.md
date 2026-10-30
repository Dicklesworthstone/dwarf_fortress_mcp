# Reviewed finite clock intervals for a whole mining blueprint

`dfmcp_adapter::dig_blueprint::clock` connects an existing whole-blueprint
excavation monitor to the unchanged run/1.13 coordinator. It permits one
explicitly reviewed finite interval at a time. It does not start a background
loop, renew the excavation deadline, or treat a clock receipt as excavation
completion.

## Original goal and current readiness

The clock owner retains the complete immutable `MonitorSpec`: original sparse
geometry, native journal identity, every original terminal Designated receipt,
independent map source, and fixed temporal policy. Run keys commit to that whole
specification and the clock journal identity. Replacing a receipt, narrowing the
mask, changing the deadline, or borrowing another monitor's run is refused.

Before native preparation and again before native commit, the caller's injected
guard rechecks original custody and acquires one complete fresh map capture.
Every selected cell must be visible, dry, free of magma and building occupancy,
and either a wall with the original Mine designation or a floor with its
designation cleared. Unselected gaps are excluded. The capture tick must equal
the selected paused run observation. This readiness does not establish worker
access, structural safety, mining causality or goal satisfaction.

Each requested interval is bounded by all of:

- the native limit of `1..1200` game ticks and `1..60000` wall milliseconds;
- the opening and current request's explicit game-tick allowances;
- the original monitor's maximum gap and absolute deadline;
- at most `min(64, max_samples)` retained run intents, with no eviction.

Callback limits are stop triggers, not exact-tick or hard real-time guarantees.
Actual observed ticks, including overshoot, raise the authority floor. Later
checks cannot hide an overshoot by returning to an older paused observation.

## Review, dispatch and recovery

Preparation durably appends the original intent before the external guard and
native PrepareRun. Commit consumes the process-local review, checks the selected
paused witness and durably appends DispatchStarted before the external guard
and native CommitRun. After each guard, the coordinator verifies its own exact
journal bytes and current authority again before contacting the native method.
Changing either original custody or the clock journal during the fresh-map
check cannot pass through to an unpause.

Only the live owner that prepared an interval can submit its distinct review
seal. Restart, reconciliation, failed publication and explicit abandonment do
not restore that permit. Once DispatchStarted exists, neither a failed guard,
lost reply nor uncertain storage synchronization makes unpause retryable.
Reconciliation queries the original key and digest once. Missing native evidence
remains unresolved; it never proves that dispatch did not happen.

The private run journal uses its existing format and file-custody checks.
External recovery requires its exact `{journal_id, head}`. The acknowledged head
must be present in the retained complete chain: a valid descendant can recover
a lost response, while an unrelated journal or rollback is rejected unchanged.
An existing empty journal also requires this pin. Only an object initialized
in the current process may enter the owner without a recovery reference.

Cancellation before dispatch retires the local intent. Cancellation after
dispatch can request the original native safety pause; only that safety action
may repeat. Cancellation requires current original and native clock authority,
but does not require a new future-tick allowance, Plan/Observe grants or an
unexpired excavation deadline. Native SourceLost remains unresolved without a
pause claim. Historical pause verification never proves the current pause state.

## Authority and independent sources

The canonical fortress requires scoped Query/Observe and Plan plus fortress-wide
ControlClock for new advancement. The run/1.13 source independently requires
NIL-source Query, Plan and ControlClock. A map-scoped or named-fortress grant is
never rewritten into native clock authority. Opening grants and current request
grants must both authorize the operation; injected replacement grants cannot
renew expired retained authority. Safety cancellation requires current Query
and ControlClock under both identities, without Plan or Observe.

The run endpoint and software pair must match the monitor's configured map
source. Its generation is independently pinned and need not equal either the
map generation or native dig generation. These selectors do not establish
shared process identity, a named native fortress identity, a global controller
fence or cross-plugin exclusion. The original run/1.13 deployment limitations
remain in effect; see [bounded native runs](BOUNDED_SIMULATION_RUN.md) and the
[durable coordinator](BOUNDED_SIMULATION_COORDINATOR.md).

The clock owner prepays complete bounded journal and native work, and narrows
one wall deadline through every operation. Consumed native and map ticks are
retained even when later validation or publication fails. Raw clock-read floors
that were never journaled remain process-local; restart reconstructs the floors
that durable records actually establish.

## Evidence boundary

The implementation reuses the existing typed native protocol and durable
coordinator. Component tests exercise actual Rust source with typed protocol
fixtures and injected storage/source failures. They do not establish a real
DFHack build, live fortress behavior, compatibility admission or a production
clock lease. The compatibility registry and production runner map are unchanged.
See `IMPLEMENTATION_STATUS.md` for the exact executed evidence at each increment.
