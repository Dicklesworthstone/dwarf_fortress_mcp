# Population watches — implementation and evidence status

## Added

- `entity_count` success/failure conditions in the shared watch engine, advertised by live
  spatial/1.8 schema discovery without adding a top-level MCP tool. Typed row predicates monitor
  changing unit, job, building, item, tile-feature and retained-announcement projections.
- Mandatory `observed_projection` scope and sound [known matches, known matches + unknowns]
  intervals. Comparisons are decisive only when every possible count agrees; missing/native-
  provenance/type/presence uncertainty remains unknown, including under negation and inequality.
- Snapshot/predicate identity, bounded generation/revision examples and explicit non-global-count
  evidence. Membership changes do not require rebuilding a list of entity handles. Counts do not
  mean stack quantities, native resource availability, global absence, causality or game success.
- One shared 1,000,000-unit entity/predicate allowance for a single evaluation or entire watch batch,
  with cooperative deadline checks and the existing shared 64-node/depth-8 definition bound.
- Integration with current single/batch watch evaluation and existing checkpoint/restart semantics.
  Rendering still precedes checkpoint sync and watch-root publication. Original evidence sealing is
  retained. Old definitions remain readable; old binaries reject the new condition operator.
- Usage, uncertainty rules, compatibility and qualification limits in `docs/POPULATION_WATCHES.md`.

## Evidence

Thirteen new Rust scenarios are registered: nine engine tests and four actual MCP-handler/private-
journal tests. They are **not compiled or executed in this environment**. No Rust toolchain was
found and direct network access to the toolchain host failed. No runtime, native DFHack, live-game,
complete repository qualification or production admission is established.

The independent Python checker passed 152 condition-schema cases (112 accepted, 40 rejected),
2,970 integer-interval cases and 59,022 population-completion cases. Only the new condition contract
and the base name/watch-literal definitions are composed; the full query envelope is not tested.
The mathematical reference does not execute Rust, replay, authority, work accounting or publication.
Executed script SHA-256: `cf5c802f4e4419622d711a33224305c35dcfb25a774c7e618557e10f37482edd`.
The committed script and extension Git blob IDs were verified against the executed bytes.

Native protocols, dependencies, journal framing, production runner maps, capability grants and game
mutation authority are unchanged. Archive-only and historical queries still do not evaluate watches.
