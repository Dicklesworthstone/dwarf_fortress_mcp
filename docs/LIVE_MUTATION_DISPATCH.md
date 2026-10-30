# Cross-family live mutation dispatch

The live adapter has several narrow, independently versioned mutation families:
control/1.7, dig/1.16, build/1.19, work-orders/1.10 and workforce/1.17. Each family
already owns its native plan, preparation journal, confirmation, effect-specific
revalidation and recovery. `dfmcp_adapter::live_dispatch` coordinates those
owners for one sealed semantic plan without bypassing any of them.

## In-memory sequencing contract

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

## Durable execution ownership

`live_dispatch::durable::Coordinator` adds the persistent execution frontier.
Use it when the host must retain preparation, dispatch and recovery history
across process loss. `DFMLDJ01` stores the complete bounded original PlanArchive,
its semantic and object digests, the journal incarnation, and a hash-chained
sequence of checked state transitions. It never reconstructs a new one-step
plan, changes an original key or renews an original deadline.

The durable subset contains at most 16 routable steps. The original archive is
bounded to 256 KiB, 4,096 nodes and depth 32; the complete store is bounded to
4 MiB and 4,096 frames. Concrete executors may impose a smaller closed subset.
All limits are checked before family contact. Every replay decodes the complete
archive, recompiles routes and dependencies, and applies each transition again.
A checksum alone cannot authorize an impossible frontier.

The additional durable states distinguish preparation and cancellation calls
whose replies may have been lost:

| State | Meaning |
|---|---|
| Pending | No accepted preparation attempt owns this step. |
| PrepareStarted | Preparation intent synchronized; the family outcome needs reconciliation. |
| Prepared | Exact family preparation retained; only the current process may also hold a review. |
| DispatchStarted | One commit intent synchronized; another commit is forbidden. |
| CancelStarted | Cancellation intent synchronized; cancellation outcome needs reconciliation. |
| Indeterminate | Original family outcome remains unresolved. |
| Applied, Failed, Cancelled | Accepted historical family outcome, with its original receipt when available. |

At most one nonterminal active step exists. Dependencies release only after an
Applied family result. The concrete executor must establish what that result
actually proves and recheck its own native predecessor custody at effect
boundaries. `all_applied()` never proves the original goal;
`original_goal_proven()` remains false.

## Write ordering and review lifetime

`prepare_next` synchronizes PrepareStarted **before** calling family prepare.
Unlike the in-memory convenience coordinator, a failed or lost preparation reply
does not simply make the step Pending again. The durable owner retains the
original uncertainty. Only a strong family proof of absence before any dispatch
can release an attempt that has no retained preparation or dispatch marker.

A successful preparation is synchronized before a process-local review is
published. The review binds the journal, complete original archive, exact step,
family preparation, current head and acting session. `commit` requires that
review's exact seal plus explicit confirmation. It consumes the review and
synchronizes DispatchStarted before contacting the family. The accepted result
must then synchronize before it can be published.

Restart, inspection and Query recovery restore no review. Even a recovered
Prepared result cannot create a native commit permit. It permits inspection or
the family's authorized pre-dispatch cancellation path. Cancellation retains its
own write-ahead intent and cannot undo an applied or possibly dispatched effect.

Every operation has one shrinking wall-time allowance and prepaid storage/family
byte work. The family receives only the time remaining after the last journal
boundary. Mutation checks use both the opening and current scoped grants, the
current session and fortress, the monotone authority clock, the original plan
expiry and the selected obligation deadline. Expiry is inclusive. Query-only
historical reconciliation remains available after the original plan expires.

## Recovery pin and private custody

Compute `initial_recovery` before exclusive creation and retain its result
outside the journal. The recovery pin contains the nonzero original journal
identity, an acknowledged head and the highest consumed authority tick. Opening
requires that exact journal and a fully replayable descendant of the
acknowledged head. A zero pin, missing ancestor, altered original archive,
impossible transition, torn tail or corrupt frame is refused without repair or
truncation.

After every operation, including a failed acquisition, retain the latest
`recovery()` value before handing the owner to another process. A later consumed
tick can narrow authority even if the failed call could not append a new frame.
That unsynchronized floor travels in the external pin; it is not presented as
a durable observation. Reopening takes the maximum of the replayed floor, the
pin's floor and the caller's floor. The next authorized append retains that
floor in the journal.

The private-file constructor reuses the existing descriptor-pinned,
no-follow, exact-mode directory/file lock shell. An uncertain write,
synchronization or result publication fences the owner. Recovery may synchronize
and verify a complete surviving descendant; it never repairs damaged bytes or
blindly repeats native work. Offline mode inspects history without contacting a
family or changing the journal.

This core creates no new native wire protocol, MCP server, source authority or
production admission. A host supplies a `DurableFamilyExecutor` whose recovery
uses the exact existing family journals and native keys. Concrete MCP wiring and
live qualification remain separate work under
`df-dfhack-bridge-plane-c-pic.4`.
