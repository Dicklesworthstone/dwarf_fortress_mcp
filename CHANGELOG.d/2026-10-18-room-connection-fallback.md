## Complete corridor fallback with replay-safe layout selection

Connect complete room projects that fit the tile budget only with shared
corridor junctions. The bounded exact fallback preserves original floors,
protected cells, entrances and the caller's remaining computation budget.

Review fixed a saved-layout compatibility regression: automatic layout search
must explicitly select `connection_strategy: greedy-then-minimum-tiles/1`.
Old searches keep their original outcomes, selected positions, digests and work
counts. Direct room compilation rescues previously refused requests unchanged.

The example retains eight bedrooms, two dining halls and all 28 furnishings in
512 tiles. Sixty-five warnings-denied Python tests pass, including real CLI
solve/replay/export. No native/live/production admission claim is introduced.
See `docs/ROOM_CONNECTION_BUDGET_FALLBACK.md`.
