# Prospective live announcement generation

- Added a versioned protocol-1.1 design for authenticated, bounded `ReadAnnouncements` access.
- Added explicit retained-window semantics with a frozen report-ID high-water mark, monotonic continuation, and honest history-truncation coverage.
- Added a safe-Rust canonical announcement-window assembler whose identity is independent of transport page size.
- Added bounds for text, pages, total records, and canonical bytes, plus adversarial tests for malformed records, cursor gaps, reordering, window drift, partial mutation, and tampering.
- Added static contract validation, negative fixtures, agent-facing epistemic guidance, and a source-only qualification wrapper.
- Kept the generation explicitly unadmitted: native DFHack extraction, safe-Rust RPC decoding, canonical event projection, MCP exposure, live evidence, and exact compatibility promotion remain pending.
- Preserved the immutability of the first admitted protocol-1.0 citizen-only tuple; no existing compatibility entry is broadened.
