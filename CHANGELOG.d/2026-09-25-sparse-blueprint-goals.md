### Added

- Read-only sparse multi-level terrain goal model: up to 32 disjoint cuboids,
  512 targets and one coherent 1,024-cell map/1.5 capture, exact floor/stair/ramp/
  wall/empty shapes, semantic mask identity and bounded remaining-cell evidence.
- Whole-mask game-tick stability, source invalidation, unknown preservation and
  twelve executed Python regression tests. No game-effect or admission changes.
