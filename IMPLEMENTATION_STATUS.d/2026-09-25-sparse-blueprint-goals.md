### Sparse multi-level terrain goals and durable foreground CLI

Added `scripts/excavation_blueprint.py` and integrated it into the actual
`track_excavation.py start-blueprint/sample/inspect/cancel` workflow. The new
separate journal profile retains the whole validated mask and native evidence,
so monitoring resumes after process restart or loss of the original input file.
Legacy floor journal bytes, limits and goal semantics remain unchanged.
See `docs/EXCAVATION_BLUEPRINTS.md` for the runnable contract.

All 63 focused Python tests pass: 35 unchanged observer/legacy tracker tests,
12 blueprint model tests and 16 new file/TCP/subprocess/fault integration tests.
Coverage includes 4,608 shape/liquid/designation cases, 500 differential traces,
1,024-cell coherent captures, 512-target/32-part bounded output, restart, profile
isolation, source changes, interrupted reads, terminal durability acknowledgement
faults, torn writes and retention-bound cancellation. Suites are wired into
`verify.sh`; the full repository gate itself was not run in this partial checkout.

The unchanged decoder/client source and original test files were reconstructed
and checked against Git blob identities before execution. No Rust toolchain was
available. This is not Rust/MCP, native SDK, live-game, physical power-loss,
whole-repository or admission evidence. The Python blueprint monitor does not
discharge a native mining obligation or authorize retries. No native protocol,
dependency, production map or admission changes. Owner:
`df-action-coordinator-exec-ero.4`; the broad bead remains open.
