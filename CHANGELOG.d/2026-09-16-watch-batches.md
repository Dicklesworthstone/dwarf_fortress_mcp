# Coherent foreground watch batches

## Added

- Live spatial/1.8 `fortress.query` kinds `poll_watches` and `await_watches`, with
  canonical selection of one to eight handles or the complete retained session set.
  Polling uses the current capture; awaiting acquires at most one coherent capture
  for all selected unfinished watches. Empty and entirely terminal sets acquire none.
- Reuse of the existing watch transition rules, preserving deadlines, sample cadence,
  distinct-observation stability, generation invalidation, failure guards and unknown
  evidence. Shared captures avoid observation gaps caused by separate per-watch waits.
- Complete-set publication through one optional existing-format watch checkpoint.
  Selection and registry identities are bound before acquisition and revalidated
  afterward. Full rendering precedes checkpoint sync and in-memory publication.
- Current-authority and custody checks, a shared cooperative foreground deadline,
  compact outcome records, structured next steps and live schema discovery. Archive
  and historical paths refuse watch batches. Existing single-watch APIs remain intact.

## Boundaries

This is source-present, unadmitted development functionality, not a qualified runtime.
Observation publication and watch checkpoint publication are separate durability
boundaries: an observation may be retained even when the later watch batch is refused.
An all-satisfied result describes retained terminal statuses, not simultaneous current
condition truth, continuous monitoring, or game-effect success. There are no new native
methods, dependencies, top-level tools, background tasks, game effects or admission changes.

## Evidence

Seventeen logical Rust scenarios are registered: nine batch-engine tests and eight
actual spatial-handler/private-journal tests. They have not been compiled or executed
in this environment; Rust, Cargo and rustfmt are unavailable.

The independent request-schema checker passed 96 cases (32 accepted, 64 rejected).
Its executed schema/script Git blob identities match the committed bytes. The checker
does not execute Rust, transitions, capture, custody, durability or MCP transport.
Script SHA-256: `5076e826fa9e9fdbb4440a828adb730a3d1315cb076893035dc61319c844da75`.

Usage, failure semantics, limits and focused test commands: `docs/BATCH_WATCHES.md`.
