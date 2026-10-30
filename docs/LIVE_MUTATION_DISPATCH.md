# Cross-family live mutation dispatch

The live adapter has several narrow, independently versioned mutation families:
control/1.7, dig/1.16, build/1.19, work-orders/1.10 and workforce/1.17. Each family
already owns its native plan, preparation journal, confirmation, effect-specific
revalidation and recovery. `dfmcp_adapter::live_dispatch` coordinates those
owners for one sealed semantic plan without bypassing any of them.

## Contract

`LiveDispatchCoordinator::new` accepts only a structurally valid, fully routable
`PreparedPlan`. Unsupported semantic actions fail before a family executor can
be contacted. The coordinator retains the original step IDs, deterministic
idempotency keys, dependencies and routed family.

At most one step may be Prepared, DispatchStarted or Indeterminate. This is
deliberately stricter than running independent plan branches concurrently:
without one durable transaction spanning multiple native family journals, a
second family must not race an unknown first effect.

`prepare_next` chooses the first dependency-satisfied step in canonical plan
order and reauthorizes its exact semantic capability, risk, entities and map
scope. Family preparation is considered non-effectful; a preparation error leaves
the step Pending.

`commit_prepared` reauthorizes again and records DispatchStarted *before*
calling the family. A valid family receipt moves the step to Applied. Any error,
zero receipt, or wrong-family receipt moves it to Indeterminate. Calling commit
again cannot redispatch it.

`reconcile` queries the exact original family operation. Applied and Failed are
terminal. Indeterminate remains blocked. `NotDispatched` is intentionally
stronger than "record absent": the family executor may return it only when its
durable state proves that the dispatch marker was never synchronized. Only that
result makes the same semantic step Pending again.

`cancel_prepared` is available only before DispatchStarted. It retires a
preparation and never claims to compensate or undo an effect that may have begun.

## Durable handoff identity

`frontier_digest()` is domain-separated and covers:

- the sealed semantic plan digest;
- every step ID and live family;
- the original deterministic semantic idempotency key;
- dependency IDs;
- Pending/Prepared/DispatchStarted/Applied/Failed/Cancelled/Indeterminate state;
- opaque preparation or family-receipt digests when present.

It is a handoff identity, not mutation authority and not proof that a native
effect occurred. A durable host can bind it beside the existing family journals,
then reconstruct actual effect truth from those journals during recovery.

## Executor boundary

A host implements `LiveFamilyExecutor`. The executor receives the original
`PreparedPlan`, exact `PlanStep`, routed family request and current
`OperationContext`. It must use the existing narrow family coordinator and the
same semantic idempotency key. It may not invent a second key, widen capability
scope, skip family confirmation, or translate an uncertain response into success.

This core intentionally does not create a new wire protocol, MCP server, global
controller fence, live evidence issuer or production admission. Concrete MCP
wiring and durable reconstruction from the existing family journals remain the
next part of `df-dfhack-bridge-plane-c-pic.4`.
