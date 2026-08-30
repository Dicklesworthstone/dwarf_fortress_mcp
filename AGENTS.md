# AGENTS.md

This repository is designed primarily for autonomous coding and operations agents. These rules
are normative.

## Mission

Build the most efficient, reliable, inspectable, and economically scalable semantic control
plane for Dwarf Fortress. Treat the game as a partially observed evolving civilization, not a
keyboard-and-screen toy. State must be versioned explicitly; authority must be scoped; mutations
must be planned, witnessed, authorized, committed, observed, and proved.

## Required reading order

1. `IMPLEMENTATION_STATUS.md`
2. `README.md`
3. `FRANKENSTACK_DEEP_DIVE.md`
4. `COMPREHENSIVE_PLAN_FOR_DWARF_FORTRESS_MCP.md`
5. `ARCHITECTURE.md`
6. `docs/WORLD_STATE_MVCC.md`
7. `docs/FORTRESS_GRAPH_ALGORITHMS.md`
8. `docs/ATP_STATE_AND_EVIDENCE_PLANE.md`
9. `docs/DEPENDENCY_POLICY.md`
10. `MCP_SURFACE.md`
11. `docs/FASTMCP_INTEGRATION.md`
12. `docs/DOGFOODING_FASTMCP.md`
13. relevant machine registries under `architecture/` and `design/registries/`

## Constitutional engineering rules

- Rust 2024 on the latest nightly toolchain.
- Safe Rust throughout the Rust workspace: `unsafe_code = "forbid"`.
- The dependency universe is closed. Use `asupersync`, owned Franken-suite crates, the owned
  `fastmcp_rust` MCP plane (modern-only MCP 2026-07-28, exact-revision pinned; ADR-013), and only
  the explicitly admitted fundamental crates. Do not introduce Tokio, async-std, petgraph,
  rusqlite, reqwest, axum, tonic, prost, any non-owned MCP framework, or another hidden runtime.
  The transport adds no authority: no fastmcp type crosses the intent, world, or adapter seams.
- MCP is 2026-07-28, modern-only, forever. Never enable the `legacy-2024-11-05` graph. Transport
  defects are filed upstream against `Dicklesworthstone/fastmcp_rust` and fixed here only by
  recorded pin bumps with conformance notes (`docs/DOGFOODING_FASTMCP.md`); masking them
  dfmcp-side is a bug in this repository.
- `asupersync` is the sole asynchronous runtime, structured-concurrency substrate, cancellation
  model, deterministic laboratory, and ATP foundation.
- No detached task, thread, watcher, cache maintainer, bridge request, checkpoint writer, or
  evidence publisher. Every unit of work belongs to a supervised region whose close implies
  quiescence.
- Every function that can block, perform I/O, acquire shared resources, or consume a bounded
  budget carries explicit context/authority.
- No direct memory scraping or C/C++ FFI in the Rust trust domain. The DFHack integration is an
  out-of-process, bounded, versioned bridge using supported DFHack facilities.
- No arbitrary shell, Lua, DFHack command, or path execution through the default MCP surface.

## State and transaction rules

- The canonical world is multi-version. Never replace it with one mutable “current snapshot.”
- Every plan records positive reads, relation/range reads, aggregate reads, negative reads,
  adapter/schema/topology epochs, and intended writes at the coarsest sound granularity.
- Witness refinement may reduce false conflicts but may never introduce a false negative. Budget
  exhaustion means conservative replan, not guessed safety.
- Leases fence ownership; witnesses validate knowledge. A mutating commit requires both when both
  apply.
- Observation is cursor-anchored. Never silently bridge a gap or cross a restore epoch.
- Entity generation prevents ABA reuse; revision orders updates within a generation.
- Semantic rebase attempts intent replay first, stable-key structural merge second, and explicit
  rejection third. Raw-byte merge of structured state is forbidden.
- A successful merge emits a deterministic certificate covering normal form, tie-break policy,
  read/write witnesses, and resulting state digest.
- Derived graph/search/index generations are immutable. Publish artifacts first, validate them,
  and swap the tiny root last. Partial publication is not a generation.

## Effects, obligations, and evidence

- A transport acknowledgment is not game mutation success, and game mutation success is not goal
  completion.
- Mutations use prepare, revalidate, commit, observe, and prove.
- Every mutating request has a stable idempotency key and an effect-journal identity.
- Two-phase effects apply to bridge mutation, observation publication, evidence publication, and
  checkpoint publication where partial completion would be ambiguous.
- Long-running work is a bounded obligation with terminal and failure predicates, game-time
  deadline, cadence, stability requirement, evidence, and cancellation policy.
- Cancellation is request, measurable drain progress, and finalize. It is not forgetting the
  operation.
- Indeterminate effects remain indeterminate until reconciled.
- Negative evidence can reject a claim but cannot certify success.
- ATP may move checkpoints, evidence, immutable generations, and state deltas. It may never become
  an alternate mutation authority.

## Determinism and graph rules

- Same canonical state, request, policy, seed, and budget must produce byte-identical eligible
  results, including order.
- Every graph algorithm with multiple valid outputs declares a canonical tie-break policy.
- Load-bearing graph decisions emit complexity and decision-path witnesses.
- Adaptive policies are clamped, sample-gated, circuit-breakable, and replayable. They may tune
  cost or attention, never weaken safety, authorization, conflict detection, or proof rules.
- Use immutable structural sharing and zero-copy views only after measuring the copy or boundary
  cost they remove. Preserve epoch, ordering, lifetime, and mutation semantics exactly.

## Coding guidance

Prefer small pure state-transition functions around an injected effect shell. Use ordered
collections wherever iteration order is observable. Hash canonical bytes, not incidental
serialization. Bound every untrusted collection, nesting level, string, frame, and output.
Preserve unknown wire values. Do not use `unwrap`, `expect`, `panic`, `todo`, or `unimplemented`
in production code.

## Qualification and release

Run:

```bash
./scripts/qualify_local.sh
```

GitHub workflow files are portable build specifications for local `doodlestein_self_releaser`,
`act`, and controlled self-hosted machines. Do not rely on GitHub-hosted Actions for correctness,
performance, compatibility, or release evidence. A release requires a clean source revision,
locked/offline dependency resolution, exact asset contracts, checksums, signatures, SBOMs, and
machine-readable qualification receipts.

## Definition of done

A feature is done only when its semantics, capability requirements, effect classification,
determinism classification, failure and recovery behavior, witness model, registries, tests,
benchmarks, documentation, compatibility matrix, and acceptance evidence are explicit. “The
command ran” is never sufficient.

## Current phase

The checked-in Rust workspace remains an executable contract scaffold. It is not yet a production
MCP server and it does not yet include a live DFHack adapter. Read `IMPLEMENTATION_STATUS.md`
before making any implementation claim.
