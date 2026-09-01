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

The current phase is **Phase 0D-R0**:

- substantial authenticated read-only DFHack source exists;
- canonical live observations and an agent-oriented read-only MCP source exist;
- exact compatibility, monotonic-floor, artifact-receipt, and single-use launch machinery exist;
- the checked-in compatibility registry is empty;
- no current live tuple is admitted;
- no live mutation RPC or capability exists;
- the final current head does not yet have a fresh full latest-nightly qualification receipt.

Never convert source presence, old evidence, a unit test, a static-only run, or a workflow badge into
an admission or support claim. `IMPLEMENTATION_STATUS.md`, the current registry, exact receipts, and
local floor bytes are authoritative.

## Required reading order

1. `IMPLEMENTATION_STATUS.md`
2. `README.md`
3. `docs/AGENT_OPERATING_MODEL.md`
4. `docs/LIVE_DFHACK_READ_PATH.md`
5. `docs/LIVE_COMPATIBILITY_ADMISSION.md`
6. `docs/LOCAL_QUALIFICATION_AND_RELEASE.md`
7. `SECURITY.md`
8. `FRANKENSTACK_DEEP_DIVE.md`
9. `COMPREHENSIVE_PLAN_FOR_DWARF_FORTRESS_MCP.md`
10. `ARCHITECTURE.md`
11. `ROADMAP.md`
12. `docs/WORLD_STATE_MVCC.md`
13. `docs/FORTRESS_GRAPH_ALGORITHMS.md`
14. `docs/ATP_STATE_AND_EVIDENCE_PLANE.md`
15. `docs/DEPENDENCY_POLICY.md`
16. `MCP_SURFACE.md`
17. `docs/FASTMCP_INTEGRATION.md`
18. `docs/DOGFOODING_FASTMCP.md`
19. relevant machine registries under `architecture/` and `design/registries/`

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
- No arbitrary shell, Lua, DFHack command, keyboard injection, bridge method, native address, or
  client-selected path through the default MCP surface.
- No GitHub-hosted Action is correctness, performance, compatibility, or release authority.
  Workflow files are portable specifications for local/self-hosted execution, including
  `doodlestein_self_releaser`.

## Evidence and status rules

Use this evidence ladder explicitly:

```text
source present
→ static/Python checked
→ full Rust-qualified
→ native R1-qualified
→ live R2-R5-qualified
→ registry-admitted
→ deployment-floor accepted
→ server-artifact qualified
→ runtime admitted
```

Rules:

- Evidence applies only to the exact source, binary, platform, versions, and inputs it names.
- A higher rung never silently transfers to a later commit or rebuilt binary.
- An empty registry means no admitted live tuple.
- A registry entry grants only the exact capabilities and coverage encoded in that entry.
- A local monotonic floor is anti-rollback custody, not compatibility evidence or distributed
  consensus.
- An admission-doctor report is deterministic diagnosis, not authority.
- A server receipt qualifies one executable, not a bridge session or game state.
- A single-use ticket authorizes one exact process start and grants no game mutation capability.
- Negative evidence may reject a claim but cannot certify success.
- Update `IMPLEMENTATION_STATUS.md` and `CHANGELOG.md` whenever implementation or evidence posture
  materially changes.

## Compatibility and launch rules

The current read-only protocol must retain exactly two native methods until a separately versioned
and qualified protocol says otherwise:

```text
Handshake
ReadObservation
```

Any new bridge method changes protocol identity and requires new wire tests, source digests, R1-R5
evidence, registry admission, floor advancement, server qualification, and launch evidence.

Admitted live startup must preserve the complete chain:

```text
exact registry bytes
+ owner-private monotonic floor
+ exact manifest
+ required entry ID
+ source-bound server receipt
+ opened executable identity and SHA-256
+ sanitized loader environment
+ single-use process ticket
+ descriptor-only exec
```

Do not add a path-based fallback. Re-read the registry and floor after artifact verification and
immediately before execution. Re-hash the already-open executable before ticket issuance and before
`exec`. The Rust consumer must revalidate and hash the current executable before deleting the
ticket and starting MCP.

The floor must remain:

- at an absolute path;
- under a real exact-mode `0700` directory;
- a regular non-symlink exact-mode `0600` file;
- root/effective-user owned;
- initialized exclusively;
- advanced only through lock + expected-file-digest compare-and-swap + atomic fsynced replacement;
- monotonic in sequence, digest chain, and accepted entry IDs.

Silent entry removal is not revocation. Design an explicit evidence-bearing revocation schema before
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

## Effects, obligations, and evidence

- A transport acknowledgment is not game mutation success, and game mutation success is not goal
  completion.
- Mutations use prepare, revalidate, commit, observe, and prove.
- Every mutating request has a stable idempotency key and an effect-journal identity.
- Two-phase effects apply to bridge mutation, observation publication, evidence publication, and
  checkpoint publication where partial completion would be ambiguous.
- Long-running work is a bounded obligation with terminal and failure predicates, game-time
  deadline, cadence, stability requirement, evidence, and cancellation policy.
- Cancellation is request, measurable drain progress, and finalize. It is not forgetting an
  operation.
- Indeterminate effects remain indeterminate until reconciled.
- ATP may move checkpoints, evidence, immutable generations, and state deltas. It may never become
  alternate mutation authority.
- The current live protocol is read-only. Do not smuggle pause, command, or any other effect into
  protocol V1.

## Determinism and graph rules

- Same canonical state, request, policy, seed, and budget must produce byte-identical eligible
  results, including order.
- Every graph algorithm with multiple valid outputs declares a canonical tie-break policy.
- Load-bearing graph decisions emit complexity and decision-path witnesses.
- Adaptive policies are clamped, sample-gated, circuit-breakable, and replayable. They may tune
  cost or attention, never weaken safety, authorization, conflict detection, compatibility,
  custody, or proof rules.
- Use immutable structural sharing and zero-copy views only after measuring the copy or boundary
  cost they remove. Preserve epoch, ordering, lifetime, and mutation semantics exactly.
- Hash canonical bytes, not incidental serialization. When a file digest is evidence, hash the
  exact bytes that were parsed or executed.

## Agent-facing synthesis rules

- Preserve the frozen eleven-tool waist. Improve ergonomics through shared schemas, profiles,
  affordances, typed handles, and progressive disclosure rather than top-level tool growth.
- Every success and error response converges on the canonical Agent Turn Packet. New fields remain
  additive unless a versioned contract explicitly changes the schema.
- Every response binds one complete anchor. State continuity as bootstrap, continuous, heartbeat,
  partial, gap, reset, stale, or indeterminate.
- Make active plans, actions, obligations, drains, confirmations, and indeterminate effects visible
  without requiring transcript memory.
- Separate `observed`, `certified_derived`, `inferred`, `predicted`, `assumed`, `stale`, `unknown`,
  `contradicted`, and `indeterminate`. Confidence never substitutes for epistemic class.
- Only observed and eligible certified-derived facts may satisfy mutation preconditions.
- Expose legal semantic affordances with capability, risk, precondition, cost, reversibility, and
  disabled-reason metadata. Affordances suggest; they do not authorize.
- Recommendations are structured protocol next steps with evidence, expected utility, expected
  information value, cost, risk, invalidators, and confirmation requirements. Return none rather
  than inventing busywork.
- Prefer value-of-information inspection: acquire a fact only when it can change a material
  decision enough to justify tokens, latency, bridge work, and game-time exposure.
- `pulse`, `briefing`, `tactical`, and `forensic` are semantic observation contracts, not vague
  verbosity settings. Safety, continuity, active work, uncertainty, and recovery survive every
  budget.
- An empty result proves absence only with a complete-domain coverage witness.
- Compare predicted and observed effects. Material divergence emits a surprise record.
- Memory, imported text, attention, recommendations, and counterfactuals never grant authority or
  satisfy a live precondition.
- A handoff packet must let a fresh agent resume safely without reconstructing the full transcript
  or trusting unverifiable prose.
- Non-MCP automation surfaces are machine-first, structured, bounded, and versioned. Keep stdout
  and stderr separate and prefer deterministic NDJSON where streaming applies.

## Coding guidance

Prefer small pure state-transition functions around an injected effect shell. Use ordered
collections whenever iteration order is observable. Bound every untrusted collection, nesting
level, string, frame, path, file, and output. Preserve unknown wire values where the protocol
requires forward compatibility. Do not use `unwrap`, `expect`, `panic`, `todo`, or `unimplemented`
in production code.

Expected source and contract text must be valid UTF-8, contain no NUL bytes, and remain within the
repository integrity bound. A text file that cannot be imported or parsed is a repository-integrity
failure, not a later test failure.

When changing a sealed gate, source-digest map, or schema:

1. update the machine contract;
2. update the implementation’s exact expected list;
3. update receipt generation;
4. update independent verification;
5. update focused tests and aggregate checkers;
6. update qualification wiring;
7. update documentation and status;
8. verify the lists are byte-for-byte semantically identical.

## Qualification and release

Run:

```bash
./scripts/verify.sh
./scripts/qualify_local.sh
```

A full qualification requires a clean source revision, locked/offline dependency resolution,
rustfmt, warning-denied Clippy, debug and release tests, warning-denied rustdoc, executable checks,
and every repository, bridge, acceptance, compatibility, floor, doctor, receipt, launcher, and
ticket gate. `DFMCP_STATIC_ONLY=1`, `DFMCP_ALLOW_MISSING_RUST=1`, or dirty-tree escape hatches create
development evidence only and must never be described as release qualification.

Qualify the release server separately with:

```bash
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

A release additionally requires exact asset contracts, checksums, signatures, SBOMs, install and
rollback evidence, and named platform/compatibility receipts.

## Definition of done

A feature is done only when its semantics, capability requirements, effect classification,
determinism classification, failure and recovery behavior, witness model, Agent Turn behavior,
epistemic and coverage semantics, compatibility and custody impact, registries, tests, benchmarks,
documentation, and acceptance evidence are explicit.

“The command ran,” “the code compiles,” “the test passed,” and “the source exists” are each useful
facts. None is sufficient by itself.
