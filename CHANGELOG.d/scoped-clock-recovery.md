## Recover scoped construction-clock effects without the furniture batch

Add independent inspect/query recovery for original next-step and final clock
scopes, using only private clock files and the run credential. Validate complete
scope/intent association, retain exact terminal receipts through the existing
store, and preserve uncertainty on absence or source loss. No Batch, map read,
placement, preparation, commit, retry, cancellation or readiness claim is added.

23 real-file/TCP/CLI tests pass; six weakened implementations fail assertions.
No live DFHack, Rust/MCP or production qualification is claimed.
