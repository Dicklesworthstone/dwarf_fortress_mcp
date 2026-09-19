## Explicit spatial/1.8 source recovery

- Add `fortress.query` kind `recover_source` for fenced live read sessions. Retain
  session, volatile baselines, watch handles and definitions instead of forcing
  close/reopen after a transport failure.
- Publish a bounded, optionally durable monitoring-gap transition before one
  reconnect/handshake and one coherent capture. Reset unfinished stability without
  sampling, extending deadlines or rewriting terminal evidence. Repeated failed
  attempts at the same anchor do not duplicate the transition.
- Reuse the existing complete-observation publication path, with both Query and
  Observe revalidated at the target even without a journal. Preserve source,
  anchor, acquisition, custody and output/deadline fences. Keep archives read-only.
- Reserve full recovery responses before work and distinguish retained gap
  progress from a failed reconnect. No mutation, automatic retry, journal repair,
  dependency, top-level tool, native wire or admission change.
- Add 18 unexecuted Rust regressions. Execute 51 isolated schema-envelope cases
  and 6 independent composition-reference cases. Rust/Cargo/rustfmt unavailable;
  no Rust, MCP, storage crash, native/live or repository qualification claim.

Implementation and evidence: `docs/SPATIAL_SOURCE_RECOVERY.md`. Current compatibility
registry and production runner map remain unchanged.
