### Added

- Read-only sparse multi-level terrain goals: up to 32 disjoint cuboids,
  512 targets and one coherent 1,024-cell map/1.5 capture, exact floor/stair/ramp/
  wall/empty shapes, semantic mask identity and bounded remaining-cell evidence.
- Executable `track_excavation.py start-blueprint` with durable foreground sample,
  offline inspect/cancel, retained input-independent recovery, fixed deadlines,
  whole-mask sampled stability and separate profile-bound journal checksums/limits.
- Sixteen new actual journal/TCP/subprocess/fault tests plus twelve model tests;
  all 63 focused tests, including unchanged legacy regressions, pass and are
  registered in `verify.sh`. No game-effect, native protocol or admission changes.
