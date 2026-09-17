## Explicit blueprint-to-watch handoff

- Add optional `monitor` settings to blueprint previews and generate a complete,
  exact-anchor `watch_request` from the disjoint layout mask. Include corridors;
  exclude reserved moat crossings and enclosed interiors without dropping tiles.
- Require future bounded deadlines, valid keys/cadence/stability and in-map geometry.
  Preserve no-auto-registration, no-mutation and shape-only evidence semantics.
- Bind proposals into continuations and whole-response budgets. Add eight Rust
  proposal/actual-handler tests for registration, stability, stale anchors and
  pagination; they remain uncompiled and unexecuted in this environment.
- Executed independent Python checks: 30 accepted and 110 rejected schema cases,
  12 runtime-relationship reference cases, and bounded largest-room request shape.
  These are not Rust/MCP/native/live qualification.
