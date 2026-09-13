# Coherent operations development profile

- Add isolated operations/1.3 DFHack producer and closed authenticated Rust client
  for jobs, buildings, inventory, containers/holders, and job-item attachments
  acquired in one bounded native observation rather than a cross-time merge.
- Validate complete rosters, identity horizons, exact relationship endpoints,
  attachment counts, containment acyclicity, canonical wire bytes, and source
  identity before atomic multi-domain publication. Preserve stable entity/edge
  IDs, generation tracking, explicit heartbeats and shared epoch resets.
- Register `dfmcp-live-operations-dev-server` and its independently gated library
  entry. Preserve eleven tools and expose the common typed queries, aggregates,
  search, graph paths, baselines and foreground watches over actual operations
  data. No placeholder citizens, usable-supply claim, or live mutation authority.
- Reuse existing bounded native framing/deadline primitives and shared query
  engines. Existing native protocols and the production runner map remain unchanged.
- Add 18 registered Rust tests and a reproducible native-source mock harness.
  GCC and Clang each passed 135 mock-interface checks and matched the independent
  423-byte golden frame. Rust compilation/tests and actual DFHack/native/live
  qualification were not available or established. Exact evidence limits and
  workflow are in `docs/LIVE_OPERATIONS.md`.
