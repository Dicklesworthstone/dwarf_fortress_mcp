## Room template quarter-turns

Room provisioning accepts explicit 0/90/180/270-degree area rotation. Complete
excavation masks, bedroom corridors and doors, unit bounds, entry points and
furniture targets rotate together before existing whole-project checks and
allocation. Unrotated artifacts remain byte-identical; nonzero rotation is
sealed into the retained intent. See `docs/ROOM_ORIENTATION.md`.

Sixteen focused Python tests passed, including actual allocation of a rotated
15-slot bedroom cluster, mixed-orientation corridor connection, near-edge
geometry, legacy digests, cancellation and tamper rejection. Native/Rust/full
repository qualification was not run; no live admission claim changes.

Automatic room layout now searches caller-permitted orientations together with
origins in one bounded whole-project search. Fixed origins, site masks and
exclusions are not relaxed. The CLI can solve, replay and export rotated plans
through its existing artifact paths. Eighteen new orientation-search tests and
sixteen existing layout tests pass, in addition to the sixteen compiler tests
above (50 total). The old negative arbitrary-rotation case now uses 45 degrees;
90 degrees is the newly supported feature. No tests were disabled.
