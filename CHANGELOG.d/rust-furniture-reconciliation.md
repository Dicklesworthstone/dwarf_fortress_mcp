## Explicit original-receipt recovery in the Rust furniture journal

- Add a query-only reconcile_receipt API for retained Indeterminate placements.
- Keep ordinary native successors, commit replay and cancellation immutable.
- Decode complete original positive proof, preserve dispatch/cancellation history,
  and append/sync/readback before publishing resolution. Never regain a permit.
- Reserve a final proof frame and revalidate exact source, authority and custody.
- Add eight Rust regression tests (uncompiled/unexecuted: no Rust toolchain here).
  Native source, Python recovery and production admission are unchanged.
