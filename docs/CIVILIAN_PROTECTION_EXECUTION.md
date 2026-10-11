# Guarded named-civilian protection

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

`CivilianProtectionPlan::compile(burrow, civilians)` lowers an explicit protection
request into the existing semantic execution engine. `prepare` seals it using
independently issued `PredicateEvidence` and current `OperationContext` authority.
The caller names 1..64 unique, nonzero civilians and one distinct burrow. Names
and inferred threat text never determine identity or authority.

## Execution and cancellation

The fortress must already be observed paused. Pause a running fortress with the
ordinary pause plan first. Every step checks current paused state, the burrow's
kind and each named citizen's Unit kind and living status. The first step
activates `CIVILIAN_BURROW_RESTRICTION`; the second assigns the named citizens,
only after that first step is verified. Current effect authority, budgets,
preconditions and the existing deferred-step gate still apply at dispatch.
The standing order retains its existing Guarded risk and checkpoint policy.

Cancellation is **retain protection**, not automatic release. Each step seals
an explicit protective reassertion as its compensation, overriding the generic
inverse of a membership assignment. Existing burrow members are never removed,
unmentioned civilians are never reassigned, and cancellation of an undispatched
step cannot dispatch it. Original effect authority is still required for any
reassertion; a cancellation refusal remains a refusal. This neither confirms
threat clearance nor authorizes a later release. An explicit, separately reviewed
release plan remains the operator's responsibility.

The compiler does not claim missing membership bits mean false. Assigning true
is an explicit authorized request, including for an unknown prior bit. Since
cancellation cannot invert it, neither unknown nor already-true prior membership
can be turned into a fabricated false baseline. No-op *whole goals* are refused
by the ordinary planner rather than dispatched again.

## Proof and observation

The embedding `prepare` binds the entire goal to the current fortress: paused
state, exact named types and liveness, all named membership bits, and the active
restriction. The final action retains this goal too. `evaluate` is a read-only
current observation, reporting assigned, unassigned and unknown membership counts
plus the three-valued configuration truth at one exact anchor. Earlier success
cannot hide later unassignment, lost restriction, death or unavailable evidence.
An untrusted snapshot cannot establish the goal even when its values agree.

The generic action-intent consumer gets the same reference standing-order
postcondition and default whole-plan terminal. The final membership step retains
all named citizens' membership, type, liveness and pause predicates. Therefore
normal action proof and whole-plan proof both participate; a transport
acknowledgement is not sufficient.

This establishes **configuration**, not physical evacuation, native burrow safety,
complete population coverage, route availability, military mobilization or threat
clearance. It is not an autonomous release controller. It adds no native protocol,
MCP tool, compatibility registry entry or production runner.

## Bounds and validation posture

Collections are fixed at 64 civilians and two actions. The compiler validates
all predicate shapes and bounds conservative duplicated encoding accounting to
256 KiB. Embedding preparation and observation check current fortress-wide Observe
authority for pause/settings, exact anchor and entity/byte budgets. Preparation
also checks the action budget and delegates Plan authority to `StaticPlanner`.

Eight core tests and three actual `MemoryAdapter` tests are checked in. They
cover canonical order, limits, unknown prior membership, qualified source/liveness,
current versus historical truth, no-op refusal, authority/anchor/budgets, deferred
guards, real prepare/commit/poll/replay, and cancellation before dispatch with
pre-existing members. They have not been compiled or executed in this editing
environment: no Rust toolchain or repository dependency cache is available. No
Rust, full-workspace, native or live qualification is claimed.
