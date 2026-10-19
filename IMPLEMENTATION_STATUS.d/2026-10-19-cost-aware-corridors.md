## Cost-aware corridors through complete observed room planning

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. Broad workflow remains open.

A bounded cost-first corridor candidate is compared against the existing
connector. Only complete tile/part-feasible candidates can win. Original floors,
protected cells, room entries, access points and all furnishings remain fixed.
One original 100,000-check allowance spans baseline, alternate routing, exact
partition verification and publication. Work/cancellation failures never return
an earlier candidate. Binary costs alone are assumptions, not terrain authority.

RoomPlan retains the complete original corridor-cost domain and cells before
sealing, rehashes its tight final mask and regenerates all derived outputs on
import. Automatic layout selects this policy only through the explicit
cost-first-with-original-candidate/1 strategy paired with a cost objective.
The observed wrapper derives all costs from original raw map bytes, not caller
site permissions, and independently recounts the final target union. Current
source/deadline checks and one-observation client lifetimes remain unchanged.
Saved artifacts retain the strategy; old defaults are not silently migrated.

The synthetic two-hall fixture changes five observed wall targets into zero by
using a longer existing passage: 29 versus 39 complete floor targets. Both
original halls and all four furnishings are retained and actually allocated.
A separate small graph replaces three charged cells with five free cells; a
three-cell hard cap keeps the original route. These are synthetic planning and
observation results, not native mining or worker-path measurements.

Executed 71 warnings-denied Python tests: 18 core routing, 17 whole-room/layout/
raw-map workflow, nine TCP/CLI and 27 unchanged existing compiler/solver cases.
The core oracle enumerates 2,187 two-terminal cost/barrier worlds. Tests cover
all eight areas/32 furnishings, original wall rings and cross-level selectors,
exact work limits, every guarded checkpoint in a small routing case, protected
cells, forged-and-rehashed outputs and all-or-nothing shortage. The unchanged
cases retain the prior 512-tile complete room regression and solver oracles.

Nineteen joined synthetic map peers exercise ordinary full modules, the actual
MapClient, fragmented TCP frames and subprocess commands. All seven live and
offline exports pass. Connection closure before search, lost replies, source/
nonce/profile refusal, replaced authorization, depleted budgets and final-output
failures publish no stale result or additional read. Every strict export refuses
a node-limited complete incumbent without claiming global network optimality.
No alternate runtime solver, client, storage backend or import adapter is used;
fault hooks wrap actual calls or alter synthetic source/configuration only.

An independent-interpreter comparison against exact base bfb0d351 preserves all
384 recorded legacy output hashes: 32 room plans, 96 layout results, 128 raw-map
survey artifacts and 128 reports. Source snapshots were verified against their
Git blob identities before modification, including connector 5ed92e085209fafc0f349c71cdb08edbb66b36a9,
solver f3f42182206893e94396fa84749bc4f81a53994d, native reader
95a8f485afa76baced77edac0702ddb891b945cb, offline CLI
e235bdf13e99d3228668344709937fa46b3ef008 and surveyed CLI
ccc417611c75d6d422e2be7e7f0ad63750f35cec. The baseline source manifest is retained
with the local validation evidence. Syntax and patch whitespace checks pass.

The two-candidate corridor comparison is not an exhaustive network optimizer;
baseline failure still refuses, and an overlong free detour may be discarded.
Layout optimum scope remains only the declared candidate/compiler domain.
Real DFHack, Rust/native builds, room-backed mutation execution, full-repository
and live/production qualification were not executed. Effect journals, native
methods, placement authority, original-key recovery and admission remain unchanged.


### Publication revalidation (2026-10-19)

The previously pending implementation was reapplied to hash-verified base
bfb0d351 and reviewed before direct-main publication. All 71 tests passed again
on the publication source: 62 component/regression cases plus CLI partitions
of four and five cases. The 384-result independent legacy comparison also
passed again. An aggregate invocation interrupted by the execution time limit
is retained separately and is not counted as a completed test run. Every
uploaded runtime/test blob was compared with the exact locally tested bytes.
