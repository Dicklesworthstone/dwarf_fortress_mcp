# Furniture construction progress over coherent observations

The adapter's `construction_progress::analyze` accepts up to 32 explicit native
building selections over the sealed operations, spatial or citizen/spatial state.
It binds the enclosing canonical anchor and source digest, retains that state's
entity generations, and joins construction/removal jobs and optional exact items.
No independent observation, replacement projection, native write or protocol is
introduced. Query authority, cancellation, entity scans and cooperative work/time
limits are checked. Results are deterministic and include bounded job examples
without limiting complete counts.

For Bed, Chair and Table, the declared observation-local condition requires a
positive bounded maximum stage, stage equality, no observed ConstructBuilding or
DestroyBuilding job, and, when requested, the exact matching-kind item held by
that building, flagged in_building, neither removed nor in_job/on_ground/in_inventory,
not container-held and not attached to any observed job. Other flag bits are
reported, not interpreted as usability. Native absence, unsupported kinds/stages,
identity mismatches, suspended construction and incomplete buildings without jobs
remain distinct. Job absence alone cannot establish completion.

This is a stage/link condition on a selected entity, not verification of an
external placement receipt or original footprint. It does not establish current
terrain, item reachability, room assignment, building usability, native causality,
continuous history, safety, native effect discharge or admission.

Ten Rust test groups cover the lifecycle, 4,608 exact-item flag/link cases,
1,120 stage/job cases, unknown/missing evidence, late removal jobs after the
example limit, authorization, budgets, canonical ordering and generation reuse.
They are source only: cargo, rustc and rustfmt are unavailable, so they were not
compiled or executed in this editing environment. No full-workspace, MCP, real
DFHack or live-fortress qualification is claimed. The upstream module-registration
file was verified against its exact Git blob before adding one module line.

Beads: df-dfhack-bridge-plane-c-pic.3, df-action-coordinator-exec-ero.4,
dfmcp-wp-pln04-long-horizon-bounded-obligations-qr1. Broad acceptance remains open.
