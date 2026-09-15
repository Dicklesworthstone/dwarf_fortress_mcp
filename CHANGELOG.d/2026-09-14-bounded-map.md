# Bounded map/1.5 observation and candidate routes

- Added a fixed-region native DFHack read profile with at most 16,384 tiles,
  128-tile sides and a 1-MiB payload bound. Hidden cells carry no terrain
  attributes; unallocated blocks are not created or treated as empty space.
- Added strict safe-Rust decoding, exact region/manifests, atomic publication,
  physical tile identities, source digests and epoch/generation handling.
- Added deterministic cardinal floor/stair BFS with explicit excluded endpoints,
  complementary vertical stairs, operation budgets and whole-vertex paging.
  Results are model candidates, not unit paths, safety or global absence proofs.
- Registered `dfmcp-live-map-dev-server` using the existing owned runtime and
  eleven tools. Typed tile queries, aggregates, baselines and condition watches
  use the published terrain projection. Route continuations bind session,
  complete anchor, policy, endpoints and work limit while keeping active work.
- Added seventeen registered Rust scenarios: six route, three codec/projection,
  two RPC and six actual-handler tests, including an 8,192-byte route page path.
- Both GCC and Clang mock-interface builds of the actual native producer passed
  110 checks and matched an independent 455-byte encoder. The reproducible
  harness is `scripts/test_live_map_native_mock.py`.
- Independent Python design checks passed 512 exhaustive obstacle models and
  500 seeded 3-D shape models. Schema validation passed 64 cases (19 accepted,
  45 rejected). These checks did not execute the Rust implementation.
- Rust compilation/tests, rustfmt, Clippy, stdio, real DFHack/protobuf builds and
  live execution remain unverified. No native qualification or admission claim.
- Map observations remain separate from operations/citizen worlds; mutable region
  windows, terrain archives, full unit navigation and live mutations are absent.
  Existing native profiles, dependency pins and production map are unchanged.
