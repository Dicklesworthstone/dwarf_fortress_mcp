## Cross-level room furnishing requests

Room templates now preserve per-kind `max_vertical_distance` limits into every
original furniture slot. Nonzero limits use the existing observed stair-route
allocator; default/zero artifacts retain their exact legacy bytes. No geometry,
item selector, route proof, native method or authority rule is relaxed.
See `docs/ROOM_CROSS_LEVEL_FURNITURE.md`.
