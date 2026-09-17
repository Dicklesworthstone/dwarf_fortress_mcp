# Recoverable control sessions and bounded one-shot pause execution

## Implemented source

- Complete the previously unregistered control-session release implementation by
  wiring it into the actual control/1.7 MCP server and adding its missing tests.
  `fortress.cancel(scope="session")` releases the connection, exclusive journal
  lock and single-session capacity without changing any retained effect state.
- Consume the session value, not only its registry entry. Already-resolved shared
  handles cannot keep resources alive or enter a new operation after close.
  The session mutex drains the current foreground call; journal/connection drop
  precedes capacity reuse. Keep 32 completed close acknowledgements for retries.
- Teardown remains possible for fenced/corrupt journals, exhausted request IDs,
  revoked grants and an individually poisoned session mutex, without disclosing
  cached game or effect evidence. Global registry failures still refuse.
- Opening renders and bounds the complete response before registering ownership,
  advertises its exact close request, and releases unpublished resources on error.
  Reopening uses fresh identity, normal custody and existing authority checks.
- Preserve the distinction between session closure and exact-key cancellation.
  Prepared records remain prepared, cancelled keys remain retired and unresolved
  attempts require reconciliation. Closing never writes, repairs or deletes the
  journal, dispatches native cancellation, compensates an effect or grants retry.
- Route `fortress.commit` through a reusable one-shot coordinator with a separate
  injectable mutation-source boundary. Exact identity, current authority and core
  budget validation precede work. The actual MCP path reserves the complete
  possible outcome packet before durable intent or a mutation-source call.
- Recheck remaining time after CommitStarted sync and pass only that remainder
  to the fixed RPC client. No reconnect or automatic mutation retry is possible
  through this execution path. Failed intent durability prevents source dispatch;
  failed/contradictory replies and failed terminal durability remain indeterminate.
- Reuse existing receipt verification and journal transitions. Valid evidence
  already returned by the source is persisted under current authority even when
  its RPC consumed the deadline. Synchronous storage is still cooperative, not
  hard-preemptible. Complete uncertain frames may recover; partial ones refuse.
- Retained native outcomes replay without connection use, but do not bypass valid
  core budgets, current authority or custody. In particular, zero budget dimensions
  retain the core InvalidRequest behavior rather than weakening replay checks.

Usage and precise limitations: `docs/CONTROL_EXECUTION_RECOVERY.md`. This fragment
records the implementation/evidence status of this increment; the formal
unadmitted phase in IMPLEMENTATION_STATUS.md remains unchanged. Native protocols,
wire methods, journal framing/tags, dependencies, production runners, compatibility
registry, game-effect families and admission authority are unchanged.

## Validation

Twenty-three new Rust test functions are registered: eight resource-lifecycle,
nine adapter one-shot/fault cases, five actual-handler/loopback integration cases,
and one full-response reservation test. Existing cancellation and recovery suites
remain present. Tests cover unchanged journal bytes, locking, close/reopen, stale
handles, races, replay, core authority/budgets, intent/terminal sync faults, partial
writes, lost replies, deterministic deadline crossings and no duplicate dispatch.
The loopback fixture uses the actual framed TCP/protobuf client but is not DFHack.

**No Rust compilation or test execution occurred in this editing environment.**
Rust, Cargo and rustfmt are unavailable; local network access to obtain a toolchain
or checkout failed. Validation is source/diff review and GitHub commit/branch
verification only. No independent Python mirror is offered as a substitute for
execution of these Rust paths. No native, live-game, filesystem power-loss,
full-repository qualification or production admission is established.
