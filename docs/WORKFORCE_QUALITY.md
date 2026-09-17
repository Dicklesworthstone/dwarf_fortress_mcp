# Quality-aware workforce allocation

The new standard-library-only `dfmcp_world::workforce_allocation` module supplies
bounded integer minimum-cost maximum flow for one-worker-per-citizen allocation.
This backend is source present, not a qualified runtime or a native labor allocator.
The existing `workforce_plan` query remains unchanged by this backend increment.

## Model and objective

Already-filtered candidates bind a worker ID and demand index to effective skill,
nominal skill and modeled travel steps. Demands carry 1..128 slots and a priority
in 0..1000; higher values prefer that role when full staffing is impossible.
First maximize filled slots, then lexicographically maximize summed priority,
effective skill and nominal skill, then minimize summed steps. This is a global
allocation: residual rerouting can move a flexible worker to leave another role
for its only specialist. One person cannot fill two simultaneous slots.

The implementation uses checked four-component integer costs rather than floating
point or large scalar weights. Equal shortest-path labels retain the first parent
in canonical demand-key/worker-ID/arc order. No random iteration or hidden seed is
used. Inputs must be canonical and duplicate-free; malformed models fail.

A forward eligibility capacity is the total request, a proven finite upper bound
on possible flow. Worker-to-sink capacity is one. This preserves the Hall shortage
interpretation while permitting vector costs for each individual worker/role edge.

## Verification and limits

Every result is independently rechecked by rebuilding residual capacities from its
assignments. A residual-closed source cut must equal assigned headcount, and every
positive residual edge must have nonnegative lexicographic reduced cost under the
provided node potentials. Together these certify cardinality and quality only for
the declared model. A deficient-demand witness counts distinct eligible workers;
it does not establish a fortress-wide labor shortage or additive independent gaps.

Limits: 4096 distinct candidate workers, 16 demands, 128 total slots, 65536 candidate
edges and 10000000 logical work units. Successive shortest paths take at most 128
augmentations; storage is O(V+E), search is O(F E log V). Logical work counts cover
model visits, residual scans, node initialization/updates, augmentations and the
independent verification pass; ordered-container comparison costs are not individual
work units. Exhaustion or arithmetic/certificate disagreement returns no allocation.
The core has no I/O, scheduler or native/game authority. No hard wall-time guarantee
is claimed.

## Evidence

Eight Rust tests are registered, including 4096 exhaustive small-graph/priority
comparisons, rerouting, exact objective precedence, empty and multi-slot models,
Hall deficiencies, deterministic budget boundaries and certificate mutation tests.
They have not been compiled or run here: Rust/Cargo/rustfmt are unavailable.

`python scripts/test_workforce_quality_reference.py` passed 4608 exhaustive-oracle
comparisons, 4096 deterministic reruns and three certificate refusals. It executes
an independent Python mathematical reference, not the Rust implementation, MCP,
DFHack or a live fortress. No Rust/native/live qualification or admission is implied.
No dependency, bridge generation, production runner or mutation capability changed.
