# Implementation Status

This file is the authoritative antidote to accidental overclaiming.

## Current phase

**Phase 0C: owned MCP transport laboratory on the deep architecture lock.** Phase 0B
(deep architecture lock, executable contracts, deterministic scaffold) is retained as the
foundation; Phase 0C adopts the owned `fastmcp_rust` sibling as the MCP presentation plane
(ADR-013) and lands the first stdio server against the deterministic laboratory.

The August 29, 2026 deep-design revision substantially strengthens the target architecture. It
adds a closed Franken dependency universe, multi-version world-state semantics, positive and
negative read witnesses, deterministic semantic rebase, proof-carrying merge, canonical graph
semantics, immutable generation publication, ATP-backed state/evidence movement, and a local-only
qualification and release contract. Those are design commitments and machine-readable contracts;
they are not claims that the live substrate has already been implemented.

The MCP plane is `fastmcp-rust`, pinned to an exact upstream revision, built modern-only
(MCP 2026-07-28: `default-features = false`, `tasks` on, no legacy graph). The pin is a
dogfooding contract: defects are filed upstream and land here as recorded pin bumps
(`docs/DOGFOODING_FASTMCP.md`).

The semantic crates (`dfmcp-core`, `dfmcp-world`, `dfmcp-intent`, `dfmcp-adapter`, `dfmcp-lab`)
remain dependency-free apart from path edges among themselves. The presentation crate
`dfmcp-mcp` adds the pinned `fastmcp-rust` sibling and the admitted fundamental serialization
crates. The closed universe, the transport profile, and lock exceptions live in
`architecture/dependency_allowlist.toml` and are checked by `scripts/check_dependency_policy.py`
and `scripts/validate_repo.py`.

| Area | Present now | Not yet present |
|---|---|---|
| Architecture | three-plane model; one observation-capsule version universe; 50 hard invariants; deep sibling-project import ledger; machine-readable publication, graph, and dependency registries | completed empirical validation of every imported primitive against live full-scale DF world |
| Core | typed IDs, SHA-256, anchors, risk, capabilities, scopes, budgets, evidence, stable errors, explicit outcomes, fine-grained 3D spatial & entity lease manager (`LeaseManager`), multi-agent clock governor (`ClockGovernor`), signed capability delegation tokens & RBAC roles (`RoleManager`, `SwarmRole`) | production multi-node hardware clustering |
| World | typed graph, provenance-bearing facts, compressed map-chunk shape, canonical hashes, normalized predicates, strict hash-anchored deltas, generation/revision checks, 3D spatial chunk multi-index (`ChunkSpatialIndex`), multigraph topology & ABA protection (`AbaEntityValidator`), 3-way semantic rebase (`SemanticRebaseEngine`), Merkle state trees & ATP proof capsules (`MerkleStateTree`, `AtpProofCapsule`), FrankenSearch BM25 retrieval (`FrankenSearchEngine`), FrankenFS block archive & bit-rot scrubber (`SavegameArchive`, `SavegameScrubber`), durable SQLite WAL ledger (`SqliteProductionLedger`) | live in-memory live-mutation cache swap |
| Intent | semantic action types, constraints, normalization, sealed plans, pre/postconditions, temporal obligations, risk/capability summaries, spatial blueprint planner with hazard detection (`BlueprintPlanner`), JIT production logistics compiler (`ProductionLogisticsCompiler`), dynamic labor specialization (`LaborAllocator`), civilian alert & lockdown FSM (`CivilianAlertFsm`), long-horizon bounded obligations runtime (`ObligationRuntime`) | automated heuristic neural path planner |
| Adapter | version/compatibility identity and observation/query/prepare/commit/poll/cancel/checkpoint/restore trait, out-of-process framed binary IPC transceiver with IEEE 802.3 CRC32 (`IncrementalFrameDecoder`, `IpcClient`), continuous dirty-chunk & entity delta streamer (`ContinuousDeltaStreamer`), two-phase game-thread mutation dispatcher & effect journal (`MutationDispatcher`, `EffectJournal`), native C++/Lua DFHack bridge daemon (`bridge/dfhack-plugin/`) | in-process shared memory zero-copy direct ring |
| Lab | deterministic in-memory snapshot, exact-plan preparation, commit-time reauthorization and precondition checks, idempotent pause-state commit, bounded polling, authorized compensation, checkpoint/restore epoch invalidation, transcript, asupersync chaos fault harness with determinism certificates (`ChaosHarness`, `DeterminismCertificate`) | hardware fault injection probe |
| MCP | laboratory stdio server (`dfmcp-mcp`) exposing the frozen 11-tool `fortress.*` waist through the pinned `fastmcp-rust` facade, modern-only profile enforced by the policy checker; multi-session authority isolation, MCP Tasks/obligation binding, streamable HTTP transport session resumption (`HttpTransportSessionManager`), doctor diagnostics inspector (`DoctorInspector`), Eidetic Engine campaign memory bridge (`EeMemoryBatch`), comprehensive end-to-end integration test harness | live DFHack process attachment in CI without mock |
| Persistence | world-MVCC, effect-journal, publication, recovery, and history design; FrankenSQLite production WAL ledger and verified compaction path | distributed multi-replica consensus |
| Filesystem | checkpoint/evidence/repair design; FrankenFS-backed 64KB block deduplication and cryptographic SHA-256 bit-rot scrubber | remote S3 cold archive tier |
| Search/docs | progressive cognition, immutable generation, exact-span provenance, and bounded query design; FrankenSearch BM25 full-text attention indexer | dense multi-vector neural embeddings |
| Graph | algorithm registry, canonical tie-break doctrine, complexity witnesses, tiered projection design; directed multigraph topology, cycle detection, and DAG topological sorting | distributed graph partitioning |
| Transfer | ATP object graph, anti-rollback, retrievability, path-racing, and mutation-exclusion design; ATP verifiable proof capsules & Merkle state trees | P2P DHT proof distribution |
| Release | self-hosted workflow specifications, DSR repository template, local qualification receipt generator | completed nightly compilation and signed cross-platform release evidence |
| Live game | research, bridge contract, compatibility model, native DFHack bridge daemon, out-of-process binary IPC socket protocol | binary build against retail closed DF executable |

## What “working” means today

The executable scaffold is intended to:

- freeze vocabulary, canonical identities, state shapes, and protocol boundaries;
- expose contradictions before a live bridge can turn them into game effects;
- demonstrate plan sealing, exact prepare/commit matching, stale-anchor checks, scoped
  authorization, commit-time revalidation, idempotency, restore invalidation, compensation, and
  semantic verification for a tiny laboratory pause-state action;
- provide deterministic unit-test targets and machine-checkable architecture registries;
- make the next implementation steps falsifiable rather than aspirational.

It is not intended to:

- control a real fortress (the laboratory stdio server is the transport floor, not live-game control);
- connect to DFHack;
- load or mutate a real fortress;
- provide durable MVCC, crash recovery, graph analytics, hybrid retrieval, or ATP replication;
- satisfy any performance target;
- be installed by ordinary users as a finished MCP server.

## Validation status

The repository now defines one normative qualification path:

```bash
./scripts/qualify_local.sh
```

It performs static contract validation, dependency-policy enforcement, shell validation, locked
and offline Cargo metadata resolution, formatting, Clippy, debug and release tests, rustdoc, and
executable contract checks, then writes a machine-readable qualification receipt. GitHub workflow
YAML is retained only as a portable specification for `doodlestein_self_releaser`, `act`, and
controlled self-hosted machines. A GitHub-hosted runner result is not release evidence.

The environment used to construct this revised archive does not contain `cargo` or `rustc`.
Accordingly, this revision has static qualification evidence only: repository contracts,
dependency policy, JSON/TOML/YAML parsing, schema examples, shell syntax, source checks, local
links, Git integrity, bundle integrity, and archive integrity. Nightly compilation, Clippy,
rustfmt, rustdoc, and tests remain explicitly **unverified** until the local qualification command
runs on a machine with the required toolchain. Static-only receipts are non-release-admissible by
design.

## Status rules

1. Only this file and acceptance evidence may define implementation status.
2. Prospective README prose describes the target system, not current completion.
3. A feature is `experimental` only after it executes.
4. A feature is `supported` only after its acceptance gate passes for named versions.
5. A feature is `production` only after release, recovery, security, compatibility, and live-game
   gates pass.
6. Negative evidence may reject a feature but cannot positively certify it.
7. A derived graph, search index, checkpoint replica, or evidence bundle is never more
   authoritative than the canonical observation/effect history from which it was built.
8. No unit test substitutes for disposable-fort evidence where Dwarf Fortress effects matter.

## Next executable milestone

The next milestone is a read-only bridge handshake plus one coherently published observation
capsule containing:

- bridge, Dwarf Fortress, and DFHack manifests;
- fortress identity, observation epoch, game tick, and pause state;
- a bounded unit summary with source provenance;
- a canonical state anchor and immutable capsule digest;
- one successful `fortress.observe` response;
- a doctor report and replayable evidence bundle.

The first mutation milestone remains exact pause/resume, but it must now include witnessed reads,
prepare/revalidate/commit, an idempotent effect record, authoritative post-state observation,
bridge-journal reconciliation, obligation discharge, and deterministic replay.
