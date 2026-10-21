# Joint room-backed route completion

- Retain the complete original room parent and every original placement receipt
  in one fixed-deadline, shared-stability completion goal.
- Require matching paused map endpoints, original sources and target occupancy;
  preserve failure, deadline, sample-limit and interrupted-read semantics.
- Add query-only acquisition, a distinct append-only private monitor and
  start/sample/wait/inspect/cancel commands. Preserve original batch custody
  through publication; never retry placement or advance game time.
- 38 focused new tests and 27 existing/local runner regressions pass. Four weakened
  variants fail assertion checks. Actual monitor files and shared reducers run;
  native, original-parent and room-diagnosis producers are injected test ports.
- Include the previously unpublished 12 room-run regressions without changing
  the runner. No native/TCP, Rust/MCP, SDK or production-admission claim.

Beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open. See ROOM_ROUTE_COMPLETION.md.

## Generation 2 completion requirements

- Derive circulation and project-access completion requirements from the retained
  original RoomPlan for newly created monitors; do not expose weakening flags.
- Add `DFMRQG02` while retaining exact decode/encode behavior for existing
  `DFMRQG01` goals.
- Reuse the same paused map capture for circulation evidence and reset the shared
  stability streak when circulation/access is not established.
- Surface requirement flags in progress and CLI output.
- Add focused source regressions for legacy framing and v2 access behavior.
  These new tests are source-present but unexecuted in this environment.

