# Room-to-furniture bridge: pure tests executed; integration not executed

Implemented `scripts/export_room_furniture.py` and separate pure/integration
regressions. Twelve pure stage-guard and serialization methods passed locally.
The new Python files also passed syntax compilation. The twelve-method private
journal + joined map/1.5 + CLI suite has not run. No actual inventory allocation,
furniture effects, room assignments, Rust/MCP or live DFHack qualification was
established. The exact request is consumable by the existing furniture-request/1
surface; that downstream consumer must still perform its independent checks.

Earlier in the session, parent `2bc8af17b55c05f50a3ccecfef1f4cb76e701cff` built on
pinned nightly-2026-08-31. The adapter `furniture_handoff` filter ran 39 tests:
38 passed, one maximum-roster TCP test failed with a generic peer error. Commit
`28df25adba089e370c521e157cd0311c89bb4a67` separates released native-connection
ownership from local graph projection and adds a joined-peer lifecycle test.
That Rust change has not compiled or executed; the baseline failure alone does
not confirm its cause or establish that the change fixes it. A concurrently
started MCP binary build has an unknown outcome after the executor connection
became unavailable. Do not treat it as either a pass or a failure.

Beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open. Native protocols, effect
journals, dependency pins, production dispatch/admission and MCP tool count are
unchanged. Detailed contract and required runs: `docs/ROOM_FURNITURE_EXPORT.md`.
