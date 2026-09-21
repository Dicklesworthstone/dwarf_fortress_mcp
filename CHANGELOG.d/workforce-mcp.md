### Workforce assignment and recovery through the existing MCP interface

Add an isolated workforce/1.17 server using the existing typed WorkforceSession,
RPC adapter and durable journal. Expose exact citizen observation, capture-pinned
work-detail pages, reviewed assignment/removal, confirmed one-attempt commit,
receipt reconciliation, local/native cancellation and offline record discovery.
Keep fixed recovery modes, post-sync runtime/operator guards, historical-only
receipt semantics and explicit recovery release without quiescence claims.
Separate compact references from full reviews and reserve whole outputs before
native work. No native wire, dependency, production runner or admission changes.

Eighteen actual-path Rust test groups are registered but uncompiled/unexecuted.
Ten independent Python model/lexical groups pass; they do not execute Rust,
macro expansion, the MCP process, native DFHack or filesystem durability.
