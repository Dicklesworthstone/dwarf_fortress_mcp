# Named citizen arrival from coherent native positions

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

`dfmcp_adapter::shelter_analysis` evaluates every originally named citizen
against a fixed union of declared three-dimensional shelter regions. Its input
is the existing validated `LiveSpatialCitizenState`, not a caller-declared
roster of positions, a synthetic labor profile or a burrow-assignment receipt.
It makes no native calls and performs no game mutation.

`ShelterSelection` seals 1..128 distinct canonical spatial/1.8 entity IDs and
their original nonzero generations together with 1..16 inclusive cuboids.
Reordering input does not change the selection digest. Duplicate citizen IDs,
even with different generations, duplicate regions, reversed bounds, native
coordinate overflow and more than 262,144 summed region tiles refuse. Region
overlap counts each citizen once, in the first canonical containing region.

At one exact native observation the result distinguishes inside, outside,
not observed, changed incarnation, not alive and unavailable position. A
missing citizen remains unresolved rather than disappearing from the target.
Reusing a native unit ID after disappearance in the same state owner cannot
satisfy the old generation. A dead selected citizen makes the original
living-arrival condition false. Negative/out-of-map positions and
inactive/invisible citizens remain unknown. Babies, children, busy citizens
and citizens without work skills are not filtered out by workforce-eligibility
rules. Each capture is evaluated afresh: a prior arrival is not latched after
the citizen leaves.

The whole-selection truth is a three-valued conjunction: any established
failure is false, otherwise any unresolved target is unknown, otherwise true.
The nonempty selection prevents vacuous success. Every row and aggregate is
bound to the same exact anchor, full native source digest and bridge generation.
IDs and generations belong to that spatial session's observation owner, not a
portable identity token across restarts. This is not a durable original-goal
monitor; a newly opened owner cannot recover prior identity continuity from a
client-supplied selection. Selection hashes describe geometry and targets;
they neither authenticate the source nor grant read or mutation authority.

Query authority must cover the fortress; narrow grants are not widened.
The context anchor and entity allowance must cover the retained capture.
The explicit comparison-work limit and one cooperative wall deadline apply.
The immutable validated native owner lends its state without a canonical-world
copy or re-encoding unrelated items and skill histories. The analysis reuses
its published source digest after checking the root's profile/count binding
and each selected native citizen's ID, position, alive, active and visible
facts against the paired canonical projection, exact source and capture tick.
These consistency checks do not replace source admission by the acquisition
owner. `required_bytes()` scales the bounded goal/report workspace by selected
citizens and regions; it remains below the 32 KiB conservative maximum.

Declared shelter geometry is not a native burrow definition or proof of safety.
Inside at one capture does not prove evacuation causality, a route, stable or
continuous presence, future whereabouts, immunity from threats, or permission
to release restrictions. Production admission and native protocols are unchanged.
The MCP entry is documented in [MCP shelter status](MCP_SHELTER_STATUS.md).

Nine Rust integration tests are added using the production spatial/citizen
codecs and an independently assembled citizen component. They cover 3D bounds,
missing/dead/unknown targets, non-worker citizens, canonical region unions,
ABA, later departures, authority and budgets, invalid goals and 128 targets.
They have been source-reviewed but NOT compiled or executed in this environment;
Rust/Cargo, dependency caches and DFHack are unavailable. No Rust, native-game
or admission qualification is claimed.
