## Entrance-anchored whole-room corridors

Room provisioning can connect every original area to one explicit access point
inside a bounded routing region, including obstacle detours beyond the old room
hull. The root and exact corridors enter the original floor goal, while walls,
furniture, exclusions, site masks and aggregate capacities remain enforced.
Existing no-access artifacts are byte-compatible. See `docs/ROOM_ACCESS.md`.

Executed 16 Python component tests and compared 318 legacy inputs against the
identity-checked prior compiler. No Rust/native/live or full-suite result is
claimed; the point's connection to the existing fortress remains unproven.

Automatic layout and the existing single-capture map survey now accept a fixed
`access_point`, search complete routes together with room positions/orientations,
and refuse unusable entrance tiles from the original raw map evidence. Caller
floor permissions cannot promote native walls to observed entrances. Corridor
work exhaustion now propagates as inconclusive instead of proving infeasibility.

Final scoped run: 39 tests passed, including 18 actual CLI/TCP sessions over
synthetic peers and all seven offline survey exports. A further 224 no-access
layout/observed-layout outcomes match the prior source byte-for-byte. Native
building, game execution and production qualification remain unestablished.
