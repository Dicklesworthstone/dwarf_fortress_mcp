# Implementation Status

This file is the authoritative antidote to accidental overclaiming.

## Current phase

**Phase 0C: modern MCP transport and semantic-contract laboratory.** The repository contains a
substantial executable scaffold, but it is not a production MCP server and does not connect to
DFHack. The separately installed Dwarf Fortress + DFHack stack is a reference environment for
future integration work, not evidence of live control.

The target architecture specifies multi-version world state, read witnesses, semantic rebase,
proof-carrying merge, canonical graph behavior, immutable publication, ATP-backed evidence
movement, and structured concurrency. Most of those are design commitments or in-memory
laboratories. Their presence as types and tests must not be described as a durable or live
implementation.

The presentation crate uses exact-revision-pinned `fastmcp_rust`, with default features disabled
and only the modern MCP 2026-07-28 Tasks feature admitted. Pin changes require the record in
`docs/DOGFOODING_FASTMCP.md`.

| Area | Present now | Not yet present |
|---|---|---|
| Architecture | three-plane target model, invariants, design documents, and machine-readable dependency/publication/graph registries | empirical validation against a live, full-scale fortress |
| Core | typed IDs, SHA-256, anchors, risks, capabilities, scopes, budgets, evidence, stable errors; experimental in-process lease, clock, and role/delegation models | authenticated principals, durable/distributed fencing, production authorization service |
| World | typed graph, provenance-bearing facts, canonical snapshots, normalized predicates, strict deltas, generation/revision checks; experimental in-memory spatial, topology, rebase, Merkle, ATP, search, archive, checkpoint, and table-ledger models | admitted Franken-suite integrations, durable MVCC/WAL, crash recovery, production indexes |
| Intent | semantic action vocabulary, normalization, sealed plans, pre/postconditions, bounded obligations; experimental fail-closed blueprint, logistics, labor, alert, and obligation laboratories | qualified execution semantics beyond the laboratory pause action; terrain predicates that prove blueprint completion |
| Adapter | transport-independent `GameAdapter` trait; experimental framing/transceiver and delta laboratories; fail-closed DFHack transport-liveness probe | authenticated handshake, canonical payload codecs, live DFHack observation or mutation adapter |
| Bridge | protobuf design contract, experimental framing header, unconnected Lua research helpers, and a C++ placeholder whose initialization deliberately fails | genuine DFHack plugin registration/linkage, supported API calls, authorization, interoperability, installation target |
| Lab | deterministic in-memory snapshot and pause-only adapter with prepare/commit checks, idempotency, polling, compensation, process-local checkpoint/restore, and transcript | equivalence with live DFHack behavior, durable effects, full action support |
| MCP | modern-only stdio laboratory exposing the 11-tool surface through pinned `fastmcp_rust`; process-local sessions and authority tests | live adapter selection, authenticated deployment, persistence, qualified HTTP/Tasks integration |
| Persistence | effect/publication/checkpoint designs plus in-memory table and archive prototypes | SQLite/FrankenSQLite, FrankenFS, WAL, process-crash recovery, durable compaction |
| Search/graph | deterministic fixed-point lexical and basic graph laboratories plus target registries | FrankenSearch/graph-engine integration and production-scale qualification |
| Transfer | local proof-capsule and Merkle integrity models | ATP transport, peer exchange, anti-rollback/retrievability qualification |
| Release | self-hosted workflow specifications and local qualification receipt generator | signed cross-platform release evidence and installable release artifacts |
| Live game | a separately installed DF Classic + DFHack reference stack for research | any Rust-to-DFHack connection, authoritative live observation, or live mutation |

## What “working” means today

The executable scaffold can:

- freeze vocabulary, canonical identities, state shapes, and protocol boundaries;
- demonstrate plan sealing, exact prepare/commit matching, stale-anchor checks, scoped
  authorization, idempotency, restore invalidation, compensation, and postcondition verification
  for a small in-memory pause-state action;
- exercise deterministic integrity and planning models under unit and integration tests;
- expose the MCP tool shape over stdio using process-local laboratory sessions.

It cannot:

- connect to DFHack, load a fortress, observe authoritative game state, or mutate a real game;
- provide durable MVCC, a SQLite WAL, process-crash recovery, Franken-suite storage/search, or
  ATP replication;
- establish performance, security, compatibility, or release evidence from unit tests;
- be installed as a finished end-user MCP server.

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
unit test for an in-memory prototype is not live-game, durability, performance, or compatibility
evidence. `DFMCP_ALLOW_DIRTY=1` permits development qualification of a dirty tree, but that result
is not release evidence. Current `fastmcp_rust` conformance limitations are recorded in
`docs/DOGFOODING_FASTMCP.md`; do not infer a full modern lifecycle pass from negative fixtures.

## Status rules

1. Only this file and acceptance evidence may define implementation status.
2. Prospective README prose describes the target system, not current completion.
3. A feature is `experimental` only after it executes.
4. A feature is `supported` only after its acceptance gate passes for named versions.
5. A feature is `production` only after release, recovery, security, compatibility, and live-game
   gates pass.
6. Negative evidence may reject a feature but cannot certify success.
7. A derived index, checkpoint replica, or evidence bundle is never more authoritative than the
   canonical observation/effect history from which it was built.
8. No unit test substitutes for disposable-fort evidence where Dwarf Fortress effects matter.

## Next executable milestone

The next milestone is a read-only bridge handshake plus one coherently published observation
capsule containing:

- authenticated bridge, Dwarf Fortress, and DFHack manifests;
- fortress identity, observation epoch, game tick, and pause state;
- a bounded unit summary with source provenance;
- a canonical state anchor and immutable capsule digest;
- one successful `fortress.observe` response backed by that live capsule;
- a doctor report and replayable evidence bundle.

The first mutation milestone remains exact pause/resume, with witnessed reads,
prepare/revalidate/commit, an idempotent effect record, authoritative post-state observation,
bridge-journal reconciliation, obligation discharge, and deterministic replay.
