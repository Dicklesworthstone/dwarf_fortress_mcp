# Demand-driven workforce allocation

Implementation increment for `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm`
(WP-PLN-03), 2026-10-11.

`LaborAllocator::allocate_demands` accepts a caller-declared roster and explicit
staffing targets. It proposes existing `SetLabor` actions; it does not dispatch
an effect, observe a citizen, or grant native mutation authority. No MCP tool,
wire protocol, dependency, production runner, or compatibility entry changes.

## Semantics

Each demand names its labor token, corresponding skill token, desired worker
count, minimum skill, and priority. Smaller priorities precede larger ones;
equal priorities use ascending labor tokens. Repeated labor targets are rejected.
Zero workers explicitly retires a managed role. A zero skill threshold permits
unskilled generalists in this declared model, not a claim of known native skill
absence. The supplied 0..1000 stress scale is a model, not a native conversion.

A citizen fills at most one demanded role. Iterative augmenting paths maximize
staffing coverage lexicographically in demand order without taking away an
already filled higher-priority seat. This avoids assigning the only qualified
miner to brewing when another citizen could brew. Search prefers the shortest
reassignment path, with candidates ordered by skill descending, existing role,
stress ascending, and stable entity identity. This is not a global skill-score
or minimum-churn optimum.

Citizens at stress 800 or above receive no new demanded role and relinquish
managed assignments. The original heavy-labor relief rule also disables their
MINING, MASONRY, WEAPONSMITHING, ARMORING, and CARPENTRY assignments. Other
unmanaged labors remain untouched. Output contains every role's eligible count,
selected identities, and missing workers, plus the relieved roster and work
count. Partial staffing is explicit, not an exception or an assertion of success.

Changed assignments are grouped by labor and enabled state, with all disables
before enables and identities sorted. Already-correct assignments produce no
redundant action. Input order cannot change the result. The legacy
`optimize_roster` API retains its heuristic behavior, with bounded validation and
canonical citizen ordering; it does not invent staffing targets.

## Bounds and execution boundary

Default limits are 512 citizens, 64 roles, 512 requested positions, 64 grouped
actions, and 1,000,000 charged work units. Profiles contain at most 256 skill and
256 labor tokens, each at most 256 UTF-8 bytes without NUL or blank content.
Independent caller limits can be smaller; fixed ceilings cannot be exceeded.
Validation, candidate inspection, matching traversal and output scans consume
work. Sorting is additionally bounded by the fixed collection ceilings.
Budget exhaustion returns no partial allocation or executable actions.

The result is an immutable proposal, not a sealed plan. Callers must independently
establish canonical source coverage, citizen eligibility, skill mappings and
stress policy before relying on those facts. The ordinary intent preparation,
capability checks, scoped witnesses, leases where applicable, commit-time
revalidation, and effect/obligation proof still apply. The order of a proposal's
actions alone is not an atomic native workforce transaction. Any caller requiring
full staffing must check `is_fully_staffed()` before using `into_actions()`.

## Evidence for this increment

Twelve Rust module tests are present: two existing heuristic behaviors and ten
allocation regressions, including a 512-graph exhaustive coverage oracle, scarce
specialty reassignment, priorities, generalists, stress relief, explicit skill
mapping, role retirement, deterministic grouping, shortages and budget refusals.
They were source-reviewed but not executed in this editing environment: Rust,
Cargo, rustfmt, repository dependencies and DFHack were unavailable.

An independent Python implementation of the matching algorithm was executed
against exhaustive assignment enumeration for 14,040 bounded model cases,
including 3x3 eligibility graphs, targets from zero through two and candidate
ordering permutations. It found no coverage or assignment-invariant mismatch.
This is algorithm-model evidence, not execution of the Rust source and not
workspace, native, live or admission qualification.
