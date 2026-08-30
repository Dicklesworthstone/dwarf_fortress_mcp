# Changelog

All notable changes will be documented in this file. Until the first stable protocol release,
versions describe design and compatibility milestones rather than production readiness.

## [Unreleased]

### Added
- Adopted the owned `fastmcp_rust` sibling as the MCP presentation plane (ADR-013): modern-only
  MCP 2026-07-28, `default-features = false` with `tasks`, pinned to an exact upstream revision
  for dogfooding; upstream defects return via gh issues (`docs/DOGFOODING_FASTMCP.md`).
- New `dfmcp-mcp` crate exposing the frozen 11-tool `fortress.*` waist over stdio through
  `fastmcp-rust`, backed by the deterministic laboratory adapter
  (`cargo run -p dwarf-fortress-mcp -- serve`).
- Machine-enforced transport policy: `[mcp_transport]` profile and lock-exception screening in
  `architecture/dependency_allowlist.toml`, checked by `scripts/check_dependency_policy.py` and
  `scripts/validate_repo.py`.

- Deep source-level audit of asupersync, FrankenSQLite, FrankenFS, FrankenSearch,
  FrankenMarkdown, FrankenGraphDB, FrankenNetworkX, and Doodlestein Self-Releaser.
- Three-plane architecture separating authoritative world/evidence state, derived cognition, and
  narrowly fenced DFHack effects.
- One observation-capsule version universe for history, projections, subscriptions, branches,
  checkpoints, evidence, and replicas.
- Multi-version world-state specification with positive, negative, range, aggregate, spatial, and
  epoch witnesses; hierarchical conflict refinement; deterministic semantic rebase; and
  proof-carrying merge.
- Canonical fortress graph algorithm plan with explicit tie-break policies, complexity witnesses,
  tiered projections, incremental maintenance, and capability non-interference.
- ATP state/evidence plane covering content-addressed manifests, RaptorQ repair, path racing,
  anti-rollback rules, proof of retrievability, and an explicit prohibition on mutation authority.
- Closed dependency policy for latest-nightly pure Rust with asupersync/Franken-suite foundations.
- Root-last immutable publication primitives and machine-readable architecture registries.
- Same-binary performance experimentation doctrine and evidence-gated optimization lifecycle.
- Local-only qualification and DSR release specification with machine-readable receipts.

### Changed

- Workspace moved to Rust 2024 and the latest nightly channel.
- GitHub workflow files now target controlled self-hosted machines and serve as locally executable
  specifications; GitHub-hosted execution is not release evidence.
- Roadmap, architecture, integration guide, contribution rules, and implementation status were
  rewritten around the deeper Franken substrate.

## [0.0.1] - 2026-08-29

### Added

- Initial public design corpus and executable contract scaffold.
