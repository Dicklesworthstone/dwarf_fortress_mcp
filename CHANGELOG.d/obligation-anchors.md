# Obligation observation-lineage fences

- Bind accepted obligation evidence to fortress, epoch, sequence, tick and digest;
  reject regressed, cross-lineage or forked observations before publishing a batch.
- Track off-cadence reads without delaying positive polls; prevent backdated cancel.
- Add anchored registration, last-anchor inspection and explicit interrupted-read
  stability reset while retaining the tick-only registration API.
- Add ten Rust regression tests for `df-action-coordinator-exec-ero.4`, uncompiled
  and unexecuted here. No durable, runtime, native or production claim is made.
