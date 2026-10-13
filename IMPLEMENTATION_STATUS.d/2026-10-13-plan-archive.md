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
