### Lossless spatial/1.8 observation delta storage

- Wire the bounded payload codec into the actual observation-journal append,
  startup replay and historical-state paths. Retain every canonical observation,
  source digest, entity-generation transition and old raw record unchanged.
- Enable mixed raw/delta records only for sealed spatial/1.8. First/reset/every-64th
  records are raw; older and other-profile readers fail closed on the new marker.
- Verify stored checksums and predecessor identity before reconstruction, enforce
  expanded acquisition limits before allocation, and verify the reconstructed
  source before append. Sync failures cannot publish a new state, base or stats.
- Add exact payload accounting and ten registered journal regression groups.
  Together with the codec there are 18 Rust groups, all uncompiled/unexecuted here.
- Execute 1,187 independent framing-reference checks. A synthetic 256-record trace
  used 1,129,523 bytes versus 67,171,408 raw; this is not a native/live benchmark.
- No dependencies, native protocols, tools, mutation capability, retention-limit
  widening, record pruning, automatic migration or production admission change.
  Full semantics and evidence limits: docs/OBSERVATION_DELTA_STORAGE.md.
