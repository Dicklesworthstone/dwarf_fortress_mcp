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
