# Coherent spatial observations and route-aware inventory

- Added a fixed spatial/1.6 native profile that captures jobs, buildings, items,
  attachments and a bounded terrain region in one RPC suspension before immutable
  paging. It does not join independently timed profile observations.
- Added composite source validation and atomic canonical publication with one
  source digest, observation cursor and entity-generation universe across both
  domains. Hidden terrain remains redacted; invalid components reject the capture.
- Added reusable bounded terrain reachability and route-aware declared inventory
  allocation. Outermost ground-container positions and conservative inherited
  exclusions feed the existing integral allocator without double-counting supply.
- Added `spatial_inventory_plan`, same-anchor route drill-downs and whole-row
  pagination retaining current watches and complete Agent Turns. Conditional model
  results do not claim unit access, native material eligibility, safety, global
  inaccessibility, reservations or executable plans.
- Registered `dfmcp-live-spatial-dev-server` with separate credentials, opt-in,
  session family and capacity. Existing query/baseline/watch engines operate on
  the combined snapshot; terrain watches and inventory baselines share one refresh.
- Added 23 registered but unexecuted Rust scenarios. Both GCC and Clang native
  mock runs passed 786 checks, including an independently encoded 810-byte fixture
  and a 40,000-item, 2,200,457-byte capture across 135 immutable pages. Schema checks
  passed 59 cases; an independent ancestry-design oracle checked 2,000 forests.
- Rust compilation/tests, rustfmt, Clippy, stdio, real DFHack headers/protobuf
  linking/loading, live-game behavior and full repository qualification remain
  unverified. Native mock results are not real DFHack qualification.
- Existing native profiles, dependency pins, production admission and the active
  runtime-migration bead are unchanged. Citizen coverage, full navigation/native
  requirements, durable spatial history/supervision and live mutations remain
  unfinished. See `docs/LIVE_SPATIAL.md` and `IMPLEMENTATION_STATUS.md`.
