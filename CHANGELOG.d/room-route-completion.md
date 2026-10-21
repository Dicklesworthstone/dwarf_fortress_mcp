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
