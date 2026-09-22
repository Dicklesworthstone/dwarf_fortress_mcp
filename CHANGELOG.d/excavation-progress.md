### Recover sampled excavation goals across process restarts

Add the actual read-only `track_excavation.py` start/sample/inspect/cancel workflow
above the map/1.5 observer. Persist exact goal/source/capture evidence in bounded
private append-only journals, sync read intent before connection, and sync sample
plus parent directory before acknowledgement. Interrupted reads reset stability;
source changes invalidate, terminal evidence remains historical and immutable,
and cancellation never changes native dig obligations or game state.

All 35 actual Python/fragmented-loopback/POSIX/subprocess tests pass, with four
mutants rejected by regression assertions. Complete result/error JSON carries an
authority-free Agent Turn spine. No Rust/MCP, real DFHack, power-loss or full
qualification claim. Native wire and production admission remain unchanged.
See `docs/EXCAVATION_PROGRESS.md` for usage, goal semantics and recovery limits.
