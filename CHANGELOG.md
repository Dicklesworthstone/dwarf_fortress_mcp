# Changelog

All notable changes will be documented in this file. Until the first stable protocol release,
versions describe design, implementation, and compatibility milestones rather than production
readiness.

## [Unreleased]

### Added

- Authenticated read-only DFHack bridge protocol V1 with exactly `Handshake` and
  `ReadObservation`, bounded loopback bearer authentication, canonical protobuf validation,
  generation/version/nonce fencing, stable citizen pagination, and no mutation RPC surface.
- Canonical immutable live-observation capsules whose identity is independent of transport
  pagination, plus deterministic fortress/citizen graph projection with fact provenance and
  explicit complete, conditional, and omitted coverage.
- Live read-only adapter and MCP path covering session bootstrap, observation, heartbeat,
  query, wait, explain, doctor, restart/reset classification, and fail-closed mutation-stage tools.
- R1 native-plugin qualification and R2-R5 disposable-fort acceptance machinery, including
  source/binary receipts, secret scanning, append-only evidence journal, capture guidance,
  deterministic exact-tuple promotion, and registry-generation-bound resolution.
- Owner-private monotonic compatibility floor with exact `0700`/`0600` custody, no-follow reads,
  exclusive initialization, expected-file-digest compare-and-swap, atomic fsynced advancement,
  monotonic sequence and digest chain, and preservation of all previously accepted entry IDs.
- Deterministic authority-free live-admission doctor with fixed registry, floor, exact-tuple, and
  optional server-artifact stages; canonical reports expose `compatibility_ready` or
  `artifact_preflight_ready` without reading a bridge secret or executing a process.
- Source-bound release-server receipt contract sealing the exact clean commit, complete local gate
  order, admission machinery, source digests, platform, executable checks, size, and SHA-256.
- Descriptor-bound admitted launcher that repeatedly verifies registry/floor generation and
  executable bytes, rejects dynamic-loader override variables, emits a secret-free launch record,
  and refuses path-based execution fallback.
- Owner-private, short-lived, single-use admission tickets bound to process ID, exact compatibility
  entry, registry, decision, monotonic-floor file/content/sequence, server receipt, launch digest,
  executable identity/SHA-256, read-only capabilities, and an empty mutation set.
- Rust ticket consumer that revalidates and hashes the current executable, consumes and proves
  deletion of the ticket, retains admission provenance, and only then starts the private live MCP
  server; direct `serve-live` invocation fails closed.
- Live Agent Turn provenance exposing exact compatibility, floor, server-receipt, launch, ticket,
  and executable identities after successful admission.
- Repository source-integrity checks rejecting invalid UTF-8, NUL-corrupted, oversized, placeholder,
  and recovery-debris source or contract files.
- Adopted the owned `fastmcp_rust` sibling as the MCP presentation plane (ADR-013): modern-only
  MCP 2026-07-28, `default-features = false` with `tasks`, pinned to an exact upstream revision for
  dogfooding; upstream defects return through the process in `docs/DOGFOODING_FASTMCP.md`.
- `dfmcp-mcp` crate exposing the frozen eleven-tool `fortress.*` waist over stdio through
  `fastmcp_rust`, backed by the deterministic laboratory adapter and the admitted read-only live
  adapter.
- Machine-enforced transport policy and closed dependency-universe screening.
- Adopted the owned `eidetic_engine_cli` sibling as an advisory, evidence-linked agent campaign
  memory layer with no canonical-state or authority path.
- Deep source-level audit of asupersync, FrankenSQLite, FrankenFS, FrankenSearch,
  FrankenMarkdown, FrankenGraphDB, FrankenNetworkX, and Doodlestein Self-Releaser.
- Three-plane architecture separating authoritative world/evidence state, derived cognition, and
  narrowly fenced DFHack effects, plus an explicit deployment-admission boundary.
- One observation-capsule version universe for history, projections, subscriptions, branches,
  checkpoints, evidence, and replicas.
- Multi-version world-state specification with positive, negative, range, aggregate, spatial, and
  epoch witnesses; hierarchical conflict refinement; deterministic semantic rebase; and
  proof-carrying merge.
- Canonical fortress graph algorithm plan with explicit tie-break policies, complexity witnesses,
  tiered projections, incremental maintenance, and capability non-interference.
- ATP state/evidence plane covering content-addressed manifests, RaptorQ repair, path racing,
  anti-rollback rules, proof of retrievability, and an explicit prohibition on mutation authority.
- Root-last immutable publication primitives, same-binary performance experimentation doctrine,
  local-only qualification, and DSR release specifications with machine-readable receipts.

### Changed

- Recovered a corrupted checked-in server-receipt verifier from a known-good source generation,
  rebuilt its exact contract validation, and hardened it around stable no-follow opens, duplicate
  JSON-key rejection, exact gate/source maps, opened-inode verification, and repeated SHA-256
  checks.
- Local verification and qualification now include compatibility-floor, admission-doctor,
  server-qualification-wrapper, launcher, admission-ticket, source-integrity, and binary process
  gates in one exact source-bound order.
- Workspace moved to Rust 2024 and the latest nightly channel.
- GitHub workflow files now target controlled self-hosted machines and serve as locally executable
  specifications; GitHub-hosted execution is not release evidence.
- README, architecture, security, agent rules, roadmap, live-admission documentation, and
  implementation status now distinguish source presence, qualification, compatibility admission,
  local floor acceptance, artifact qualification, and runtime admission.

### Security

- Compatibility registry rollback relative to an accepted local generation now fails closed.
- Same-size executable byte substitution is checked before ticket issuance, before descriptor
  execution, and again by the Rust process before live MCP startup.
- Dynamic-loader injection variables, permissive or symbolic custody, stale compare-and-swap
  identities, direct live-server bypass, and mixed registry/floor generations fail closed.
- Credentials remain absent from receipts, floor files, decisions, launch records, tickets,
  Agent Turns, and admission-doctor reports.

### Current evidence status

- The checked-in compatibility registry remains `no_admitted_live_tuples` with zero entries.
- No live mutation RPC or capability exists.
- The current source includes the complete read-only and admission machinery, but a fresh full
  latest-nightly qualification receipt, exact R1-R5 tuple evidence, registry promotion, deployment
  floor advancement, and admitted launch evidence are still required for the final current head.

## [0.0.1] - 2026-08-29

### Added

- Initial public design corpus and executable contract scaffold.
