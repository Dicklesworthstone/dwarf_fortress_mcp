# Sources and Research Ledger

This ledger records the primary material used for the phase-zero and deep-substrate design. A
repository README or design plan may describe a target state ahead of implementation; source-level
inspection informed the imports, but only this repository's acceptance evidence can certify them
for the Dwarf Fortress workload.

## Motivating proposal

- Doodlestein, X post, August 2025:
  <https://x.com/doodlestein/status/1958764361058574734>

The motivating idea was an MCP server through which coding agents could efficiently control Dwarf
Fortress while monitoring state and progress. This project treats that as the seed, not the full
specification.

## Dwarf Fortress and DFHack

- DFHack remote interface: <https://docs.dfhack.org/en/stable/docs/Remote.html>
- `dfhack-run`: <https://docs.dfhack.org/en/stable/docs/tools/dfhack-run.html>
- Lua API: <https://docs.dfhack.org/en/stable/docs/Lua%20API.html>
- DFHack source: <https://github.com/DFHack/dfhack>

The documented remote and Lua facilities make an out-of-process bridge feasible in principle.
They do not prove exact field coverage, mutation semantics, or compatibility for any named
Dwarf Fortress/DFHack pair; those require disposable-fort probes and version-specific evidence.

## Model Context Protocol

- Architecture: <https://modelcontextprotocol.io/specification/latest/architecture>
- Tools: <https://modelcontextprotocol.io/specification/latest/server/tools>
- Resources: <https://modelcontextprotocol.io/specification/latest/server/resources>
- Lifecycle: <https://modelcontextprotocol.io/specification/latest/basic/lifecycle>
- Transports: <https://modelcontextprotocol.io/specification/latest/basic/transports>

MCP transport work must re-check the then-current official specification. The internal `dfmcp`
semantic protocol is independently versioned.

## Franken-stack deep dives

### asupersync

Repository: <https://github.com/Dicklesworthstone/asupersync>

Material inspected included `src/cx/cx.rs`, `src/cancel/progress_certificate.rs`, ATP object,
manifest, and path modules, `docs/cx_authority_flow_graph.md`, and
`docs/atp_architecture.md`. Imported concepts include region-owned work, explicit context authority,
request/drain/finalize cancellation, two-phase effects, quantitative progress certificates,
deterministic laboratory execution, verified object DAGs, resumable manifests, RaptorQ repair,
and path-graph racing.

### FrankenSQLite

Repository: <https://github.com/Dicklesworthstone/frankensqlite>

Material inspected included the MVCC core types, concurrent-begin path, commit combiner,
deterministic rebase, physical merge, time travel, history compression, witness hierarchy,
witness refinement/publication, SSI e-process gate, regime monitor, page-lock combining, and
two-phase commit modules. Imported concepts include multi-version semantic snapshots, granular
positive and negative witnesses, phantom protection, conservative refinement, semantic intent
replay, stable-key structural merge, deterministic normal forms, proof-carrying merge, and brief
commit sequencing.

### FrankenFS

Repository: <https://github.com/Dicklesworthstone/frankenfs>

Material inspected included the comprehensive V1 specification, proposed architecture, MVCC
persistence/crash/compression modules, block cache and I/O policy, and repair autopilot, evidence,
ownership, pipeline, codec, exchange, and proof-of-retrievability modules. Imported concepts
include evidence-gated readiness, immutable generation publication, lease incarnation plus
generation fences, drain/drop/process queues, crash matrices, same-binary A/B receipts,
generation-monotone remote state, repair sealed to a state root, and retrievability audits.

### FrankenSearch

Repository: <https://github.com/Dicklesworthstone/frankensearch>

Material inspected included the decision plane, activation, commit replay, recall certificates,
generation roots, multi-resolution lattice, Quill query/delta arenas, durable job queue,
staleness, graph ranking, and adaptive fusion. Imported concepts include progressive cognition
under budgets, one immutable generation per request, verify-before-activate publication,
fail-closed coverage certificates, bounded non-recursive query arenas, lease-bounded mutable
deltas, deterministic fusion, and adaptive policies guarded by priors, clamps, sample thresholds,
and circuit breakers.

### FrankenMarkdown

Repository: <https://github.com/Dicklesworthstone/franken_markdown>

Material inspected included the comprehensive plan, span model, structural diff, transactional
file writer, batch publication, search index, verifier, and direct MCP/JSON implementation.
Imported concepts include a dependency-light semantic core, exact source spans, recoverable
diagnostics, deterministic multi-output publication, bounded direct protocol parsing,
machine-readable doctor/capability contracts, and exact citations into manuals and runbooks.

### FrankenGraphDB

Repository: <https://github.com/Dicklesworthstone/frankengraphdb>

- Comprehensive plan:
  <https://github.com/Dicklesworthstone/frankengraphdb/blob/main/COMPREHENSIVE_PLAN_FOR_THE_DESIGN_OF_FRANKENGRAPHDB.md>

Imported concepts include one version universe, content-addressed history, graph-structured
temperature tiers, factorized and worst-case-optimal execution, incremental Z-set projections,
branch-per-agent isolation, capability predicates before expansion, deterministic plan
certificates, reference-oracle testing, and acceptance-gated engineering.

### FrankenNetworkX

Repository: <https://github.com/Dicklesworthstone/franken_networkx>

Material inspected included the algorithm catalog, nightly workspace configuration, Canonical
Graph Semantics Engine, complexity/decision-path witness design, negative-evidence doctrine, and
zero-copy view analysis. Imported concepts include explicit canonical tie-break policies,
operation-count witnesses, reproducible decision-path hashes, order-preserving graph semantics,
immutable structural sharing, and the requirement to measure the actual copy/boundary cost before
claiming a zero-copy optimization.

### Doodlestein Self-Releaser

Repository: <https://github.com/Dicklesworthstone/doodlestein_self_releaser>

Material inspected included the release model and Rust repository template. Imported concepts
include workflows as locally executable specifications, clean source snapshots, bounded
multi-host builds, resume without blessing partial results, exact release-asset contracts,
checksums, signatures, SBOMs, source/sibling revision pinning, and machine-readable qualification
receipts. GitHub-hosted Actions are not part of this project's evidence model.

The complete adopt/adapt/reject analysis is in `FRANKENSTACK_DEEP_DIVE.md`; machine-readable
imports are in `architecture/franken_imports.json`.

## Dwarf Fortress MCP prior art

Repositories found during the August 2026 ecosystem check include:

- <https://github.com/alexanderolvera/dfhack-mcp>
- <https://github.com/Dodothereal/dfhack-mcp>
- <https://github.com/ryanbateman/vizier_mcp>
- <https://github.com/mrfentmen/dwarffortress-mcp>

These projects validate demand and supply implementation lessons. Concrete reused ideas must be
credited and license boundaries preserved.

## Research caveats

- Dwarf Fortress, DFHack, MCP, and sibling repositories evolve.
- Source-level plausibility is not workload-specific proof.
- Performance targets remain unmeasured until reproducible benchmark artifacts exist.
- Negative evidence can falsify a claim but cannot positively certify readiness.
- Derived indexes and graph projections cannot become authoritative over game observations.
- Every live mutation guarantee requires disposable-fort evidence for named versions.

## Owned MCP sibling: fastmcp_rust

- Source: <https://github.com/Dicklesworthstone/fastmcp_rust>
- Consumed as the MCP presentation plane, modern-only MCP 2026-07-28, exact-revision pinned
  (ADR-013, `docs/FASTMCP_INTEGRATION.md`); upstream defects are tracked per
  `docs/DOGFOODING_FASTMCP.md`.
- MCP 2026-07-28 specification: <https://modelcontextprotocol.io/specification/2026-07-28>

## Owned agent-memory sibling: eidetic_engine_cli

- Source: <https://github.com/Dicklesworthstone/eidetic_engine_cli>
- Adopted as the recommended agent-side campaign memory layer (`ee`), outside the canonical plane
  and outside the workspace dependency universe (doctrine: docs/EIDETIC_MEMORY.md, Part XXIII).
- Its "evidence before promotion", "derived indexes", and "no silent memory mutation" doctrines
  informed the memory-plane integration rules.
