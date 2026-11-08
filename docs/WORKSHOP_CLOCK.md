# Bounded game time for one original workshop

Work item: `dfmcp-reality-wiring-proof-4bf.14.6`, under the original first-fortress
production prerequisite. This component reuses Workshop/1.20 and Run/1.13. It
introduces no native methods, wire generations, dependencies or admission.

## Durable original clock owner

`dfmcp_adapter::workshop_construction::clock` associates separately reviewed
finite runs with one exact, acknowledged original Carpenter placement. Its
immutable target contains the Workshop journal identity, source binding, full
canonical `Placed` receipt and an explicit absolute clock deadline. Workshop
placement itself has no persisted game-time goal deadline; this clock horizon
is separately selected by the operator and never described as inherited goal
authority.

Every retained Run key binds that target and its Run journal identity. Changing
the original workshop, placement receipt, source, horizon or journal cannot
adopt old run history. The same loopback endpoint and DF/DFHack software pair
must match, while Workshop and Run plugin generations are independently pinned.
Run/1.13 has no fortress folder or site identity. Matching these pins and a
paused tick is not proof of the same owned game process or an atomic snapshot.

Named-fortress authority and native NIL clock authority are supplied separately.
Both the current and opening grants are checked, and request budgets can only
narrow the original allowance. Each run is at most 1,200 game ticks and 60,000
native wall milliseconds, and its whole requested horizon must fit the frozen
absolute deadline. Callback stop limits are not exact-tick or hard real-time
guarantees. Observed overshoot remains visible.

Preparation reobserves the exact paused clock, synchronizes the original Run
intent, and invokes the supervising fresh-workshop guard before native
preparation. Commit consumes its process-local review, reobserves the exact
clock, synchronizes `DispatchStarted`, then invokes the guard again before the
single native unpause attempt. A lost reply, failed guard, storage failure,
changed source, rejected confirmation or wrong seal cannot recreate that review.

The guard must return a complete fresh Workshop progress observation for the
original building and material. Only the exact original `InProgress` verdict
at the selected clock tick can support another interval. A complete, absent,
contradicted or unknown original refuses advancement. Readiness exposes stage,
maximum stage and ConstructBuilding job count. It does not prove worker
availability, job suspension state, scheduler viability or construction causality.

The existing append-only Run journal retains the complete native records.
External recovery requires the independently preserved journal identity and
acknowledged head; complete descendants are accepted, rollback and unrelated
histories are refused. Recovery grants Query/reconciliation only, never another
unpause. Offline inspection has no native calls. Unresolved and terminal-unknown
runs block new intervals. Cancellation in the original authorized control session
can retire an unattempted interval or request a bounded native safety stop; it
never repeats unpause or deconstructs the workshop.

The adapter bounds history to 64 intervals. An integrating host must also respect
the original Workshop journal's separate 64-sample capacity. It should durably
record and acknowledge progress at both external guard boundaries so consumed
source continuity survives process recovery. Goal evidence must be sampled
independently after a settled run: a Run receipt is not workshop completion.

## Evidence scope

The component has independent Rust tests for exact target binding, lifecycle,
guard ordering after durable markers, confirmation consumption, lost replies,
external recovery lineage, deadline and budget limits, authority revocation,
safety stop, corruption, consumed source floors, and complete bounded history.
Execution results are recorded in `IMPLEMENTATION_STATUS.md`; these tests use
the actual adapter owner with injected Run sources and journal storage. They do
not execute the modern MCP transport, real DFHack SDK or a game process.

The Workshop MCP integration and exact source/native/game qualification remain
separate evidence gates. This source does not establish production admission or
close the original untouched-fortress workshop-and-furniture-output criteria.
