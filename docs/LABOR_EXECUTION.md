# Guarded staffing configuration

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

`LaborAllocator::compile_execution` lowers a declared roster and staffing demands
into an immutable `LaborExecutionPlan`. The existing allocator still decides the
assignment. This layer connects that decision to the ordinary semantic planner,
per-step effects and original-goal proof instead of calling allocation success
an executed assignment.

## Exact configuration, not hypothetical completion

All staffing targets must be feasible. Shortages refuse effect generation; use
`allocate_demands` to inspect partial coverage. Only changed labor bits become
actions. A no-change model remains inspectable but cannot be dispatched as an
invented operation with an inverse compensation.

The first step requires independently eligible evidence of every named citizen's
Unit kind, living status, and complete initial managed-labor matrix. Each later
step additionally requires the opposite current value for the bits it changes.
Absent, unknown, omitted, untrusted or future-dated fields do not establish a
false labor bit. An external change cannot silently turn a step into a no-op
whose compensation would enable a previously disabled labor.

Steps form a dependency chain, with disables before enables. Dependency proof is
part of the sealed plan, not an assumption that vector order controls dispatch.
The terminal predicate describes all requested managed labor bits over the named
roster, including unchanged selected and unselected workers. The last step also
retains that complete predicate in its postconditions for generic action-intent
consumers. Changed-bit readback alone cannot prove the configuration. Death or
a conflicting unchanged assignment prevents the final proof.

`LaborExecutionPlan::prepare` uses the existing `StaticPlanner` and an independently
issued `PredicateEvidence`. It checks current Observe authority for the named
roster and retains normal Plan, source, anchor, risk, shape and deadline checks.
The resulting plan must still use the ordinary adapter prepare/commit/poll path
with current ConfigureLabor authority. Stable action identities, compensation,
idempotency and later proof remain owned by that path.

## Model and native boundaries

Skills, their labor mappings and the 0..1000 stress scale are caller-declared
allocation preferences. This layer does not certify them as observations or as
native Dwarf Fortress skill/stress conversions. They never satisfy a mutation
precondition. The observed guards protect concrete requested configuration,
identity kind and living status, not claims about the quality of the allocation.

Completion means the requested labor configuration at the proof anchor. It does
not prove job assignment, productive service, future staffing, safety of a native
burrow or a native atomic multi-detail transaction. Other unmentioned labors and
citizens remain outside the target. Scope authority and native resolution are not
widened. In particular the existing single-step native semantic workforce handoff
still refuses dependencies, compensation and unsupported preconditions; this
compiler does not bypass it or add a multi-step native dispatcher.

## Bounds and evidence

Compilation accepts at most 64 named citizens, 16 roles and 128-byte non-control
labor names, additionally constrained by the caller's allocation limits. Both
allocation and predicate construction consume the supplied work allowance.
Canonical predicate/action accounting is bounded to 2 MiB. Sealing also enforces
the caller's entity, action and encoded-byte budgets. Failure exposes no partial
executable list. No new dependency, bridge protocol or production admission exists.

Seven core integration tests use the real planner and predicate evidence engine.
They cover ordering, full-goal proof, determinism, unknown/forged/future/changed
initial state, changed deferred inputs, no-ops, shortages, liveness, authority and
budgets. Two laboratory tests use actual MemoryAdapter preparation, commit,
polling and idempotent replay, plus mutation-authority denial. All nine are
source-reviewed but unexecuted in this environment: Rust, Cargo and DFHack are
unavailable. No Rust, native, live or admission qualification is claimed.
