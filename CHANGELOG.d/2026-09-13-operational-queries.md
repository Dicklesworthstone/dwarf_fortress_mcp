# Operational queries through the existing live development MCP tool

Added `aggregate` and `search` variants to `dfmcp.query/1`, dispatched by the
existing protocol-1.1 `fortress.query` handler. The original entity/inspection/graph
executor and its tests are preserved byte-for-byte in `semantic_query_core.rs`.

Summaries group by entity kind or a typed field, retain distinct unknown/omitted/
redacted/absent states, and compute explicitly typed exact numeric statistics.
Search uses bounded Unicode-lowercase substring terms over labels and explicitly
selected known text fields. It keeps only a page of ranked references, binds
continuations to request/session/full anchor, and returns inspectable hit identities.

Twelve added Rust tests are source present, not executed here. JSON Schema 2020-12
validation and 19 request acceptance/rejection checks passed in Python. No Rust
compiler, Cargo, or rustfmt is available in this editing environment. No live
admission, new native observation domain, mutation, or full qualification claim.
