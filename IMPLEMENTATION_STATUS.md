# Implementation Status

This file is the authoritative antidote to accidental overclaiming.

## Current phase

**Phase 0C-A1: modern MCP transport, semantic-contract laboratory, and additive agent-orientation
facade.** The repository contains a substantial executable scaffold, but it is not a production
MCP server and does not connect to DFHack. The separately installed Dwarf Fortress + DFHack stack
is a reference environment for future integration work, not evidence of live control.

The target architecture specifies multi-version world state, read witnesses, semantic rebase,
proof-carrying merge, canonical graph behavior, immutable publication, ATP-backed evidence
movement, structured concurrency, objective decomposition, counterfactual comparison, surprise
records, and evidence-gated agent memory. Most of those are design commitments or in-memory
laboratories. Their presence as types, documents, or tests must not be described as a durable or
live implementation.

The presentation crate uses exact-revision-pinned `fastmcp_rust`, with default features disabled
and only the modern MCP 2026-07-28 Tasks feature admitted. Pin changes require the record in
`docs/DOGFOODING_FASTMCP.md`.

### Agent-operating-model status

The following has landed:

- a normative synthetic control loop in `docs/AGENT_OPERATING_MODEL.md`;
- a machine-readable contract in `architecture/agent_turn_contract.json`;
- an authority-free `AgentTurnBuilder` in `dfmcp-mcp`;
- an additive agent-oriented facade over all eleven laboratory tools;
- the facade is the default `dfmcp_mcp::run_stdio` entrypoint;
- every facade result attempts to expose continuity, briefing, changes, attention, active work,
  affordances, recommendations, uncertainty, coverage, budget, and references;
- prepared plans remain visible in later turn packets, unchanged observations are classified as
  heartbeats, restore is classified as an epoch reset, and errors carry structured recovery
  guidance;
- the original authority-bearing laboratory handlers remain intact under the facade.

The following has **not** yet landed and must not be inferred from the packet shape:

- the authority-bearing semantic request ID is not yet projected; the facade currently emits a
  clearly namespaced presentation-turn identifier;
- the orientation cache is process-local, bounded, non-durable, and never authoritative;
- the laboratory packet covers pause/protocol state only, not units, items, jobs, map, economy,
  welfare, military, or live game state;
- `pulse`, `briefing`, `tactical`, and `forensic` are currently presentation profiles over that
  small slice, not full target-state implementations;
- recommendation utility, information value, cost, and confidence are structured placeholders
  where the laboratory has no empirical model;
- objectives, candidate-set comparison, surprise persistence, handoff resources, and memory
  promotion are specified but not yet executable end to end;
- the new Rust commits have not been locally compiled or Clippy-qualified in this editing
  environment. Only a passing local qualification receipt for the exact revision can establish
  that evidence.

| Area | Present now | Not yet present |
|---|---|---|
| Architecture | three-plane target model, one agent operating loop, invariants, design documents, machine-readable dependency/publication/graph/agent registries | empirical validation against a live, full-scale fortress |
| Agent surface | additive Agent Turn Packet builder; all eleven laboratory tools wrapped by an oriented facade; bounded process-local continuity/active-work projection; structured affordances, recommendations, uncertainty, coverage, and recovery | durable handoff, complete profiles, live attention, empirical cost/VOI models, objective hierarchy, candidate comparison, durable surprise/learning loop |
| Core | typed IDs, SHA-256, anchors, risks, capabilities, scopes, budgets, evidence, stable errors; experimental in-process lease, clock, and role/delegation models | authenticated principals, durable/distributed fencing, production authorization service, shared typed agent-turn source model |
| World | typed graph, provenance-bearing facts, canonical snapshots, normalized predicates, strict deltas, generation/revision checks; experimental in-memory spatial, topology, rebase, Merkle, ATP, search, archive, checkpoint, and table-ledger models | admitted Franken-suite integrations, durable MVCC/WAL, crash recovery, production indexes, complete epistemic/coverage annotations |
| Intent | semantic action vocabulary, normalization, sealed plans, pre/postconditions, bounded obligations; experimental fail-closed blueprint, logistics, labor, alert, and obligation laboratories | qualified execution semantics beyond the laboratory pause action; objective/candidate/surprise implementation; terrain predicates that prove blueprint completion |
| Adapter | transport-independent `GameAdapter` trait; experimental framing/transceiver and delta laboratories; fail-closed DFHack transport-liveness probe | authenticated handshake, canonical payload codecs, live DFHack observation or mutation adapter |
| Bridge | protobuf design contract, experimental framing header, unconnected Lua research helpers, and a C++ placeholder whose initialization deliberately fails | genuine DFHack plugin registration/linkage, supported API calls, authorization, interoperability, installation target |
| Lab | deterministic in-memory snapshot and pause-only adapter with prepare/commit checks, idempotency, polling, compensation, process-local checkpoint/restore, transcript, and oriented MCP projection | equivalence with live DFHack behavior, durable effects, full action support, full fortress briefing/attention model |
| MCP | modern-only stdio laboratory exposing the 11-tool surface through pinned `fastmcp_rust`; process-local sessions and authority tests; agent-oriented default facade | live adapter selection, authenticated deployment, persistence, qualified HTTP/Tasks integration, shared schema-catalog admission of agent-turn fields |
| Persistence | effect/publication/checkpoint designs plus in-memory table and archive prototypes | SQLite/FrankenSQLite, FrankenFS, WAL, process-crash recovery, durable compaction |
| Search/graph | deterministic fixed-point lexical and basic graph laboratories plus target registries | FrankenSearch/graph-engine integration and production-scale qualification |
| Transfer | local proof-capsule and Merkle integrity models | ATP transport, peer exchange, anti-rollback/retrievability qualification |
| Release | self-hosted workflow specifications and local qualification receipt generator | signed cross-platform release evidence and installable release artifacts |
| Live game | a separately installed DF Classic + DFHack reference stack for research | any Rust-to-DFHack connection, authoritative live observation, or live mutation |

## What “working” means today

The executable scaffold can:

- freeze vocabulary, canonical identities, state shapes, protocol boundaries, and the agent
  operating loop;
- demonstrate plan sealing, exact prepare/commit matching, stale-anchor checks, scoped
  authorization, idempotency, restore invalidation, compensation, and postcondition verification
  for a small in-memory pause-state action;
- exercise deterministic integrity and planning models under unit and integration tests;
- expose the MCP tool shape over stdio using process-local laboratory sessions;
- orient an agent over the process-local pause-state slice without requiring it to reconstruct
  pending-plan and last-action state from unrelated historical responses.

It cannot:

- connect to DFHack, load a fortress, observe authoritative game state, or mutate a real game;
- provide durable MVCC, a SQLite WAL, process-crash recovery, Franken-suite storage/search, or
  ATP replication;
- establish performance, security, compatibility, release, or full agent-ergonomics evidence from
  unit tests or response shape alone;
- be installed as a finished end-user MCP server;
- claim that recommendations are strategically intelligent about Dwarf Fortress beyond the
  current pause/protocol-state laboratory.

DFHack's built-in remote service is protobuf over TCP, not gRPC. The repository's
`proto/dfmcp.proto` is a proposed contract; there is no generated gRPC server or client.

## Validation status

The normative qualification path is:

```bash
./scripts/qualify_local.sh
```

It runs repository contracts, dependency policy, shell syntax, locked/offline Cargo metadata,
rustfmt, Clippy with denied warnings, debug and release tests, rustdoc, and executable contract
checks, then writes a machine-readable receipt under `target/qualification/`.

Qualification applies only to the exact source revision and receipt named by a run. A passing
unit test for an in-memory prototype is not live-game, durability, performance, compatibility, or
agent-strategy evidence. `DFMCP_ALLOW_DIRTY=1` permits development qualification of a dirty tree,
but that result is not release evidence. Current `fastmcp_rust` conformance limitations are
recorded in `docs/DOGFOODING_FASTMCP.md`; do not infer a full modern lifecycle pass from negative
fixtures.

The environment used for the current direct-to-main editing session has no Rust toolchain and no
network path from the shell. Therefore the agent-oriented facade commits are **unqualified source
changes** until `./scripts/qualify_local.sh` passes on one of the project's controlled machines.
This limitation is intentionally recorded here rather than hidden behind confident prose.

## Status rules

1. Only this file and acceptance evidence may define implementation status.
2. Prospective README prose describes the target system, not current completion.
3. A feature is `experimental` only after it executes.
4. A feature is `supported` only after its acceptance gate passes for named versions.
5. A feature is `production` only after release, recovery, security, compatibility, live-game,
   and agent-resumption gates pass.
6. Negative evidence may reject a feature but cannot certify success.
7. A derived index, checkpoint replica, attention item, affordance, recommendation, memory item,
   or evidence bundle is never more authoritative than the canonical observation/effect history
   from which it was built.
8. No unit test substitutes for disposable-fort evidence where Dwarf Fortress effects matter.
9. A structurally complete Agent Turn Packet does not establish that its strategic content is
   complete, correct, or live.
10. Presentation continuity state may improve ergonomics but may never participate in mutation
    authorization or proof.

## Next executable milestones

### Immediate local gate

Run local qualification on the current `main`, fix every compile/rustfmt/Clippy/test/rustdoc issue,
and attach the receipt to the exact revision. This is required before calling Gate A1 executable.

### Agent Gate A1 completion

Move the common turn inputs closer to the semantic source of truth so the packet carries:

- the real authority-bearing request ID;
- an exact complete anchor on every session-bound success and error;
- profile selection as an actual bounded request input rather than facade metadata;
- deterministic budget consumption and remaining-budget accounting;
- active-work state derived directly from the adapter/session ledger rather than a presentation
  cache;
- golden vectors for every success/error/restore/heartbeat transition.

### Live read-only bridge milestone

The next game-facing milestone remains a read-only bridge handshake plus one coherently published
observation capsule containing:

- authenticated bridge, Dwarf Fortress, and DFHack manifests;
- fortress identity, observation epoch, game tick, and pause state;
- a bounded unit summary with source provenance;
- a canonical state anchor and immutable capsule digest;
- one successful `fortress.observe` briefing backed by that live capsule;
- a doctor report and replayable evidence bundle;
- identical Agent Turn Packet semantics between lab replay and the live adapter, except for
  declared provenance and compatibility fields.

The first mutation milestone remains exact pause/resume, with witnessed reads,
prepare/revalidate/commit, an idempotent effect record, authoritative post-state observation,
bridge-journal reconciliation, obligation discharge, prediction-versus-observation comparison,
and deterministic replay.
