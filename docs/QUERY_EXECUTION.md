# Bounded canonical query execution

`dfmcp_world::execute_query` and `execute_bounded_query` use the `query_page`
executor. The private evaluator remains the reference implementation of typed
predicates, deterministic ordering, row limits, and whole-row byte limits.

## Continuation contract

Treat returned continuations as opaque. A page is bound to the fortress,
observation epoch and sequence, game tick, canonical state hash, kind selectors,
predicate tree, and ordering. Changing the requested result set or resuming on a
different snapshot refuses with `ERR-STALE-ANCHOR`. The snapshot's stored hash is
verified before execution. Kind selectors are treated as a set; custom kind names
remain distinct from built-ins. Predicate tree order is intentionally part of
this version's query identity.

Page width and output-byte budget may change between pages. An oversized first
row returns `ERR-BUDGET-EXCEEDED` instead of emitting a non-progressing cursor.

The `q1` format replaces query use of the old offset-only `cont` format. Legacy
query continuations return `ERR-CURSOR-GAP`; restart the query. Delta continuation
encoding is unchanged.

This digest binding is **not authentication or a capability**. It detects edits
and unintended reuse, but does not prevent a caller from computing a fresh
commitment. The adapter must authorize every request and bind client-facing
continuations to session ownership. A continuation cannot grant read authority.

## Work and input limits

Before predicate execution, the evaluator refuses a conservative bound greater
than 4,000,000 entity-selector/predicate operations. This prevents individually
legal entity and predicate limits from multiplying into billions of evaluations.
It is an operation-count guard, not a measured wall-time guarantee. Canonical
snapshot hash verification remains a full snapshot operation.

Predicate shape/value limits remain in force. Query identity encoding additionally
bounds aggregate bytes to 256 KiB and nested kind names to 128 bytes. No predicate
is weakened, normalized to true, or partially evaluated to evade a budget refusal.

## Evidence

`query_page_tests.rs` covers all four ordering modes, repeated pages, changes in
page width and byte budgets, cross-query reuse, every anchor component, same-cursor
forks, malformed and edited tokens, legacy offsets, kind-name aliases, invalid
snapshot hashes, and aggregate input/work bounds. The public API truth-table suite
also resumes a returned bound cursor.

These additions are source implementation and regression tests, not live-bridge
admission, runtime qualification, or proof of a successful Rust test run.
