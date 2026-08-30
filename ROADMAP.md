# Roadmap

Progress is gate-based. Dates are intentionally absent: evidence matters more than calendar theater.

## Phase 0A — Executable contract scaffold

- [x] motivating-proposal and prior-art research;
- [x] initial comprehensive plan and frozen 11-tool waist;
- [x] hard invariants and registries;
- [x] safe-Rust workspace scaffold;
- [x] typed anchor/capability/budget/evidence/error contracts;
- [x] canonical graph/map/predicate/delta shapes;
- [x] semantic plan/obligation contracts;
- [x] adapter trait and deterministic in-memory pause demo;
- [x] prospective MCP and bridge schemas.

## Phase 0B — Franken-substrate design lock

- [x] source-level deep dives across asupersync, FrankenSQLite, FrankenFS, FrankenSearch, FrankenMarkdown, FrankenGraphDB, FrankenNetworkX, and DSR;
- [x] one-version-universe and world-MVCC specification;
- [x] hierarchical read/write/negative witness model;
- [x] graph projection and canonical algorithm registry;
- [x] ATP state/evidence plane;
- [x] closed dependency allowlist and latest-nightly policy;
- [x] root-last publication registry;
- [x] local/self-hosted qualification and DSR release specification;
- [ ] run and repair all nightly Rust gates on a machine with the toolchain;
- [x] adopt the owned `fastmcp_rust` sibling as the MCP plane, modern-only MCP 2026-07-28, pinned
  for dogfooding (ADR-013), with a laboratory stdio server exposing the 11-tool waist;
- [ ] freeze registry v0 after public review.

Exit: `GATE-010` plus `SUBSTRATE-G1` design acceptance.

## Phase 0C — Owned MCP transport (laboratory)

- [x] `dfmcp-mcp` crate: all eleven `fortress.*` tools over stdio via `fastmcp-rust` on
  `MemoryAdapter` (open/observe/plan/commit/wait/cancel/checkpoint/restore/explain/doctor);
- [x] machine-enforced modern-only profile (no `legacy-2024-11-05` graph; `tasks` on; exact rev pin);
- [ ] session-scoped capability negotiation replacing process-local laboratory state;
- [ ] MCP Tasks store backed by the bounded-obligation engine (`ServerBuilder::final_tasks`);
- [ ] MCP 2026-07-28 conformance evidence recorded in `docs/DOGFOODING_FASTMCP.md` (WP-21);
- [ ] first upstream defect loop completed end to end (file → fix → pin bump → conformance note).

Exit: `WP-13` gate closure plus the first completed upstream fix cycle.


## Phase 1 — Reference version universe

- implement state-anchor v2;
- implement immutable observation capsules;
- complete field presence, completeness, provenance, and generation-safe references;
- root-last in-memory publication;
- exact historical reads and reachability-based retention;
- full snapshot versus capsule replay differential tests;
- reference graph projections and canonical tie-break policy.

Exit: `SUBSTRATE-G1` executable evidence.

## Phase 2 — Owned runtime and witnessed plans

- integrate `asupersync` as the sole runtime and lab;
- region ownership for sessions/plans/obligations/evidence;
- context-carried authority and multidimensional budgets;
- cancellation progress certificates;
- read/write/negative witnesses;
- deterministic rebase and merge certificates;
- plan dependency graph and conservative refinement.

Exit: `SUBSTRATE-G2` and `SUBSTRATE-G3`.

## Phase 3 — Read-only DFHack bridge

- finalize the bounded bridge subset and handshake;
- compatibility profile and operation/capability discovery;
- fortress identity, tick, pause state, units, jobs, buildings, work orders, resources, events, and selected map chunks;
- full reads plus capsule derivation;
- doctor and golden fixtures;
- differential comparison against independent DFHack scripts;
- disconnect, restart, malformed-frame, and epoch-reset campaigns.

Exit: `GATE-020` and `GATE-030`.

## Phase 4 — MCP read and cognition plane
- stdio and Streamable HTTP lifecycles through the pinned `fastmcp_rust` modern-only plane
  (stdio laboratory slice already landed in Phase 0C);
- open session, observe, query, explain, and doctor;
- interest sets, continuations, and output budgets;
- read-only capability delegation;
- graph algorithms with decision/complexity witnesses;
- immutable search and knowledge generations;
- progressive attention with certified completeness/freshness.

Exit: `GATE-040` and `SUBSTRATE-G4`.

## Phase 5 — Shadow planning

- action registry engine;
- live reference resolution;
- predicted semantic diff and conflict footprint;
- counterfactual branch-per-agent planning;
- graph/resource analysis;
- rebase and merge explanations;
- no live mutation available in this mode.

Exit: `GATE-050`.

## Phase 6 — First reversible effects

Order:

1. pause/resume;
2. labor settings;
3. burrow membership;
4. stockpile configuration;
5. work-order setup.

Each ships independently with prepare, witnesses, idempotency, short-lived effect ticket, bridge journal, operation lookup, observed postcondition, cancellation/reconciliation, compensation where valid, crash replay, and version certification.

Exit per family: `GATE-060`.

## Phase 7 — Checkpoint custody and ATP

- checkpoint object graph and root-last publication;
- restore with new epoch and stale-handle invalidation;
- local ATP transfer, resume, corruption, and reconstruction;
- evidence bundles and retrievability audits;
- doctor, sealed repair plan, revalidate, apply;
- indeterminate-effect recovery.

Exit: `GATE-070`, `GATE-090`, and `SUBSTRATE-G5`.

## Phase 8 — Persistent Franken adapters

Admit independently:

- FrankenSQLite ledger/MVCC implementation;
- FrankenFS checkpoint/evidence implementation;
- FrankenSearch retrieval/attention implementation;
- FrankenMarkdown knowledge implementation;
- FrankenGraphDB projection/incremental-query implementation;
- selected pure-Rust FrankenNetworkX algorithm crates;
- ATP remote/multi-donor topology.

Each must preserve reference semantics and demonstrate a workload benefit.

## Phase 9 — Guarded spatial and logistics work

- exact map masks and path witnesses;
- designations and construction;
- material and labor reservations;
- connectivity/articulation/dominator safety analysis;
- min-cost-flow and matching candidate planners;
- environmental hazards;
- long-running obligations.

Exit per family: `GATE-080`.

## Phase 10 — Multi-agent control

- delegated capabilities;
- branch-per-agent isolation;
- hierarchical witnesses and concurrent disjoint commits;
- scoped leases/fencing;
- clock coordinator;
- fairness/preemption and swarm soak.

Exit: `GATE-100`.

## Phase 11 — Local release qualification and higher-risk domains

- same-binary performance receipts;
- controlled Linux/macOS/Windows DSR builds;
- exact asset contracts, checksums, signatures, SBOMs, and qualification manifests;
- supported compatibility window;
- public negative-evidence corpus;
- carefully selected military/governance effects;
- security and recovery review.

Exit: `GATE-110` and `SUBSTRATE-G6`.

## First implementation sequence

1. make the current workspace pass latest-nightly local qualification;
2. implement anchor v2 and capsule publication in the reference core;
3. implement exact read and negative witnesses;
4. implement deterministic rebase certificates;
5. introduce `asupersync` regions/Cx/lab with no second runtime;
6. implement read-only bridge handshake and bounded pause/tick/identity observation;
7. expose MCP open/observe/doctor over stdio through the pinned fastmcp_rust modern-only server
   (laboratory slice landed; session-scoped authority next);
8. implement traversal and plan-DAG reference graphs with canonical tie-breaks;
9. add shadow pause plan;
10. add live idempotent pause commit with lookup/observe/prove;
11. add checkpoint root and restore epoch;
12. qualify locally through DSR before any release claim.
