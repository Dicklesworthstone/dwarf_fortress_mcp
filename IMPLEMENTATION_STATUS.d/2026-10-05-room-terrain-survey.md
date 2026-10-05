# Remaining excavation for partially completed room recipes

Add a bounded, source-bound terrain reducer that keeps the complete original
room intention and proposes only its remaining visible wall targets. Observe
required bedroom walls and a full 3D remaining-target halo; refuse unknown,
wet, designated, incompatible or over-capacity work without returning a subset.
Preserve existing floor cells, sparse holes, material constraints and all levels.

Executed 16 Python tests with complete native-format byte fixtures, including
4,096 small masks, 250 bedroom cases, 64 dining cases and every guarded boundary
of a small survey. These are not native SDK/live-game or production admission.
See `docs/ROOM_TERRAIN_SURVEY.md`. Bridge beads `.3/.4/.5` remain open.
