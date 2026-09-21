### Durable Rust mining coordination and offline record discovery

Compose the existing dig/1.16 codec/RPC with a bounded source- and map-scope-bound
journal: sync intent before native prepare, consume process-local permission and
sync dispatch before the sole commit, and sync complete terminal proof before
acknowledgement. Reopen/replayed/query preparations never regain dispatch rights.
Recover exact records by query or authorized native cancellation; permanent
Unknown and every other nonterminal record block new keys throughout the journal.
Add session/head-bound record pagination and full key/digest review without DFHack.
Require an explicit supervising runtime guard at native edges, including after
sync; no built-in permissive policy, new dependency, native wire or MCP route.

Sixteen Rust test groups are registered but uncompiled/unexecuted. Independent
Python framing/transition checks pass (7,752 corruptions, 7,748 incomplete prefixes,
100 transition pairs); these do not execute Rust or establish native/live or
filesystem qualification. See docs/DIG_RUST_COORDINATOR.md for remaining runtime
lease/checkpoint integration and the exact development-only evidence boundary.
