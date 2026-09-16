# Coherent production diagnostics and declared supply plans

## Added

- `production_diagnosis` and `inventory_plan` in the actual spatial/1.8 live and
  archive-only query dispatcher. `mode="production"` is a convenience query, not
  a new top-level MCP tool. Exact-record historical queries support both analyses.
- A sealed adapter view for operations/1.3, operations/1.4, spatial/1.6 and
  spatial/1.8 states. Analysis borrows the enclosing canonical snapshot, source
  digest and entity generations rather than rebuilding an operations-only world.
  Spatial/1.6 gains the reusable adapter API, not another advertised MCP surface.
- Job-assignment inspection and anchor-bound job/item/container relationship
  drill-downs. Archived drill-downs stay pinned to their exact record and digest.
- Complete Agent Turn/active-watch reservations, current Query and observation
  custody checks, explicit cooperative analysis deadlines, and whole-row pages.
  Current watches are carried through without sampling or checkpoint changes.
- Schema composition preserving existing spatial, workforce, history, batch and
  population-watch definitions. Archive discovery has twelve stateless variants
  and three history variants. The spatial/1.8 history schema uses its actual
  `sch1` cursor prefix without changing spatial/1.6's shared schema source.

## Semantics retained

Production flags and attachment observations do not prove causal blockers or
readiness. Inventory allocation is only for declared interchangeable stack units
under the existing conservative policy and is not full native requirement matching,
reachability, a reservation, labor assignment or executable plan. The shared
integral allocator and diagnosis ordering are retained, not reimplemented.

No dependency, native protocol, mutation authority, compatibility registry,
production runner or admission state changed. The formal unadmitted development
phase in IMPLEMENTATION_STATUS.md remains unchanged. Usage and the implementation
status of this increment are in `docs/COHERENT_PRODUCTION_ANALYSIS.md`.

## Evidence

Ten new Rust functions are registered: four adapter tests and six handler/schema
scenarios. Existing archive regressions are extended, not replaced. They cover
coherent identities, ID reuse, inherited container restrictions, competing supply,
watch preservation, historical drill-downs, pagination, authority and custody.
None has been compiled or executed here: rustc, Cargo and rustfmt are unavailable.
No Rust, native, live-game or full-repository qualification is established.

`scripts/test_production_inspection_wrappers.py` passed 256 independent compact-JSON
size cases. The largest historical-wrapper growth was -22 bytes. This checks the
inspection-envelope sizing assumption only, not Rust serialization, typed analysis,
archive replay, real filesystem custody, MCP transport or live-game behavior.
Executed script SHA-256:
`07d1113f6d6865fa0d78fd7f85922a4ebf81a7f16c02fcf916d7de15c796e196`.
