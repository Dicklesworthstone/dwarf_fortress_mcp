# Work-package registry

Work packages are dependency-ordered evidence units, not calendar promises. A package is complete
only when code, tests, docs, schemas, and negative evidence all pass its acceptance gate.

| WP | Name | Depends on | Principal deliverable | Acceptance gate |
|---|---|---|---|---|
| WP-00 | contract repository | none | plan, registries, schemas, pure scaffold | repository validator; no false implementation claims |
| WP-01 | canonical primitives | WP-00 | IDs, digests, anchors, errors, authority, budgets | property tests and canonical vectors |
| WP-02 | world graph | WP-01 | entity/edge/fact/spatial model | integrity and ABA tests |
| WP-03 | delta algebra | WP-02 | snapshots, deltas, continuations, events | full→delta equivalence corpus |
| WP-04 | DfQL core | WP-02 | typed bounded query planner/evaluator | truth-table and budget tests |
| WP-05 | intent compiler | WP-01, WP-02 | normalized actions, constraints, sealed plans | mutation-digest and risk tests |
| WP-06 | action coordinator | WP-03, WP-05 | prepare/commit/idempotency/postconditions | exhaustive state-machine tests |
| WP-07 | obligation runtime | WP-06 | temporal completion and cancellation drain | virtual-time/model tests |
| WP-08 | bridge protocol | WP-00, WP-01 | protobuf handshake/read/mutate contracts | decoder fuzz and golden vectors |
| WP-09 | DFHack read bridge | WP-03, WP-08 | canonical fortress snapshots/deltas | compatibility fixtures and live smoke |
| WP-10 | DFHack mutation bridge | WP-06, WP-08, WP-09 | typed prepare/commit/lookup/cancel | fault-injected reconciliation suite |
| WP-11 | durable ledger | WP-06 | FrankenSQLite-backed WAL/state | crash-point matrix |
| WP-12 | checkpoint/restore | WP-09, WP-11 | sealed save snapshots and epoch reset | bit-rot/kill/restore suite |
| WP-13 | MCP server | WP-03, WP-04, WP-06, WP-07 | 11-tool narrow waist over stdio | protocol conformance and token budgets |
| WP-14 | HTTP/session mode | WP-13 | Streamable HTTP, auth, reconnect | duplicate/disconnect/load tests |
| WP-15 | leases/delegation | WP-07, WP-11, WP-13 | scoped multi-agent coordination | DPOR conflict/fencing tests |
| WP-16 | FrankenSearch attention | WP-03, WP-04, WP-11 | evidence-ranked attention and memory | deterministic relevance ledger |
| WP-17 | FrankenMarkdown knowledge | WP-04 | lossless guides/runbooks/policies | byte-span and taint tests |
| WP-18 | asupersync production runtime | WP-07, WP-10, WP-13 | structured effects/regions/lab | cancellation and replay gate |
| WP-19 | FrankenFS checkpoint backend | WP-12 | clone-aware manifests/doctor/repair | filesystem fault matrix |
| WP-20 | operations and packaging | all required | releases, doctor, bundles, support matrix | full release gate |

## Critical path

`WP-00 → WP-01 → WP-02 → WP-03 → WP-05 → WP-06 → WP-07 → WP-08 → WP-09 → WP-10 → WP-11 → WP-13`

Some packages can proceed in parallel, but no integration name may be advertised as implemented
before its package gate closes.
