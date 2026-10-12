# Observed labor configuration and staffing analysis

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

`ObservedLaborRoster` connects the existing staffing allocator to source-qualified
current configuration. Callers supply named citizens, skills and modeled stress
preferences, not a claimed initial labor matrix. Skills/stress remain advisory;
no source identity or native conversion is inferred from those preferences.

`inspect` is a pure projection under a borrowed `PredicateEvidence`; its shell
must authorize reading the selected scope. `inspect_authorized` additionally
checks current Query authority, exact anchor and negotiated entity/byte budgets.
A qualified Unit identity, true living status and a known boolean for every
managed labor are required for each citizen. Missing, stale, omitted, asserted,
replayed, future-dated, wrong-typed and unqualified source fields cannot prove a
false labor setting. Any missing required observation withholds the entire
profile list and returns bounded per-citizen/field problems. No partial roster
can silently remove a worker from the request and change its allocation.

Eligible enabled unmanaged labors are retained for the existing allocator's
relief rule. Unobserved unmanaged labors remain outside the coverage claim; this
is not a complete native labor census or proof of complete stress relief.
Results retain the exact anchor, managed true/false counts and charged work and
representation allowance. Profiles are stable-ID ordered. The existing
`LaborAllocator` and `compile_execution` remain the only allocation/lowering
implementations; their capabilities, guards and final-goal semantics are unchanged.

Bounds: 64 selected citizens, 16 distinct demands, 64 requested slots, 256 skill
or positive labor tokens per profile, 512 source fields per selected entity,
256 KiB accounted representation and one million charged read operations.
Smaller explicit budgets fail without partial output. The caller must share its
remaining budget with subsequent allocation and compilation, not reset it.

Nine Rust regressions cover real planner handoff, absence versus false, source
qualification, liveness, partial rosters, deterministic order, retained positive
assignments, authority and bounds. They are source-reviewed but unexecuted: this
editing environment has no Rust toolchain or dependency cache. No Rust, native,
live, compatibility-admission or physical-job-completion claim is made.
