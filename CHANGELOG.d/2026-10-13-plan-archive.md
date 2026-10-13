# Recover exact original plans without recompilation

- Add a bounded, lossless prepared-plan archive with separately pinned semantic
  and archive-object identities, closed enum tags and exact decode/re-encode.
- Preserve all original goals, action keys, dependencies, compensations,
  checkpoints, deadlines, cadence and stability requirements; restore no authority.
- Bound serialization before legacy normalization/hashing. Refuse unknown tags,
  duplicate keys, oversized input, corruption and changed externally pinned seals.
- Add thirteen Rust regressions (unexecuted). The independent Python construction
  verifies the retained 324-byte pause fixture and its semantic/object identities.
- No canonical digest migration, native protocol change or live admission.

## Durable custody and native reattachment

- Add an immutable, non-cloneable plan archive owner with sync-before-publication,
  byte-exact verification, current Query/Plan checks, session/context floors and
  no overwrite, tail repair or truncation. Reuse the existing private-file backend.
- Reattach archived original work orders through the existing native association
  checks with the remaining operation allowance. No insertion, retry, reconciliation,
  goal proof, deadline renewal or authority restoration is implied.
- Add sixteen Rust store regressions, including private-file custody and injected
  publication faults. All 29 codec/store Rust tests remain unexecuted here.
- Independent Python construction/framing checks pass the 324-byte vector and
  reject 651 malformed framing/identity cases. This is not Rust or native evidence.
