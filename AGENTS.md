# AGENTS.md

This repository is designed primarily for autonomous coding and operations agents. These rules are
normative.

## Mission

Build the most efficient, reliable, inspectable, and economically scalable semantic control plane
for Dwarf Fortress. Treat the game as a partially observed evolving civilization, not a
keyboard-and-screen toy. State must be versioned explicitly; authority must be scoped; mutations
must be planned, witnessed, authorized, committed, observed, and proved.

The agent-facing system is one synthetic control loop, not a bag of tools. Every successful or
failed call must help the caller answer: what is true, what changed, what matters, what is possible,
what should happen next, and how certain those answers are. The normative operating model is
`docs/AGENT_OPERATING_MODEL.md`; its machine contract is
`architecture/agent_turn_contract.json`.

## Current evidence posture

The current phase is **Phase 0D-R0 with an implemented but unadmitted Phase-1 announcement read
slice**:

- substantial authenticated protocol-1.0 read-only DFHack source exists;
- canonical live citizen observations and an agent-oriented read-only MCP source exist;
- exact compatibility, monotonic-floor, artifact-receipt, and V2 process-admission machinery exist;
- protocol-1.1 retained-announcement source, transactional publication, single-read bootstrap, and
  an explicitly unadmitted development MCP runtime exist;
- the V2 production runner map contains protocol 1.0 only;
- the checked-in compatibility registry is empty;
- no current live tuple is admitted;
- no live mutation RPC or capability exists;
- the final current head does not yet have a newly checked-in full latest-nightly qualification
  receipt.

Never convert source presence, development execution, old evidence, a unit test, a static-only run,
or a workflow badge into an admission or support claim. `IMPLEMENTATION_STATUS.md`, the current
registry, exact receipts, the production protocol map, and local floor bytes are authoritative.

## Required reading order

1. `IMPLEMENTATION_STATUS.md`
2. `README.md`
3. `docs/AGENT_OPERATING_MODEL.md`
4. `docs/LIVE_DFHACK_READ_PATH.md`
5. `docs/LIVE_ANNOUNCEMENT_STREAM.md`
6. `docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md`
7. `docs/LIVE_COMPATIBILITY_ADMISSION.md`
8. `architecture/live_admission_ticket_v2.json`
9. `docs/LOCAL_QUALIFICATION_AND_RELEASE.md`
10. `docs/SOURCE_BUNDLE.md`
11. `SECURITY.md`
12. `FRANKENSTACK_DEEP_DIVE.md`
13. `COMPREHENSIVE_PLAN_FOR_DWARF_FORTRESS_MCP.md`
14. `ARCHITECTURE.md`
15. `ROADMAP.md`
16. `docs/WORLD_STATE_MVCC.md`
17. `docs/FORTRESS_GRAPH_ALGORITHMS.md`
18. `docs/ATP_STATE_AND_EVIDENCE_PLANE.md`
19. `docs/DEPENDENCY_POLICY.md`
20. `MCP_SURFACE.md`
21. `docs/FASTMCP_INTEGRATION.md`
22. `docs/DOGFOODING_FASTMCP.md`
23. relevant machine registries under `architecture/` and `design/registries/`

Read all of a governing file, not snippets, before modifying a boundary it defines.

## Constitutional engineering rules

- Rust 2024 on the latest nightly toolchain.
- Safe Rust throughout the Rust workspace: `unsafe_code = "forbid"`.
- The dependency universe is closed. Use `asupersync`, owned Franken-suite crates, the owned
  `fastmcp_rust` MCP plane (modern-only MCP 2026-07-28, exact-revision pinned; ADR-013), and only
  explicitly admitted fundamental crates. Do not introduce Tokio, async-std, petgraph, rusqlite,
  reqwest, axum, tonic, prost, any non-owned MCP framework, or another hidden runtime.
- Existing intentional owned dependencies such as `fastmcp_rust` and `eidetic_engine_cli` are
  architectural choices, not accidental exceptions. Preserve them unless a reviewed design change
  proves a superior owned boundary.
- MCP is 2026-07-28, modern-only, forever. Never enable the `legacy-2024-11-05` graph. Transport
  defects are filed upstream against `Dicklesworthstone/fastmcp_rust` and fixed here only by
  recorded exact pin bumps with conformance notes in `docs/DOGFOODING_FASTMCP.md`.
- `asupersync` is the sole asynchronous runtime, structured-concurrency substrate, cancellation
  model, deterministic laboratory, and ATP foundation.
- No detached task, thread, watcher, cache maintainer, bridge request, checkpoint writer, or
  evidence publisher. Every unit of work belongs to a supervised region whose close implies
  quiescence.
- Every function that can block, perform I/O, acquire shared resources, or consume a bounded budget
  carries explicit context and authority.
- No direct memory scraping or C/C++ FFI in the Rust trust domain. DFHack integration is an
  out-of-process, bounded, versioned bridge using supported DFHack facilities.
- No arbitrary shell, Lua, DFHack command, keyboard injection, bridge method, native address,
  protocol selector, or client-selected path through the default MCP surface.
- No GitHub-hosted Action is correctness, performance, compatibility, or release authority.
  Workflow files are portable specifications for local/self-hosted execution, including
  `doodlestein_self_releaser`.

## Evidence and status rules

Use this ladder explicitly:

```text
source present
→ static/Python checked
→ full Rust-qualified
→ native R1-qualified
→ live campaign qualified
→ registry-admitted
→ deployment-floor accepted
→ server-artifact qualified
→ protocol runner admitted
→ runtime admitted
```

Rules:

- Evidence applies only to the exact source, binary, protocol, platform, versions, and inputs it
  names.
- A higher rung never silently transfers to a later commit, rebuilt binary, or another protocol.
- An empty registry means no admitted live tuple.
- A registry entry grants only the exact capabilities and coverage encoded in that entry.
- A local monotonic floor is anti-rollback custody, not compatibility evidence or distributed
  consensus.
- An admission-doctor report is deterministic diagnosis, not authority.
- A server receipt qualifies one executable, not a bridge session or game state.
- A production protocol runner must be explicitly present in the V2 runner map.
- A single-use ticket authorizes one exact process and protocol start and grants no game mutation
  capability.
- A development runtime is never production admission.
- Negative evidence may reject a claim but cannot certify success.
- Update `IMPLEMENTATION_STATUS.md` and `CHANGELOG.md` whenever implementation or evidence posture
  materially changes.

## Bridge generation rules

The current read-only protocols retain exactly two native methods:

```text
Handshake
ReadObservation
```

Protocol 1.1 adds fields inside `ReadObservation`; it does not add a standalone announcement method.
Method names alone do not identify a generation. Protocol version, bridge version, plugin bytes,
source revisions, and platform all participate in exact identity.

Any new bridge method or semantic field generation requires new wire tests, canonicalization,
source digests, native evidence, live evidence, registry admission, floor advancement, server
qualification, production runner review, and process evidence.

Do not modify protocol 1.0 in place to add protocol-1.1 behavior. Do not infer protocol-1.1 admission
from protocol-1.0 evidence.

## Protocol-1.1 development isolation

`dfmcp-live-v1-1-dev-server` is an explicitly unadmitted source and evidence-capture surface. It:

- requires `DFMCP_ALLOW_UNADMITTED_LIVE_V1_1` to equal exactly `1`;
- uses a distinct session-ID namespace;
- preserves the frozen eleven-tool waist;
- exposes read-only behavior only;
- rejects production ticket, protocol, entry, registry, decision, floor, receipt, and launch
  environment state;
- cannot consume a production admission ticket;
- cannot expose production admission provenance.

Its public `dfmcp-mcp` API wrapper must reject `DFMCP_ADMITTED_BRIDGE_PROTOCOL` before entering the
private development server. The production V2 contract must continue to mark protocol 1.1
`implemented_unadmitted_development_only` until the complete evidence chain exists.

## Compatibility and V2 launch rules

Admitted startup must preserve:

```text
exact registry bytes
+ owner-private monotonic floor
+ exact deployment manifest
+ required entry ID
+ exact bridge protocol
+ source-bound server receipt
+ opened executable identity and SHA-256
+ sanitized loader environment
+ protocol-bound single-use ticket
+ reviewed private runner
+ descriptor-only exec
```

The bridge protocol must agree across the deployment manifest, compatibility decision, launch
record, ticket, `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, Rust admission context/provenance, and runner
lookup. Both launch and ticket digests cover it. Unknown protocols, mismatches, legacy V1 tickets,
and protocols absent from the production map fail before server startup.

The current production map admits protocol 1.0 only. Widening it is an evidence-bearing source
change, not an environment or configuration toggle.

Do not add a path-based execution fallback. Re-read registry/floor after artifact verification and
immediately before execution. Re-hash the already-open executable before ticket issuance and before
`exec`. The Rust consumer repeats semantic and executable checks before deleting the ticket and
starting MCP.

The floor must remain:

- at an absolute path;
- under a real exact-mode `0700` directory;
- a regular non-symlink exact-mode `0600` file;
- root/effective-user owned;
- initialized exclusively;
- advanced only through lock + expected-file-digest compare-and-swap + atomic fsynced replacement;
- monotonic in sequence, digest chain, and accepted entry IDs.

The ticket must remain a regular non-symlink exact-mode `0600` file under a real exact-mode `0700`
directory. Owner-only but noncanonical modes such as `0500` or `0400` are not equivalent.

Silent entry removal is not revocation. Design explicit evidence-bearing revocation before
supporting removal.

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
- Derived graph, search, and index generations are immutable. Publish children first, validate
  them, and swap the tiny root last. Partial publication is not a generation.

## Observation-generation rules

- Transport pagination is never canonical state.
- Citizen pagination and announcement continuation are one protocol-1.1 publication transaction.
- A combined capsule is published only after every required page agrees on observation identity,
  summary, source manifest, retained bounds, projection, and continuation.
- Complete retained-announcement suffix and complete fortress history are distinct claims.
- `gap_before_window` is evidence that older history is unknown, not a warning that may be hidden.
- Bootstrap must derive identity and initialize the adapter from one exact combined capsule. Do not
  reintroduce a second underlying bridge read.
- Primed replay preserves citizen offset, announcement cursor, limits, projection policy, source
  manifest, and final snapshot identity.

## Effects, obligations, and evidence

- A transport acknowledgment is not game mutation success, and game mutation success is not goal
  completion.
- Mutations use prepare, revalidate, commit, observe, and prove.
- Every mutating request has a stable idempotency key and effect-journal identity.
- Two-phase effects apply to bridge mutation, observation publication, evidence publication, and
  checkpoint publication where partial completion would be ambiguous.
- Long-running work is a bounded obligation with terminal/failure predicates, game-time deadline,
  cadence, stability requirement, evidence, and cancellation policy.
- Cancellation is request, measurable drain progress, and finalize. It is not forgetting an
  operation.
- Indeterminate effects remain indeterminate until reconciled.
- ATP may move checkpoints, evidence, immutable generations, and state deltas. It may never become
  alternate mutation authority.
- Current live protocols are read-only. Do not smuggle pause, command, or any effect into them.

## Determinism and graph rules

- Same canonical state, request, policy, seed, and budget must produce byte-identical eligible
  results, including order.
- Every graph algorithm with multiple valid outputs declares a canonical tie-break policy.
- Load-bearing graph decisions emit complexity and decision-path witnesses.
- Adaptive policies are clamped, sample-gated, circuit-breakable, and replayable. They may tune
  cost or attention, never weaken safety, authorization, conflict detection, compatibility,
  protocol identity, custody, or proof rules.
- Use immutable structural sharing and zero-copy views only after measuring the copy or boundary
  cost they remove. Preserve epoch, ordering, lifetime, and mutation semantics exactly.
- Hash canonical bytes, not incidental serialization. When a file digest is evidence, hash the
  exact bytes parsed or executed.

## Agent-facing synthesis rules

- Preserve the frozen eleven-tool waist. Improve ergonomics through shared schemas, profiles,
  affordances, typed handles, and progressive disclosure rather than top-level tool growth.
- Every success and error converges on the canonical Agent Turn Packet.
- Every response binds one complete anchor and states continuity explicitly.
- Make active plans, actions, obligations, drains, confirmations, and indeterminate effects visible
  without transcript memory.
- Separate `observed`, `certified_derived`, `inferred`, `predicted`, `assumed`, `stale`, `unknown`,
  `contradicted`, and `indeterminate`. Confidence never substitutes for epistemic class.
- Only observed and eligible certified-derived facts may satisfy mutation preconditions.
- Affordances suggest; they do not authorize.
- Recommendations are structured protocol next steps with evidence, utility, information value,
  cost, risk, reversibility, invalidators, and confirmation requirements.
- Prefer value-of-information inspection.
- Observation profiles are semantic contracts, not vague verbosity settings.
- Empty results prove absence only with complete-domain coverage.
- Material prediction divergence emits a surprise record.
- Memory, imported text, attention, recommendations, and counterfactuals never grant authority.
- A handoff packet must let a fresh agent resume safely without reconstructing the transcript.
- Non-MCP automation is machine-first, structured, bounded, and versioned. Keep stdout and stderr
  separate and prefer deterministic NDJSON where streaming applies.

## Coding guidance

Prefer small pure state-transition functions around an injected effect shell. Use ordered
collections whenever iteration order is observable. Bound every untrusted collection, nesting
level, string, frame, path, file, and output. Preserve unknown wire values where forward
compatibility requires it. Do not use `unwrap`, `expect`, `panic`, `todo`, or `unimplemented` in
production code.

Expected source and contract text must be valid UTF-8, contain no NUL bytes, and remain within the
repository integrity bound. Reject symbolic links, special files, machine-local placeholders,
probe markers, and unstable reads.

When changing a sealed gate, source-digest map, schema, or production protocol map:

1. update the machine contract;
2. update the implementation’s exact expected list;
3. update receipt generation;
4. update independent verification;
5. update focused positive and mutation tests;
6. update aggregate checkers and qualification wiring;
7. update source-digest inventories;
8. update documentation, changelog, and status;
9. verify the representations are semantically identical.

## Qualification and release

Run:

```bash
./scripts/verify.sh
./scripts/qualify_local.sh
```

A full qualification requires a stable exact clean source revision, locked/offline dependency
resolution, rustfmt, warning-denied Clippy, debug/release tests, warning-denied rustdoc, executable
checks, and every repository, bridge, publication, bootstrap, MCP, acceptance, compatibility, floor,
doctor, receipt, launcher, and ticket gate.

`DFMCP_STATIC_ONLY=1`, `DFMCP_ALLOW_MISSING_RUST=1`, or dirty-tree escape hatches create development
evidence only. A dirty run must not produce a receipt whose status is indistinguishable from a clean
release-admissible `passed` receipt.

Protocol 1.1 has a separate source-only qualifier:

```bash
scripts/qualify_live_announcement_source.sh
```

It does not establish native build, A1-A6, baseline R2-R5, registry, floor, artifact, production-map,
or runtime evidence.

Qualify the protocol-1.0 release server separately with:

```bash
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

A release additionally requires exact source bundles, asset contracts, checksums, signatures, SBOMs,
install/rollback evidence, and named platform/compatibility receipts.

## Definition of done

A feature is done only when semantics, capability requirements, effect classification, determinism,
failure/recovery behavior, witness model, Agent Turn behavior, epistemic and coverage semantics,
protocol/compatibility/custody impact, registries, tests, benchmarks, documentation, and acceptance
evidence are explicit.

“The command ran,” “the code compiles,” “the test passed,” and “the source exists” are useful facts.
None is sufficient by itself.
