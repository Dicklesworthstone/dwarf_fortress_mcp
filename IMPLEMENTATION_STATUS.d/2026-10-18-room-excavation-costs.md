## Whole-room excavation-cost comparison

Bead: dfmcp-wp-pln01-spatial-blueprint-layout-pwd. Broad work remains open.

The existing layout compiler now supports a request-bound binary cost map and
cost-first objective. Every complete target, including connectors/access, is
charged; original constraints and the selected corridor compiler are unchanged.
Independent lower bounds prune only non-improving branches. Node exhaustion may
retain a whole incumbent without proof; work/cancellation returns no plan.
Offline costs are assumptions, never map evidence or native permission.

Executed 15 warnings-denied Python tests using actual full compiler/allocator
imports. Independent enumeration covers 128 cost maps and 18 two-area cases;
additional cases cover complete 32-furnishing/eight-area selection, exclusions,
original access charges, all checkpoint cancellations, exact work/node limits,
replay/tampering and selector retention. A separate comparison against fetched
8855037c preserves 108 legacy layout outcomes byte-for-byte.

All ten original compiler/dependency modules were reconstructed and verified by
Git blob identity before changes. No AST extraction, import adapter or alternate
compiler was used. Map acquisition, CLI transport, native DFHack, Rust, live game
and full-repository qualification have not been run for this increment. No native
method, journal, source-admission or mutation-authority boundary is widened.
