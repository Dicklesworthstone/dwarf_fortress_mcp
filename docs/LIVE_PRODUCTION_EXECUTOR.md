# Bounded labor and work-order execution

Owner: `df-dfhack-bridge-plane-c-pic.4`. Recovery cases also belong to
`df-dfhack-bridge-plane-c-pic.5`.

`dfmcp_adapter::live_production` executes selected steps of one complete original
semantic plan through the existing workforce/1.17 and work-orders/1.10 owners.
It is a Rust embedding boundary. The host supplies the real family sessions,
their private native and semantic association stores, a canonical citizen
evidence owner with independently issued source authority, and the workforce
connection factory. Existing protocol clients and native coordinators remain
responsible for the actual effects.

## The supported plan is explicit

The complete sealed plan is checked before either family can be contacted. The
closed subset has one to sixteen steps and a canonical archive of at most
64 KiB. Archive traversal is bounded to 4,096 nodes and depth 16; the selected
family boundary additionally bounds the complete parent's predicate complexity.
Every step must be either:

- `SetLabor` for one to 32 selected canonical citizens and one exact labor; or
- a finite `CreateWorkOrder` from the existing wooden bed, door, table or chair
  recipe catalog.

Work orders must be leaves of the dependency graph. Their insertion cannot
release a downstream step that needs finished goods. Conditions, compensation,
mandatory checkpoints, unsupported actions and predicates outside the respective
family's protected precondition subset are refused before contact.

For a labor step with dependents, every original postcondition must be an equality
between a selected citizen's `labor.<requested labor>` field and the requested
boolean, or a nonempty conjunction of those equalities. Such a predecessor may
not carry a temporal obligation. The strict native single-labor readback proves
these exact historical postconditions, including preservation of every other
labor column. A broader native assignment result cannot release a successor.

Leaf postconditions, the complete plan terminal predicate and leaf obligations
remain in the original plan. Executing this subset does not discharge them.

## Selection preserves the parent

The private selected-step value retains the whole `PreparedPlan` and its actual
`StepId`. It does not construct a one-step replacement, change the plan digest,
rebase its anchor, renew its deadline or derive a replacement idempotency key.
Each native association retains the complete parent digest, original selected
step, original key, canonical source and exact native plan/journal identity.

Existing public single-step family entry points continue to require one step.
Selected multi-step reviews cannot enter the legacy whole-goal monitors, whose
one-step assumptions would otherwise conflate the selected effect with the
parent's complete goal.

The executor checks the exact original plan, selected step and complete typed
route on every trait call. A caller cannot bypass dependency checks by invoking
`LiveFamilyExecutor` directly or supplying an invented Applied frontier. The
combined owner retains the family sessions privately and verifies predecessor
evidence in those sessions.

## What releases a dependency

A dependency requires the exact original predecessor's
`SingleLaborResult::Verified` receipt and its retained native/semantic custody.
The owner checks this at advancement boundaries. Work-order evidence refresh
also verifies the predecessor after the native dispatch marker has synchronized,
so loss of predecessor custody during a callback prevents the order setter.

This is proof of a historical selected labor effect. It does not assert current
labor at a later observation or supply arbitrary downstream preconditions.
Each family's own independently authorized canonical/native evidence still
guards its current preparation and commit.

The existing native owners require unlimited grants for the named whole
fortress. The combined owner intersects its opening and current grants, carries
the lower risk ceiling and earlier expiry into every nested call, and requires
the native families' Guarded risk level. Entity/map-only or finite-use grants
cannot be widened into that native authority. After a strict queue decode, even
a later source or Query refusal retains the consumed tick. The executor and
durable recovery pin preserve that floor, so an older context cannot revive it.
The native floor belongs to the owner's original restore epoch. Within an
established owner, a later epoch can inspect historical custody under current
Query authority, but cannot use that owner for fresh queue observation or
production control. Old-incarnation ticks do not expire that later epoch's
historical Query authority.

`NativeOrderCreation::Inserted` proves the exact finite order was inserted into
the queue. It does not prove material availability, a workshop, execution,
produced items, physical quiescence or the complete original goal. The combined
owner never reports original-goal completion from all native receipts being
present.

## Recovery and integration boundary

Use this executor with the durable coordinator described in
[`LIVE_MUTATION_DISPATCH.md`](LIVE_MUTATION_DISPATCH.md). That coordinator retains
the complete original archive and execution frontier before family contact.
After a lost reply or restart, recovery consults the original native keys under
Query authority. It neither constructs a replacement operation nor restores a
process-local review or native dispatch permit.

Canonical evidence refresh reacquires the original paused citizen observation;
it cannot allocate a replacement cursor or silently rebase the plan. Canonical
citizen and native family generations remain separately bound. Their numeric
generation counters need not match.

This boundary adds no MCP tool or combined MCP server, bridge method, protocol
generation, source issuer, capability grant, production runner or admitted live
tuple. Concrete native clients remain unadmitted development implementations.
Synthetic component execution is distinct from full workspace, ambient runtime,
native DFHack and live-fortress qualification.
