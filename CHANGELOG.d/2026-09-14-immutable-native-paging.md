# Immutable native operations paging

- Added the separate operations/1.4 native profile, immutable snapshot cache,
  bounded byte pages and whole-payload SHA-256. Native capture is still one
  coherent suspended read; later pages do not reread game state.
- Added explicit 65,536-item/16-MiB codec limits, distinct profile provenance,
  strict assembly, fixed plugin/type negotiation, one whole-acquisition deadline,
  explicit release and permanent failure fencing before semantic publication.
- Registered `dfmcp-live-operations-paged-dev-server` with separate operator
  credentials, opt-in and session family. Reused existing query, production,
  baseline and watch handlers without adding tool names or game authority.
- Preserved old native sources, default 1.3 codec ceilings, allocation policies
  and dependency pins. The paged entry rejects 1.3 journal configuration and
  history requests rather than relabeling archive evidence.
- Added fifteen registered, unexecuted Rust scenarios and a reproducible native
  mock harness. GCC and Clang each passed 748 checks, including a 40,000-item
  observation over 135 pages, the 65,536-item ceiling, cache lifecycle and eleven
  SHA-256 vectors. This is not Rust execution or real DFHack qualification.
- Documented source integration and limitations in `docs/OPERATIONS_PAGING.md`.
  Real native/Rust/stdio/live qualification and 1.4 archive support remain absent.
