# Architecture Decision Log

This file contains accepted phase-zero decisions. Future changes append ADRs; they do not rewrite
history.

## ADR-000 — Build a semantic control plane, not a command wrapper

**Status:** accepted
**Decision:** canonical state, intent compilation, transactional mutation, obligations, and
evidence are core. DFHack commands are bridge implementation details.
**Why:** broad imperative tools do not solve state continuity, retry, long-running completion, or
multi-agent safety.
**Consequence:** more up-front architecture; smaller and more stable MCP surface.

## ADR-001 — Safe Rust trust domain with out-of-process native bridge

**Status:** accepted
**Decision:** `unsafe_code = "forbid"` in Rust; no direct C/C++ FFI.
**Why:** version/ABI and memory-safety risk belong behind a bounded protocol.
**Consequence:** serialization and process-boundary cost; better isolation/replay.

## ADR-002 — Canonical world state distinct from raw and presentation state

**Status:** accepted
**Decision:** normalize into versioned typed graph/chunks/events with provenance.
**Why:** raw DFHack layout and MCP output are both unstable for different reasons.

## ADR-003 — SHA-256 canonical anchors in v1 scaffold

**Status:** accepted for phase zero
**Decision:** use SHA-256 over explicit framed ordered bytes.
**Why:** deterministic, implemented with the standard library in the scaffold, and auditable.
**Follow-up:** production bounded schema encoding and algorithm tagging.

## ADR-004 — Cursor plus hash continuity

**Status:** accepted
**Decision:** deltas require exact base cursor and state hash; restore resets epoch.
**Why:** sequence alone cannot detect divergent state.

## ADR-005 — Small MCP narrow waist

**Status:** accepted
**Decision:** 11 top-level tools; action/query growth occurs in schemas/registries.
**Why:** discoverability and semantic consistency.

## ADR-006 — Prepare/revalidate/commit/observe/prove

**Status:** accepted
**Decision:** all mutations follow the same protocol.
**Why:** state changes between planning and effect; receipt is not completion.

## ADR-007 — Temporal work is a bounded obligation

**Status:** accepted
**Decision:** temporal actions require terminal/failure predicates, game-tick deadline, stability,
and ownership.
**Why:** queued work can block, fail, or never complete.

## ADR-008 — Indeterminate is a first-class state

**Status:** accepted
**Decision:** uncertain possible effects are not converted to failure.
**Why:** blind retry can duplicate destructive effects.

## ADR-009 — Compensation is a new action, not rollback

**Status:** accepted
**Decision:** compensation is separately authorized and verified.
**Why:** game state and time are not transactionally reversible.

## ADR-010 — Capabilities and leases are explicit data

**Status:** accepted
**Decision:** scopes, risk, expiry, uses, budgets, and fencing travel with operations.
**Why:** no ambient authority and multi-agent race protection.

## ADR-011 — Reference traits before Franken adapters

**Status:** accepted
**Decision:** semantic traits and deterministic reference implementations precede sibling
integration.
**Why:** prevent implementation coupling and enable differential tests.

## ADR-012 — Latest nightly, closed Franken dependency universe, local qualification

**Status:** accepted
**Decision:** target Rust 2024 on the latest nightly toolchain; permit only `asupersync`, owned
Franken-suite crates, and explicitly admitted fundamental crates; treat local DSR qualification as
the release authority.
**Why:** the entire runtime, storage, graph, transfer, and release behavior must remain inspectable,
deterministically testable, and under project control without relying on opaque executors or
GitHub-hosted capacity.
**Consequence:** more owned implementation work and explicit sibling revision management; a much
smaller hidden behavior surface and reproducible local evidence.

## ADR template

```markdown
## ADR-NNN — Title
Status:
Date:
Decision:
Context:
Alternatives:
Counterexamples:
Affected invariants/registries:
Compatibility/migration:
Security:
Determinism/replay:
Performance/token economics:
Testing/evidence:
Rollback/reversal:
```
