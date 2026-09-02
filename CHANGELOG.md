# Changelog

All notable changes will be documented in this file. Until the first stable protocol release,
versions describe design, implementation, and compatibility milestones rather than production
readiness.

## [Unreleased]

### Added

- Protocol-bound V2 production admission contract
  (`architecture/live_admission_ticket_v2.json`). The exact bridge protocol now travels from the
  deployment manifest through the compatibility decision, launch record, single-use ticket,
  `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, Rust admission provenance, and final private runner lookup.
  Launch and ticket digests both cover the protocol. The production map currently contains only
  protocol 1.0; protocol 1.1 and unknown protocols fail before live-server startup.
- Protocol-1.1 retained-announcement read generation with a distinct protobuf package, plugin,
  bridge version, source qualification contract, native receipt contract, A1-A6 acceptance
  contract, evidence journal, diagnostic probe, and development MCP runtime.
- Canonical retained-announcement batches with strict report-ID ordering, bounded UTF-8 text,
  retained-window oldest/latest identities, explicit gap evidence, continuation progress, and
  complete-through-latest semantics without a complete-history claim.
- Transactional protocol-1.1 publication across citizen pagination and announcement continuation.
  No combined capsule is published until the citizen roster and configured retained suffix are
  complete and every page reproduces the same observation state.
- Single-publication protocol-1.1 bootstrap. One complete combined capsule now supplies fortress
  identity, source digest, initial world projection, and adapter bootstrap through a primed replay
  layer without a duplicate underlying bridge read.
- Explicit two-dimensional primed replay over citizen pagination and announcement continuation,
  with cursor, projection, limit, source-manifest, and final-snapshot drift checks.
- `dfmcp-live-v1-1-dev-server`, a separately named, exact-opt-in, read-only development runtime
  preserving the frozen eleven-tool waist and a protocol-specific session namespace. It rejects
  production admission state and cannot consume or impersonate a production ticket.
- Protocol-1.1 world projection, briefing, bounded attention, certified-derived report-ID change
  summaries, and query modes for `summary`, `citizens`, `announcements`, and `all`.
- Aggregate protocol-1.1 source checker that executes the core isolation, transactional
  publication, single-read bootstrap, and development-MCP isolation checkers.
- Mutation suites for production-protocol-map widening, inherited admission, method-waist widening,
  history-coverage overclaim, development-guard removal, process-test loss, and mutation
  contamination.
- Canonical clean-commit source-bundle contract, stable no-follow repository-file reader,
  Git-object-derived deterministic tar creation, hostile archive verification without extraction,
  create-only receipts, and atomic sibling-directory publication after complete verification.
- Authenticated read-only DFHack bridge protocol 1.0 with exactly `Handshake` and
  `ReadObservation`, bounded loopback bearer authentication, canonical protobuf validation,
  generation/version/nonce fencing, stable citizen pagination, and no mutation RPC surface.
- Canonical immutable live-observation capsules whose identity is independent of transport
  pagination, plus deterministic fortress/citizen graph projection with fact provenance and
  explicit complete, conditional, and omitted coverage.
- Live read-only adapter and MCP path covering session bootstrap, observation, heartbeat, query,
  wait, explain, doctor, restart/reset classification, and fail-closed mutation-stage tools.
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
- Owner-private, short-lived, single-use admission tickets bound to process ID, bridge protocol,
  exact compatibility entry, registry, decision, monotonic-floor file/content/sequence, server
  receipt, launch digest, executable identity/SHA-256, read-only capabilities, and an empty mutation
  set.
- Rust V2 ticket consumer that validates protocol, rejects legacy V1 tickets, revalidates and hashes
  the current executable, consumes and proves deletion of the ticket, retains admission provenance,
  and only then invokes the exact reviewed private runner; direct `serve-live` invocation fails
  closed.
- Live Agent Turn provenance exposing bridge protocol plus exact compatibility, floor,
  server-receipt, launch, ticket, and executable identities after successful admission.
- Repository source-integrity checks rejecting symbolic links, special files, invalid UTF-8,
  NUL-corrupted or oversized source, machine-local placeholders, recovery debris, and files that
  change while being inspected.
- Adopted the owned `fastmcp_rust` sibling as the MCP presentation plane (ADR-013): modern-only MCP
  2026-07-28, `default-features = false` with `tasks`, pinned to an exact upstream revision for
  dogfooding; upstream defects return through `docs/DOGFOODING_FASTMCP.md`.
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

- Closed a protocol-confusion defect in process admission: an admitted compatibility decision can no
  longer execute an implicitly selected protocol-1.0 server. Production startup now requires exact
  protocol agreement at every representation and an explicit runner in the V2 production map.
- Hardened ticket custody so both Python issuance and Rust consumption require a real exact-mode
  `0700` directory and exact-mode `0600` ticket. Owner-only but noncanonical modes such as `0500`
  fail closed.
- Moved the protocol-1.1 production-environment refusal to the public `dfmcp-mcp` development API
  seam. External callers cannot bypass the guard by invoking the library wrapper directly.
- Replaced the stale monolithic announcement checker coupling with an aggregate that executes all
  specialized layers while the core checker owns shared protocol, source-map, native, model, and
  qualification invariants.
- Protocol-1.1 source identity now includes the bootstrap checker, development MCP contract/server/
  binary/process tests, and the production V2 admission contract whose runner map it must remain
  outside.
- Local verification compiles every specialized announcement checker and runs the protocol-1.1 MCP
  mutation suite explicitly.
- Local qualification receipts now hash the complete protocol-1.1 publication, adapter, bootstrap,
  transaction-test, development-MCP, and production-isolation source graph rather than only the
  initial wire and batch layers.
- Recovered a corrupted checked-in server-receipt verifier from a known-good source generation,
  rebuilt its exact contract validation, and hardened it around stable no-follow opens, duplicate
  JSON-key rejection, exact gate/source maps, opened-inode verification, and repeated SHA-256
  checks.
- Local verification and qualification now include source-bundle, announcement, compatibility-floor,
  admission-doctor, server-qualification-wrapper, launcher, V2 ticket, source-integrity, and binary
  process gates in one exact source-bound order.
- Workspace moved to Rust 2024 and the latest nightly channel.
- GitHub workflow files target controlled self-hosted machines and serve as locally executable
  specifications; GitHub-hosted execution is not release evidence.
- README, architecture, security, agent rules, roadmap, live-admission documentation, announcement
  documentation, changelog, and implementation status distinguish source presence, development
  execution, qualification, compatibility admission, local floor acceptance, artifact
  qualification, production-protocol dispatch, and runtime admission.

### Security

- Bridge-protocol confusion across manifest, launch, ticket, environment, Rust provenance, and
  runtime dispatch now fails closed.
- Legacy V1 tickets, unknown bridge protocols, protocol 1.1 production attempts, and mismatched
  protocol representations fail before live-server startup.
- Protocol-1.1 development execution refuses every production admission marker, including the V2
  protocol environment field.
- Compatibility registry rollback relative to an accepted local generation fails closed.
- Same-size executable byte substitution is checked before ticket issuance, before descriptor
  execution, and again by the Rust process before live MCP startup.
- Dynamic-loader injection variables, permissive or symbolic custody, stale compare-and-swap
  identities, direct live-server bypass, and mixed registry/floor generations fail closed.
- Credentials remain absent from receipts, floor files, decisions, launch records, tickets, Agent
  Turns, and admission-doctor reports.

### Current evidence status

- The checked-in compatibility registry remains `no_admitted_live_tuples` with zero entries.
- Protocol 1.0 and protocol 1.1 source are not currently admitted by that empty registry.
- The V2 production runtime map contains protocol 1.0 only; protocol 1.1 remains explicitly
  unadmitted development source.
- No live mutation RPC or capability exists.
- A fresh full latest-nightly qualification receipt, exact native/live evidence, registry
  promotion, deployment-floor advancement, qualified server artifact, and admitted launch evidence
  are still required for the final current head.

## [0.0.1] - 2026-08-29

### Added

- Initial public design corpus and executable contract scaffold.
