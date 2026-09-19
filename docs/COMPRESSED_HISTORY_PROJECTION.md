# Compressed multi-record history projection

The delta-storage integration updated single-record and startup replay, but left
`project_records` calling `decode_profile_frame`. That helper had become
`#[cfg(test)]`, so the production call named a test-only item. Even a test build
could not replay a delta through it: the helper supplied no predecessor payload.
This affected the existing quantity/condition `historical_series` feature.

The multi-record reader now uses `compression::decode` directly, with the same
expanded acquisition budget as startup and single-record replay. It retains one
local verified payload base and advances that base for every prefix record, not
just rows requested by the caller. Sparse selections and subsequent pages can
therefore reconstruct delta chains without assuming that the current live
payload is their predecessor.

Checksums, exact recorded anchors, source digests, entity generation history,
current authority, complete-request validation, callback/output refusal, final
custody checks and the shared deadline remain in force. Only projected values
are collected; journal bytes, current state, compression statistics and the next
append's base are unchanged. Existing raw-only profile behavior is retained.

Eight Rust regressions are registered in
`observation_projection_compressed_tests.rs`, alongside the five original raw
projection tests. They cover a mixed raw/delta archive, sparse selections, replay
across keyframes and resets, reopen, failed projections and storage corruption.

The editing environment does not contain Rust, Cargo or rustfmt. These scenarios
are source, not executed Rust evidence. No full build, MCP, native DFHack or live
qualification is claimed. The normal-build test-only reference is established by
source inspection, not a captured compiler run. The compatibility registry and
production runner map are unchanged.
