# Comprehensive Plan for the Design and Implementation of `dwarf_fortress_mcp`

**Document class:** normative architecture and execution plan
**Initial issue date:** 2026-08-29
**Status:** Draft 0.2 — deep Franken-substrate revision for public iteration
**Repository:** `Dicklesworthstone/dwarf_fortress_mcp`
**Primary audience:** implementers, reviewers, autonomous coding agents, DFHack experts, systems
researchers, reliability engineers, and operators
**Normative companion files:** `design/registries/*.md`, `schemas/*.json`,
`proto/dfmcp.proto`, `SECURITY.md`, `IMPLEMENTATION_STATUS.md`,
`FRANKENSTACK_DEEP_DIVE.md`, and `architecture/*`

---

## Document control

This plan is intentionally more demanding than a conventional roadmap. Dwarf Fortress is not
merely a game process with buttons. It is a large, version-sensitive, partially observed
simulation in which actions may be accepted now, begin much later, complete only after many game
ticks, be invalidated by changing conditions, or produce effects that cannot be safely retried.
An MCP wrapper that ignores those facts will appear useful in a demo and then fail exactly when a
long-horizon agent begins to rely on it.

The plan therefore specifies:

- the semantic truth model;
- hard invariants and explicit non-goals;
- trust and failure domains;
- observation, query, intent, action, and obligation contracts;
- persistence, recovery, and compatibility behavior;
- capability, effect, determinism, error, schema, and test registries;
- quantitative performance and token-economy targets;
- work-package dependencies and acceptance gates;
- negative evidence required before production claims are permitted.

### Normative language

The terms **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD
NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** are used in their ordinary RFC 2119 sense.

A requirement is not implemented merely because code exists. It is implemented only when:

1. its behavior is represented in a stable contract or registry;
2. success and failure semantics are explicit;
3. compatibility and migration behavior are explicit;
4. deterministic tests cover the relevant transitions;
5. the acceptance evidence is retained;
6. documentation and schemas agree with the implementation.

### Evidence labels

This plan distinguishes four kinds of statements:

- **FACT:** directly established by a cited source or checked repository.
- **DESIGN:** a proposed normative choice for this project.
- **HYPOTHESIS:** a plausible claim requiring measurement.
- **TARGET:** an acceptance objective, not a measured current result.

Statements lacking a label are architectural requirements or explanatory prose, not claims of
present implementation.

### Stable identifiers

Requirements and work are referred to by stable IDs:

| Prefix | Meaning |
|---|---|
| `INV-` | hard invariant |
| `GOAL-` | project goal |
| `NONGOAL-` | explicit non-goal |
| `CAP-` | capability |
| `EFFECT-` | effect class |
| `ERR-` | stable error |
| `SCHEMA-` | versioned schema |
| `ADR-` | architecture decision |
| `WP-` | work package |
| `GATE-` | acceptance gate |
| `TEST-` | required test family |
| `SLO-` | measurable service objective |
| `RISK-` | tracked risk |
| `OPEN-` | unresolved design question |

Renumbering published identifiers is forbidden. Superseded entries remain as tombstones.

---

# Preface: the opportunity

The motivating observation was that no one appeared to have built a Dwarf Fortress MCP server
through which coding agents could efficiently control the game and monitor state and progress.
That idea is compelling for a deeper reason than novelty. Dwarf Fortress combines:

- a huge and semantically rich state space;
- incomplete, delayed, and version-dependent observability;
- thousands of interacting agents and resources;
- spatial planning;
- production logistics;
- military risk;
- social and psychological dynamics;
- open-ended objectives;
- irreversible and path-dependent consequences;
- long causal chains;
- a natural need for monitoring, diagnosis, planning, delegation, and recovery.

It is therefore an unusually powerful proving ground for agentic systems. But it will only be a
useful proving ground if the interface preserves the hard parts instead of replacing them with
screenshots, hidden omniscience, and imperative cheat commands.

The project’s thesis is:

> The best agent interface to Dwarf Fortress is a semantic, transactional, replayable control
> plane over a canonical, provenance-carrying world model. It should make useful actions cheap,
> dangerous actions difficult, uncertainty explicit, retries safe, long-running completion
> verifiable, failures reproducible, and multi-agent cooperation race-free.

The radical leap is not a larger catalog of tools. It is the substrate beneath the tools.

---

# Part I — Discovery and ecosystem forensics

## 1.1 Research question

The design process began with four questions:

1. What does the motivating proposal actually require for an agent to operate efficiently?
2. What supported DFHack surfaces can supply structured observation and mutation?
3. What has existing Dwarf Fortress MCP prior art already demonstrated?
4. Which mechanisms in the six sibling Franken projects are materially useful rather than merely
   thematically similar?

## 1.2 Motivating proposal

**FACT:** the indexed text of the motivating post asks why no one had made a Dwarf Fortress MCP
server that would allow an agent such as Codex or Claude Code to efficiently control the game and
monitor state and progress.

The phrase “efficiently control” is doing important work. Efficiency is not only bridge latency.
For an agent it includes:

- tokens required to understand the current situation;
- calls required to perform a coherent operation;
- retries consumed by stale state or ambiguous outcomes;
- context lost to repetitive dumps;
- work needed to discover that an operation did not complete;
- human attention required to recover from errors;
- compute spent reconstructing facts the server already knew.

The phrase “monitor state and progress” also rules out a command-only server. Progress is
temporal. It requires durable operation identity, later observations, terminal predicates, and a
way to explain why completion is blocked.

## 1.3 DFHack control-surface findings

**FACT:** DFHack documents an extensible remote interface using Protocol Buffers over a TCP
connection, with a core protocol and plugin-provided services.

**FACT:** DFHack’s `dfhack-run` mechanism can connect externally and invoke commands.

**FACT:** DFHack’s Lua API exposes broad structured access to game data and functions.

These facts support an out-of-process architecture. The Rust server does not need to scrape
process memory or link native DFHack code. A small bridge can live inside the supported DFHack
environment and expose a purpose-built, bounded, versioned semantic protocol.

### Bridge options considered

| Option | Advantages | Fatal or material drawbacks | Decision |
|---|---|---|---|
| Keyboard and mouse automation | Easy prototype; mimics ordinary play | Slow, brittle, hard to verify, inaccessible state, UI/version sensitivity | Diagnostic fallback only |
| Screenshot and vision | General; visually inspectable | Token-heavy, lossy, delayed, difficult identity/provenance | Non-authoritative evidence only |
| Arbitrary `dfhack-run` command strings | Broad command coverage | Ambient authority, injection, poor schemas, weak completion semantics | Bootstrap/diagnostic only |
| Lua evaluation endpoint | Rapid access to structures | Equivalent to remote code execution; unstable contracts | Forbidden by default |
| Rust-to-C++ FFI | Low latency | Expands unsafe/native trust domain; version and ABI risk | Rejected |
| Purpose-built DFHack RPC plugin/service | Structured, supported, versioned, bounded | Requires bridge maintenance across versions | Primary |
| Purpose-built Lua module behind bounded RPC | Faster prototyping, supported data access | Runtime and schema discipline still required | Early bridge implementation candidate |

## 1.4 Existing MCP prior art

Repository search shows that multiple Dwarf Fortress/DFHack MCP efforts now exist. At least one
offers a substantial read-mostly sensor/actuator surface and a preview/confirm/apply/undo style
mutation loop. This establishes several useful facts:

- agents can derive value from structured DFHack access;
- sensor and actuator naming can be made understandable;
- a preview step is useful;
- undo or compensation matters;
- installation and version friction are real;
- the idea has enough appeal to attract independent implementations.

It also establishes the minimum competitive baseline. A new project should not merely rename the
same sensors and actuators. The intended differentiation is:

1. canonical identity and semantic state independent of raw DFHack layout;
2. resumable hash-anchored deltas;
3. declarative query and attention under token budgets;
4. immutable intent compilation and two-phase commit;
5. idempotency and exact retry semantics;
6. bounded obligations and later proof of completion;
7. durable action, evidence, and recovery ledgers;
8. deterministic replay and fault exploration;
9. capability-scoped multi-agent leases with fencing;
10. explicit compatibility certification and degraded modes.

## 1.5 Findings from `asupersync`

The repository’s core claim is that async reliability is a protocol rather than a scheduler
feature. Relevant mechanisms include:

- context-carried cancellation, deadlines, budgets, tracing, and capabilities;
- region/nursery ownership that prevents orphan work;
- two-phase cancellation: request, drain, finalize;
- explicit outcome quadrants rather than error flattening;
- deterministic virtual-time execution and schedule exploration;
- trace/replay, fault injection, checkpoints, DPOR/model checking, and chaos;
- bounded, partially reliable actor transport through ATP;
- strong resource limits and malicious-input posture;
- safe Rust and narrow effect boundaries.

### Design import

Long-running fortress operations are modeled as owned obligations inside structured regions.
A session owns plans; plans own action coordinators; coordinators own obligations, leases, bridge
calls, timers, and evidence writers. Session cancellation cannot simply drop futures. It must
request cancellation, prevent new effects, drain in-flight operations, reconcile receipts,
compensate where policy allows, and leave every obligation terminal or explicitly indeterminate.

`Cx`-like operation context is not an internal convenience. Its budgets, deadlines, capabilities,
trace identity, and cancellation state cross every effect boundary.

## 1.6 Findings from `frankensqlite`

Relevant mechanisms include:

- safe-Rust, clean-room database architecture with no C FFI;
- asynchronous operation and context-first APIs;
- WAL, checksums, atomic commit, and torn-write recovery;
- MVCC and lock-free readers;
- deterministic fault injection and differential testing;
- explicit storage layers and VFS abstraction;
- transaction, planner, VM, and B-tree separation;
- recovery as a first-class correctness surface.

### Design import

The server needs a durable ledger that can answer, after a crash:

- Which sessions existed?
- What anchor did an agent observe?
- Which plan digest was prepared?
- Which capabilities and leases authorized it?
- Was a checkpoint complete before commit?
- Which bridge requests were sent?
- Which receipts were persisted?
- Which semantic effects were later observed?
- Which obligations remain active?
- Which effects are indeterminate?
- Which idempotency keys have already produced an effect?

This data belongs in a transactional ledger, not scattered logs. The first production persistence
adapter should be built around FrankenSQLite only after its required APIs pass an integration
gate. Until then, the contract must admit a deterministic in-memory adapter and a reference file
ledger without changing semantics.

## 1.7 Findings from `frankenfs`

Relevant mechanisms include:

- deterministic core logic with injected block device, clock, and fault effects;
- snapshot and clone-on-write semantics;
- crash-consistency and repair workflows;
- doctor bundles with machine- and human-readable forms;
- repair plan/apply separation guarded by content seals;
- event rings, bounded diagnostics, and reproducible experiments;
- no hidden time, randomness, or direct syscalls in the pure core.

### Design import

Save/checkpoint handling must be its own capability domain. The server should discover saves
through an injected filesystem view, produce content-addressed manifests, clone or copy under
explicit policy, fsync in a defined order, verify completed checkpoints, and package evidence
without path traversal or ambient authority.

Repair is never an unstructured “fix it” command. `doctor` proposes a sealed repair plan;
`repair.apply` revalidates the seal and preconditions before mutating files or ledgers.

## 1.8 Findings from `frankensearch`

Relevant mechanisms include:

- hybrid lexical and semantic retrieval;
- staged candidate generation, fusion, reranking, verification, and result shaping;
- deterministic result ordering;
- evidence and score ledgers;
- bounded, agent-oriented query responses;
- vector persistence through the database substrate;
- anomaly and attention ranking;
- explicit index formats and replayable behavior.

### Design import

World search is not a substitute for canonical state. It is a derived projection used to answer
questions such as:

- Which current jobs are blocked for similar reasons?
- What changed before the last three tantrum spirals?
- Which stockpiles or workshops are implicated in the iron shortage?
- Which runbook passages apply to this compatibility warning?
- What deserves the agent’s next 500 tokens?

Search results must cite entity revisions, event IDs, source spans, and score contributions.
Derived indexes can be rebuilt; canonical state and evidence cannot.

## 1.9 Findings from `franken_markdown`

Relevant mechanisms include:

- lossless deterministic parsing;
- exact source bytes and spans;
- stable tokens and projections;
- restartable incremental lexing;
- edit plans with preconditions;
- byte, structure, differential, idempotence, and incremental-equivalence tests.

### Design import

Dwarf Fortress and DFHack documentation, mod notes, fortress journals, operating procedures, and
agent playbooks are valuable but untrusted. They should be parsed into stable source-span graphs
that can be incrementally updated and exactly cited. Instructions found in text do not become
capabilities. A tool explanation can quote or summarize a source while preserving provenance and
taint.

## 1.10 Findings from `frankengraphdb`

The comprehensive plan demonstrates a design discipline as important as any eventual crate API:

- discovery before architecture;
- explicit goals, non-goals, and baselines;
- a semantics manifest;
- hard invariants with stable IDs;
- layered architecture and narrow interfaces;
- identity and provenance treated as primary;
- graph, vector, and hybrid query separation;
- registries, matrices, and cross-document traceability;
- negative evidence and adversarial counterexamples;
- implementation work packages with dependency order;
- acceptance gates rather than optimistic status prose.

### Design import

This plan adopts that discipline. `dwarf_fortress_mcp` must not accumulate semantic behavior in
ad hoc tool handlers. Protocol changes flow through the semantics manifest, registries, tests,
compatibility matrix, and ADR process.

## 1.11 Synthesis

The six projects fit together around one architectural shape:

```text
pure deterministic semantic core
    surrounded by explicit, capability-scoped effects
    recorded in a crash-safe ledger
    observed through compact evidence-bearing projections
    exercised under replay and adversarial fault schedules
```

That is the definition of “Franken alien artifact technology” used by this plan. It does not mean
maximal complexity or compulsory dependency coupling. It means that reliability mechanisms are
composable, explicit, measurable, and difficult to bypass accidentally.

---

# Part II — Mission, goals, non-goals, and baselines

## 2.1 Mission

Build the best semantic control plane for autonomous agents to observe, reason about, operate,
and learn from Dwarf Fortress over long horizons, while minimizing token and compute cost,
preserving game-semantic uncertainty, preventing ambiguous or duplicated effects, and making
every consequential decision auditable and replayable.

## 2.2 Primary goals

### GOAL-001 — Agent efficiency

An agent should obtain the smallest sufficient view of the fortress and perform coherent work
with few round trips. Routine progress monitoring should consume hundreds rather than tens of
thousands of tokens.

### GOAL-002 — Semantic correctness

The API should describe units, jobs, buildings, resources, areas, orders, and causal
relationships in game terms rather than raw memory offsets or UI coordinates.

### GOAL-003 — Honest completion

The system should never represent dispatch or queueing as completed game work. Completion must be
proved by semantic postconditions over authoritative observations.

### GOAL-004 — Retry safety

A repeated request with the same idempotency identity must not duplicate effects. A repeated
request with conflicting content must fail.

### GOAL-005 — Crash recovery

After process, bridge, database, or host failure, the server must recover known state and
distinguish completed, failed, cancelled, pending, and indeterminate effects.

### GOAL-006 — Deterministic diagnosis

Given a doctor bundle and compatible implementation, core decisions should replay to the same
anchors, plans, scores, and state transitions.

### GOAL-007 — Capability security

No component should possess more authority than its operation requires. Scopes must cover
fortress, entities, map regions, action classes, time, risk, uses, and budgets.

### GOAL-008 — Version resilience

DF and DFHack version differences must be detected and represented through compatibility
manifests, field availability, translation rules, golden fixtures, and degraded modes.

### GOAL-009 — Multi-agent safety

Concurrent agents should cooperate through leases and delegation without races, stale writers,
or implicit global ownership.

### GOAL-010 — Economical scalability

The design should support one local fortress on modest hardware first, then many sessions and
remote workers without changing semantic contracts. Indexes, derived projections, and transport
must be optional layers around the same canonical truth.

### GOAL-011 — Safe-Rust trust domain

The Rust workspace must forbid unsafe code and avoid direct C/C++ FFI. Native DFHack integration
is quarantined out of process.

### GOAL-012 — Useful research instrument

The deterministic lab, action/obligation traces, and fortress outcomes should make the project a
serious benchmark for long-horizon planning, coordination, reliability, and agent epistemology.

## 2.3 Non-goals

### NONGOAL-001 — Human-input imitation

The project is not primarily a keyboard/mouse macro or UI-navigation emulator.

### NONGOAL-002 — Screenshot omniscience

Vision may support diagnosis but is not canonical world state.

### NONGOAL-003 — Command-shell exposure

The default MCP server will not offer arbitrary shell, Lua, DFHack command, memory-write, or file
path execution.

### NONGOAL-004 — Semantic flattening

The project will not claim that every Dwarf Fortress concept fits a universal game-control
ontology. Dwarf Fortress-specific semantics are a strength.

### NONGOAL-005 — Hidden cheating

The project will not silently expose facts unavailable under its declared observation policy.
Adapters must mark privileged or synthetic facts.

### NONGOAL-006 — Premature distributed consensus

The first production target is a single authoritative fortress control plane with durable local
state. Distributed operation may use ATP and replicated read projections later; it must not force
consensus complexity into phase one.

### NONGOAL-007 — Automatic unsafe recovery

The system will not guess that an indeterminate mutation failed and retry it. Reconciliation or a
sealed operator decision is required.

### NONGOAL-008 — Compatibility by hope

Unknown DF/DFHack/mod combinations will not be treated as known-good merely because the bridge
started.

### NONGOAL-009 — Unbounded world dumps

No read tool may bypass budgets with “return everything.”

### NONGOAL-010 — Finished-product theater

Scaffolds, mocks, and shadow-mode behavior must be labeled. Status is derived from acceptance
evidence, not README enthusiasm.

## 2.4 Baselines

Three baselines matter.

### Baseline A — manual/visual operation

A human reads screens and navigates menus. It has strong semantic judgment but poor machine
throughput and little deterministic auditability.

### Baseline B — imperative DFHack automation

Scripts and commands can mutate game state efficiently, but their schemas, authorization,
idempotency, temporal completion, and recovery semantics vary.

### Baseline C — sensor/actuator MCP

Structured tools improve agent access, but a broad tool list alone does not create canonical
state, delta continuity, durable obligations, transactional intent, or multi-agent concurrency.

The project must measure itself against all three. It should preserve the semantic richness of
human operation, the throughput of DFHack, and the discoverability of MCP while adding systems
properties absent from each.

## 2.5 Quantitative targets

These are **TARGETS**, not current measurements.

| ID | Target |
|---|---|
| `SLO-001` | p99 heartbeat response ≤ 10 ms on the reference local host |
| `SLO-002` | p99 bounded semantic delta generation ≤ 25 ms for the reference fortress |
| `SLO-003` | p99 indexed entity/query response ≤ 50 ms before model-token generation |
| `SLO-004` | p99 direct static plan compilation ≤ 100 ms for ≤ 64 steps |
| `SLO-005` | ordinary heartbeat ≤ 150 output tokens |
| `SLO-006` | ordinary meaningful delta ≤ 500 output tokens |
| `SLO-007` | default situation summary ≤ 1,500 output tokens |
| `SLO-008` | no duplicate verified effect under 10,000 deterministic retry/crash schedules |
| `SLO-009` | no silent cursor gap or unknown-field coercion in the compatibility corpus |
| `SLO-010` | full ledger recovery deterministic across 1,000 injected crash points per release |
| `SLO-011` | idle read-only server ≤ 150 MiB RSS excluding optional indexes |
| `SLO-012` | canonical state storage grows sublinearly with observation count through deltas and compaction |
| `SLO-013` | bridge payload, string, collection, and recursion limits enforced before allocation amplification |
| `SLO-014` | every mutating tool response includes plan/action identity and evidence or explicit indeterminacy |
| `SLO-015` | doctor identifies the earliest compatibility or recovery divergence in the certified corpus |

The reference fortress, host, DF version, DFHack version, mods, and benchmark procedure must be
versioned with every published result.

# Part III — Semantics manifesto and hard invariants

## 3.1 Semantics manifesto

The system is built around ten semantic commitments.

1. **Canonical truth is explicit.** Raw bridge structures, cached projections, search indexes,
   agent memory, and UI output are not interchangeable.
2. **Unknown is not false.** Missing because unsupported, omitted by projection, inaccessible,
   not yet observed, and semantically absent are distinct states.
3. **Time is plural.** Wall time, monotonic runtime time, game ticks, calendar time, and ledger
   sequence are different domains.
4. **Identity outlives presentation.** Labels and coordinates may change; stable typed IDs and
   generations anchor references.
5. **Observation is a protocol.** Full snapshots, deltas, continuations, heartbeats, and epoch
   resets have formal continuity rules.
6. **Mutation is a transaction-like protocol.** Intent, plan, preparation, revalidation, commit,
   observation, and proof are distinct.
7. **Long work is an obligation.** A job that may finish later must have an owner, deadline,
   terminal states, and evidence.
8. **Cancellation is work.** Requesting cancellation starts a drain/reconciliation protocol; it
   does not erase responsibility.
9. **Authority is data.** Capabilities, leases, risk ceilings, and budgets travel with operations
   and are checked at every effect boundary.
10. **Reproducibility is a feature.** If a failure cannot be replayed or explained, the system has
    lost important state.

## 3.2 Hard invariants

### Truth and identity

- **INV-001 — Single canonical anchor:** every authoritative world view has exactly one fortress
  ID, observation epoch, sequence, game tick, and canonical state hash.
- **INV-002 — Hash exclusion discipline:** a state hash covers canonical semantic bytes and never
  incidental map iteration, pointer identity, transport framing, or presentation ordering.
- **INV-003 — Stable entity identity:** entity references include a type domain and generation or
  equivalent anti-ABA evidence.
- **INV-004 — No label identity:** names, translated names, titles, and coordinates are never
  primary keys.
- **INV-005 — Explicit field presence:** unsupported, unknown, omitted, redacted, stale, null, and
  absent are representable without coercion.
- **INV-006 — Provenance retention:** every externally meaningful fact can identify its source
  field or derivation, observation tick, compatibility status, and source digest.
- **INV-007 — Derived-state separation:** indexes, summaries, scores, embeddings, and agent notes
  cannot overwrite canonical facts.

### Observation

- **INV-008 — Cursor continuity:** a delta applies only to the exact base cursor and state hash it
  names.
- **INV-009 — Epoch discontinuity:** restore, non-resumable adapter reset, or incompatible schema
  transition creates a new epoch and requires a full snapshot.
- **INV-010 — Honest truncation:** a partial read is marked truncated and carries a bounded,
  integrity-protected continuation; it is never presented as complete.
- **INV-011 — Monotone revisions:** content changes advance entity, edge, map-chunk, or aggregate
  revision.
- **INV-012 — No dangling canonical edges:** every canonical edge endpoint resolves in the same
  complete snapshot or is explicitly typed as an external reference.
- **INV-013 — Event deduplication:** event identity is stable across polling overlap; duplicate
  delivery cannot create duplicate canonical events.
- **INV-014 — Bounded observations:** every observation has explicit entity, byte, time, depth,
  and token limits.

### Planning and mutation

- **INV-015 — Immutable prepared plan:** after preparation, changing any covered field changes the
  plan digest and invalidates the prepare receipt.
- **INV-016 — Revalidation before effect:** affected state, capabilities, leases, and checkpoint
  requirements are revalidated immediately before the first game mutation.
- **INV-017 — Idempotent mutation identity:** every mutating step has a stable idempotency key;
  conflicting content under the same key fails.
- **INV-018 — Dispatch is not completion:** adapter acceptance cannot transition an action to
  `verified`.
- **INV-019 — Semantic postconditions:** every mutating step has registered postconditions over
  normalized world state.
- **INV-020 — Bounded temporal work:** any action whose goal may complete later has a bounded
  obligation.
- **INV-021 — Explicit indeterminacy:** when the system cannot determine whether an effect
  occurred, it records `indeterminate` and forbids automatic duplicate retry.
- **INV-022 — Risk monotonicity:** compilation may raise an action’s risk classification in light
  of context but may not lower the registry minimum.
- **INV-023 — Checkpoint ordering:** when policy requires a checkpoint, durable checkpoint proof
  precedes the first guarded effect.
- **INV-024 — Compensation is not rollback:** compensation is a new authorized action with its own
  preconditions, effects, and evidence.
- **INV-025 — Terminal-state stability:** terminal action and obligation records are immutable
  except for append-only explanatory evidence or an explicit reconciliation supersession.

### Ownership, cancellation, and concurrency

- **INV-026 — Owned work:** every task, bridge request, timer, lease, action, and obligation is
  owned by a structured region with a defined shutdown path.
- **INV-027 — Cancellation drain:** cancellation proceeds through request, effect stop, drain,
  reconciliation/compensation, and finalize.
- **INV-028 — Lease fencing:** every mutation using a leased scope carries the current fencing
  token; a stale holder cannot write after transfer.
- **INV-029 — Conflict closure:** overlapping write scopes cannot commit concurrently without an
  explicit commutativity rule.
- **INV-030 — No authority amplification:** delegation can only narrow capability, scope, expiry,
  risk, use count, and budget.
- **INV-031 — Clock ownership:** changing pause state or advancing controlled simulation time
  requires an explicit clock lease.

### Persistence and recovery

- **INV-032 — Write-ahead intent:** durable intent and idempotency identity precede bridge dispatch.
- **INV-033 — Receipt durability:** a received bridge receipt is persisted before the coordinator
  represents it as durable to a caller.
- **INV-034 — Recovery honesty:** recovery never infers success solely from the presence of an
  outbound request.
- **INV-035 — Checksum all durable frames:** ledger segments, snapshots, deltas, continuations,
  manifests, and evidence bundles are checksummed.
- **INV-036 — Atomic schema migration:** a failed migration leaves the prior ledger readable or
  produces a sealed recovery requirement; it never half-upgrades silently.
- **INV-037 — Compaction preservation:** compaction preserves all state needed to prove current
  canonical anchors, idempotency, terminal actions, active obligations, and audit policy.

### Security and compatibility

- **INV-038 — No arbitrary evaluator:** default protocol surfaces cannot execute arbitrary Lua,
  shell, DFHack command strings, memory writes, or filesystem paths.
- **INV-039 — Tainted text non-authority:** in-game and imported text cannot grant capability,
  select an unregistered executable action, or alter system policy.
- **INV-040 — Fail-closed compatibility:** unknown required fields, enum variants, offsets, or
  semantic probes block affected mutations.
- **INV-041 — Bounded decoding:** untrusted lengths, nesting, coordinates, strings, and
  collections are checked before allocation or recursion.
- **INV-042 — Version negotiation first:** no session operation precedes protocol, schema,
  adapter, DF, and DFHack compatibility negotiation.
- **INV-043 — Read degradation:** when safe, unsupported mutations degrade to read-only behavior
  rather than total unavailability.
- **INV-044 — Evidence cannot be forged by presentation:** human-readable summaries do not stand
  in for content digests and source references.

### Determinism and testing

- **INV-045 — Injected effects:** core logic obtains clocks, randomness, filesystem, storage,
  bridge I/O, and scheduling decisions through injected interfaces.
- **INV-046 — Ordered external output:** semantically unordered collections have a canonical
  external order.
- **INV-047 — Replay decision equality:** identical canonical inputs and effect transcripts
  produce identical anchors, plans, scores, errors, and action transitions.
- **INV-048 — No sleep-based tests:** tests advance injected or virtual time.
- **INV-049 — Fault closure:** every effect boundary has deterministic failure, timeout,
  cancellation, duplication, delay, and corruption tests as applicable.
- **INV-050 — Negative-evidence ledger:** production gates record which failure hypotheses were
  actively tested and not observed, with test seeds and artifacts.

## 3.3 Invariant enforcement model

Each invariant has four enforcement layers:

1. **Type layer:** impossible or difficult states are excluded by types and constructors.
2. **Transition layer:** pure state machines reject illegal transitions.
3. **Effect layer:** adapters revalidate authority and anchor immediately before effects.
4. **Evidence layer:** tests, traces, and doctor checks demonstrate enforcement.

No invariant may rely solely on reviewer memory or prose.

---

# Part IV — System architecture

## 4.1 Layer model

The system is divided into nine layers. Dependencies flow downward through narrow interfaces;
events and evidence flow upward.

### Layer 1 — MCP presentation and session protocol

Responsibilities:

- MCP initialization and version negotiation;
- stdio and Streamable HTTP transports;
- tool, resource, and prompt registration;
- session identity and authentication;
- request parsing, bounded validation, and response shaping;
- progress notifications and cancellation routing;
- output-token budgeting and continuation presentation.

This layer does not understand raw DFHack structures and does not perform game effects directly.

### Layer 2 — authority and policy

Responsibilities:

- capability verification;
- delegation and scope narrowing;
- risk policy;
- confirmation seals;
- budgets and quotas;
- agent/session ownership;
- multi-agent clock and write leases;
- prompt-injection taint policy.

Policy decisions produce evidence and stable denial reasons.

### Layer 3 — observation, query, and attention

Responsibilities:

- interest-set management;
- snapshot/delta/heartbeat selection;
- query planning over canonical state and derived indexes;
- token-aware projection;
- anomaly and priority ranking;
- score ledgers;
- continuation state.

This layer reads canonical state through snapshot transactions.

### Layer 4 — intent compiler and plan verifier

Responsibilities:

- normalize intent;
- resolve stable identities;
- validate constraints;
- expand recipes into semantic actions;
- infer or require preconditions and postconditions;
- produce a topologically ordered action DAG;
- calculate scopes, capabilities, risks, checkpoints, obligations, and predicted semantic diffs;
- seal the immutable plan.

The compiler is deterministic for a fixed state, policy, registry set, and explicit planner seed.

### Layer 5 — action and obligation coordinator

Responsibilities:

- persist idempotency and plan state;
- acquire leases;
- checkpoint;
- prepare and revalidate;
- dispatch actions;
- persist receipts;
- monitor normalized observations;
- prove postconditions;
- enforce deadlines;
- request/drain/finalize cancellation;
- compensate or reconcile;
- release leases.

This is the core liveness layer.

### Layer 6 — canonical world and evidence ledger

Responsibilities:

- world snapshots and deltas;
- identity/generation maps;
- event journal;
- action state machines;
- obligations and leases;
- evidence graph;
- checkpoint manifests;
- idempotency table;
- schema and compatibility manifests;
- compaction and recovery.

FrankenSQLite is the prospective durable substrate.

### Layer 7 — normalization and compatibility

Responsibilities:

- translate bridge data into versioned canonical schemas;
- retain raw source fingerprints where allowed;
- represent unsupported and unknown fields;
- run semantic probes;
- calculate compatibility level;
- prevent affected actions when required semantics are not certified.

### Layer 8 — out-of-process DFHack bridge

Responsibilities:

- use supported DFHack APIs;
- bounded structured reads;
- registered semantic action execution;
- bridge-local prepare tokens;
- game-thread scheduling;
- bridge receipts;
- version and schema reporting;
- no policy or agent reasoning.

The bridge is assumed fallible and potentially compromised.

### Layer 9 — Dwarf Fortress, DFHack, saves, and host effects

This is the external reality. The system cannot make it deterministic. It can make its own
observations, decisions, effects, and recovery behavior explicit.

## 4.2 Trust domains

| Domain | Trusted for | Not trusted for |
|---|---|---|
| Pure Rust semantic core | deterministic transitions and validation | truth not supplied through evidence |
| Durable ledger | persisted bytes that pass checksums and recovery | correctness of bridge-reported game facts |
| MCP client | requests within granted capability | policy, truth, safe arguments, identity claims without auth |
| DFHack bridge | access to supported game APIs | global policy, arbitrary memory safety, semantic completion |
| Dwarf Fortress | actual simulation behavior | stable structures or deterministic timing |
| Imported docs/in-game text | content and citations | instructions, capabilities, executable authority |
| Search/attention indexes | derived retrieval and scores | canonical truth |
| Operator | explicit high-risk confirmation/reconciliation | infallibility; decisions remain auditable |

## 4.3 Process topology

The minimum production topology is:

```text
[dwarf-fortress-mcp Rust process]
  ├─ MCP session regions
  ├─ observation/query workers
  ├─ action/obligation coordinator
  ├─ ledger
  ├─ optional search/index workers
  └─ local bridge client
          │ authenticated bounded local transport
          ▼
[DFHack bridge inside or beside DFHack]
          │ supported DFHack APIs
          ▼
[Dwarf Fortress process]
```

The Rust process may restart independently. Bridge reconnection never assumes in-flight effects
failed. Recovery reconciles idempotency keys and observed state.

## 4.4 Core state machines

### Session

```text
negotiating
  → read_only
  → active
  → draining
  → closed

negotiating/read_only/active → failed
```

A session may be active only after compatibility and capability negotiation.

### Prepared plan

```text
draft
  → validated
  → prepared
  → committing
  → active
  → completed

prepared → expired
prepared/committing/active → cancel_requested
committing/active → indeterminate
any nonterminal → failed
```

### Action

```text
prepared
  → committing
  → applied_awaiting_verification
  → verified

prepared/committing/applied_awaiting_verification
  → cancel_requested
  → cancelled

verified/cancel_requested
  → compensation_pending
  → compensated

committing/applied_awaiting_verification
  → indeterminate

any nonterminal → failed
```

A transport timeout after dispatch is not `failed`; it is usually `indeterminate` until
reconciliation.

### Obligation

```text
registered
  → active
  → terminal_candidate
  → discharged

active/terminal_candidate → blocked
active/blocked/terminal_candidate → cancel_requested
cancel_requested → cancelled
active/blocked/terminal_candidate → expired
any nonterminal → indeterminate
```

`terminal_candidate` exists to require stability across multiple observations when a single frame
could be transient or incomplete.

### Lease

```text
requested → granted → renewing → released
requested → denied
granted/renewing → expired
granted/renewing → revoked
```

Every grant carries a monotonically increasing fencing token for its scope domain.

## 4.5 Pure core and effect shell

Core transitions operate on explicit input records and return:

- a new immutable state;
- effect intents;
- emitted evidence;
- timers to arm;
- stable errors.

Effect executors perform storage, bridge, filesystem, transport, or clock work and feed receipts
back into the transition function. This architecture allows deterministic replay and exhaustive
fault exploration.

Example:

```text
transition(ActionState::Prepared, CommitRequested)
  => ActionState::Committing
  + PersistDispatchIntent
  + AcquireLease
  + EnsureCheckpoint
  + RevalidateBridgeState

transition(ActionState::Committing, RevalidationPassed)
  => ActionState::Committing
  + SendBridgeCommit

transition(ActionState::Committing, BridgeTimeoutAfterSend)
  => ActionState::Indeterminate
  + ReconciliationObligation
  + EmergencyPauseIfPolicy
```

## 4.6 Crate decomposition target

The phase-zero scaffold starts with six crates. The production decomposition may expand to:

| Crate | Responsibility |
|---|---|
| `dfmcp-core` | IDs, anchors, errors, contexts, budgets, capabilities, evidence, digests |
| `dfmcp-schema` | canonical schema versions and bounded codecs |
| `dfmcp-world` | graph, chunks, facts, snapshots, deltas, predicates |
| `dfmcp-query` | DfQL, projection, continuation, attention scoring |
| `dfmcp-intent` | semantic actions, recipes, constraints, plan compiler |
| `dfmcp-coordinator` | action, obligation, cancellation, leases, reconciliation |
| `dfmcp-ledger` | persistence traits and in-memory reference |
| `dfmcp-ledger-frankensqlite` | durable FrankenSQLite adapter |
| `dfmcp-checkpoint` | save manifests and checkpoint protocol |
| `dfmcp-checkpoint-frankenfs` | FrankenFS-backed implementation |
| `dfmcp-search` | index interfaces and reference search |
| `dfmcp-search-franken` | FrankenSearch adapter |
| `dfmcp-docs` | source-span knowledge corpus |
| `dfmcp-docs-franken-markdown` | FrankenMarkdown adapter |
| `dfmcp-graph` | graph projection interfaces |
| `dfmcp-graph-franken` | FrankenGraphDB adapter when ready |
| `dfmcp-adapter` | game adapter trait and receipts |
| `dfmcp-bridge-client` | bounded bridge transport |
| `dfmcp-mcp` | MCP protocol/session implementation |
| `dfmcp-lab` | virtual time, fake game, replay, fault schedules |
| `dfmcp-doctor` | diagnosis and sealed repair planning |
| `dwarf-fortress-mcp` | binary and configuration |

Crate boundaries may change through ADRs, but the semantic boundaries may not be collapsed merely
to reduce file count.

## 4.7 Configuration precedence

Configuration sources, from lowest to highest precedence:

1. compiled safe defaults;
2. system configuration;
3. user configuration;
4. fortress profile;
5. session policy;
6. per-request narrowing.

A higher layer may narrow authority or budgets by default. It may not silently broaden them.
Environment variables are restricted to bootstrap paths and transport addresses; secrets use
dedicated secret providers.

---

# Part V — Identity, canonical world model, and provenance

## 5.1 Identity domains

Identifiers are typed and opaque at the protocol surface.

```text
fortress:<world-fingerprint>:<site-id>
entity:<kind>:<native-id>:<generation>
edge:<relation>:<digest>
event:<source-domain>:<native-or-derived-id>
session:<random-128>
plan:<digest-prefix>
action:<plan-id>:<step-id>:<attempt-domain>
obligation:<action-id>:<purpose>
lease:<scope-domain>:<fencing-token>
checkpoint:<content-digest>
evidence:<content-digest>
```

Presentation strings may shorten these forms, but wire schemas preserve type.

## 5.2 Fortress identity

A fortress identity is derived from a manifest containing:

- world/site identifiers exposed by DFHack;
- save/world metadata;
- a stable world fingerprint;
- schema version;
- optional operator alias.

Moving or renaming a save does not change fortress identity. Cloning a checkpoint creates a
lineage record. A branch may intentionally receive a new fortress-lineage ID while preserving
the parent checkpoint digest.

## 5.3 Entity identity and ABA prevention

Raw numeric IDs may be reused after destruction, unload/reload, or version-specific behavior. The
normalizer maintains a generation map keyed by `(fortress, entity kind, native ID)`. Generation
advances when evidence establishes that the prior referent ended and a distinct referent now
occupies the key.

An entity reference is valid only when:

- kind matches;
- native identity matches;
- generation matches;
- required revision constraints hold;
- compatibility manifest supports the identity derivation.

A stale generation produces `ERR-STALE-ENTITY`, not a best-effort match by name.

## 5.4 Fact presence algebra

Every canonical field uses a presence state:

```text
Known(value)
Absent                 // semantically established not present
Unknown(reason)        // required truth not established
Unsupported(version)   // adapter cannot expose it
Omitted(projection)    // known/unknown state not included in this response
Redacted(policy)       // withheld by capability or privacy policy
Stale(last_tick)       // prior value retained but freshness requirement failed
```

This algebra prevents common failures such as treating an unsupported military assignment as
“not assigned” or an omitted inventory list as empty.

## 5.5 Provenance

A fact provenance record includes:

- source kind: DFHack field, bridge derivation, canonical derivation, replay, operator assertion;
- source path or registered derivation ID;
- observed game tick and observation cursor;
- source schema and compatibility manifest;
- source-content digest;
- confidence class;
- taint flags;
- optional evidence parents.

Derived facts form a DAG. For example, `food_days_remaining` may cite stock quantities,
population, consumption assumptions, and a formula registry version.

## 5.6 Entity model

Initial canonical entity classes:

| Domain | Entity classes |
|---|---|
| Fortress | fortress, civilization, site, season/calendar, policy aggregate |
| Population | unit, historical figure, role, profession, need, thought, syndrome |
| Inventory | item, stack, material lot, container, artifact |
| Work | job, task, work order, labor assignment, workshop queue |
| Structures | building, construction, workshop, furnace, furniture, trap |
| Space | zone, stockpile, burrow, room, route, map feature |
| Military | squad, position, schedule, alert, order, equipment assignment, threat |
| Ecology | creature, plant, tree, vermin, biome feature |
| Governance | mandate, demand, justice case, noble office, standing order |
| Information | announcement, report, combat log, manager notice |
| Agent operations | plan, action, obligation, lease, checkpoint, evidence |

The last domain is stored in the control ledger but projected into the same query graph so an
agent can ask how its plans relate to world entities.

## 5.7 Edge model

Initial edge classes:

```text
located_at
contained_in
owns
assigned_to
member_of
occupies
performs
requires
consumes
produces
blocks
depends_on
ordered_by
managed_by
protected_by
threatens
caused_by
evidenced_by
derived_from
reserved_by
leased_by
checkpointed_by
```

Edges carry revisions and provenance. Hyper-relations are represented as relationship entities
when they have independent identity, fields, or temporal state.

## 5.8 Spatial model

Representing every tile as a graph entity would be catastrophically expensive. The map uses:

- fixed or negotiated chunk dimensions;
- chunk coordinates and revisions;
- terrain run-length encoding;
- bitplanes for designation, occupancy, liquid, temperature class, visibility, and other common
  flags;
- sparse overlays for buildings, items, jobs, flows, contaminants, and exceptional metadata;
- content digests per chunk;
- optional multi-resolution summaries.

Spatial entities refer to points, cuboids, polygons, or chunk-relative masks. Plans lease and
hash the exact affected geometry.

## 5.9 Canonical encoding

Canonical bytes use:

- explicit schema and domain tags;
- fixed endianness;
- length prefixes;
- sorted map keys and set values;
- integer or fixed-point numeric representation;
- normalized strings only where the field’s semantics require normalization;
- raw bytes retained separately when exact source representation matters;
- no floating-point values in hashed semantic state unless a future schema defines canonical
  NaN, rounding, and encoding rules.

State hash algorithm v1 is SHA-256 over a framed canonical stream. The algorithm is a schema
choice and may later support additional digests through multihash-style tagging. Migration never
relabels old hashes.

## 5.10 Snapshot completeness

A canonical **full snapshot** contains all fields required by its declared completeness profile.
Profiles may include:

- `control-minimum`: enough for registered actions and verification;
- `operations`: units, jobs, orders, resources, structures, alerts;
- `spatial`: required map chunks and spatial overlays;
- `historical`: bounded event/report history;
- `research-full`: maximal supported normalized state.

A projected MCP response is not itself a full canonical snapshot. It references the underlying
anchor and completeness profile.

## 5.11 Raw-field escape hatch

Compatibility work occasionally needs raw DFHack fields. A diagnostic resource may expose
bounded raw records under a separate capability. Raw fields:

- are never accepted directly as semantic action arguments;
- are marked unstable and tainted;
- carry exact DF/DFHack/schema versions;
- are excluded from canonical identity unless a registered migration says otherwise;
- may be omitted from ordinary doctor bundles for size or privacy.

## 5.12 Aggregate facts

Frequently used values such as available drink, edible meals, beds, idle citizens, active
threats, blocked critical jobs, and free hospital capacity are materialized as derived aggregates
with explicit formulas and freshness bounds. They accelerate observation but remain
reconstructible from evidence.

---

# Part VI — Observation, deltas, events, and continuations

## 6.1 Observation modes

`fortress.observe` chooses one of four response modes:

1. **snapshot** — bounded projection from a canonical full snapshot;
2. **delta** — all relevant changes from the supplied anchor to the target anchor;
3. **heartbeat** — no relevant semantic change; current liveness and anchor;
4. **reset** — supplied cursor cannot resume; includes new epoch and snapshot instructions.

The client may request a mode preference but cannot force a delta when continuity evidence is
absent.

## 6.2 Interest sets

An interest set may include:

- entity IDs and generations;
- entity kinds;
- edge kinds;
- field paths;
- map cuboids;
- event classes;
- active plans/actions/obligations;
- query-derived dynamic memberships;
- severity thresholds;
- freshness requirements.

Interests are normalized and assigned stable IDs. Dynamic interests store the query and last
membership anchor so additions/removals are explicit.

## 6.3 Delta algebra

A delta is:

```text
Delta {
    fortress_id
    schema_version
    completeness_profile
    base_anchor
    target_anchor
    ordered_changes[]
    event_window
    evidence[]
    truncated
    continuation
}
```

Change operations include:

```text
upsert_entity(record)
remove_entity(id, expected_generation, expected_revision)
upsert_edge(record)
remove_edge(id, expected_revision)
upsert_chunk(record)
remove_chunk(coord, expected_revision)
append_event(record)
replace_aggregate(record)
```

Applying a complete delta to its exact base must produce the declared target hash. This property
is tested over generated state transitions.

## 6.4 Delta ordering

Within a delta:

1. schema/compatibility notices;
2. removals of edges that would otherwise dangle;
3. entity and chunk removals;
4. entity and chunk upserts;
5. edge upserts;
6. aggregates;
7. events;
8. action/obligation evidence.

The receiver still validates the final graph rather than relying solely on order.

## 6.5 Truncation and continuation

When a result hits a budget, the server returns:

- `truncated: true`;
- a continuation token;
- the same target anchor for every page;
- page index and covered key range;
- cumulative digest commitment;
- expiry and session binding.

Continuation tokens are opaque authenticated capabilities, not client-editable offsets. A client
must finish or abandon a continuation before asking the server to treat the partial projection as
complete. Canonical ledger ingestion occurs before presentation pagination, so pagination does
not split world truth.

## 6.6 Event semantics

Events are immutable observations, not commands. Each event has:

- stable source identity;
- source tick and first/last observed cursor;
- type and severity;
- subject/object references;
- source text and normalized fields;
- deduplication key;
- provenance and compatibility;
- optional causal parents;
- retention class.

Polling overlap is expected. Deduplication uses source-native IDs when reliable and a registered
fingerprint otherwise. Fingerprint collisions produce a diagnostic ambiguity record rather than
silent merging.

## 6.7 Freshness

Every field and aggregate declares a freshness class:

| Class | Intended use |
|---|---|
| `same-frame` | must come from one coherent bridge read transaction |
| `same-tick` | may use several reads while game tick is unchanged |
| `bounded-ticks(n)` | acceptable within `n` game ticks |
| `eventual` | background index or history |
| `static-version` | valid for one compatibility manifest |

A plan’s preconditions declare required freshness. The coordinator requests a refresh rather than
using stale cached state.

## 6.8 Observation consistency

The bridge should offer a read epoch:

1. capture game tick and bridge sequence;
2. read requested structures under supported synchronization;
3. capture game tick and sequence again;
4. accept if coherence policy passes;
5. otherwise retry within budget or return `ERR-UNSTABLE-READ`.

Not all DF data can be globally snapshotted. The compatibility manifest records consistency
strength by field group. The canonical snapshot carries these limits honestly.

## 6.9 Map-delta economics

Map changes are expected to dominate raw volume. Optimizations, in order:

1. interest-set exclusion;
2. chunk revision comparison;
3. content-hash equality;
4. changed bitplane ranges;
5. changed run ranges;
6. sparse overlay updates;
7. spatial summary substitution for distant areas;
8. compression at transport/storage layers.

No optimization may change canonical target reconstruction.

## 6.10 Agent-facing summaries

Summary projection is a deterministic function over canonical state and policy. A default
situation summary should cover:

- anchor and elapsed game time;
- population and critical health;
- food, drink, fuel, and essential reserves;
- active threats and alerts;
- stalled critical jobs;
- top attention items with score ledger;
- active plans and obligations;
- changes since the agent’s last acknowledged cursor;
- uncertainty and compatibility warnings;
- available continuation or drill-down resources.

The summary generator cannot invent causal explanations. Causal language requires evidence graph
support or is labeled hypothesis.

## 6.11 Acknowledgment and retention

Sessions may acknowledge processed cursors. Retention policy considers:

- minimum cursor across active sessions;
- unresolved actions/obligations;
- audit policy;
- checkpoint and replay requirements;
- disk budget.

Slow or abandoned sessions cannot block compaction indefinitely. Policy may expire them, after
which they receive an epoch reset.

# Part VII — Query, attention, explanation, and knowledge

## 7.1 Query philosophy

Agents need a declarative way to ask for semantic facts without learning internal storage
layouts or downloading the entire fortress. Query is read-only, bounded, deterministic at a
fixed anchor, and separate from mutation.

The initial query language, **DfQL**, is a structured schema rather than a free-form text parser.
Natural language may be compiled into DfQL by an agent or optional planner, but the server
executes only the explicit structured form.

## 7.2 DfQL core

DfQL v1 supports:

- entity-kind selection;
- stable ID lookup;
- field projections;
- typed comparisons;
- graph-edge traversal with bounded depth;
- spatial intersection and containment;
- event windows;
- action/obligation state filters;
- deterministic ordering;
- grouping and registered aggregates;
- bounded top-k attention ranking;
- provenance and freshness constraints.

Example:

```json
{
  "from": {"kind": "job"},
  "where": {
    "all": [
      {"field": "state", "op": "eq", "value": "suspended"},
      {"field": "age_ticks", "op": "gte", "value": 1200},
      {"edge": "requires", "target_kind": "item"}
    ]
  },
  "select": [
    "id",
    "job_type",
    "worker",
    "workshop",
    "blocking_requirements",
    "age_ticks"
  ],
  "order_by": [
    {"field": "criticality", "direction": "desc"},
    {"field": "age_ticks", "direction": "desc"},
    {"field": "id", "direction": "asc"}
  ],
  "limit": 40
}
```

## 7.3 Query safety

Every query is statically costed before execution. Cost dimensions include:

- entity scans;
- edge expansions;
- spatial chunks;
- event window;
- index probes;
- result rows;
- provenance expansion;
- output bytes and tokens.

Queries above budget return a cost explanation and suggested narrowing. They are not run
partially unless the query explicitly permits continuation.

Graph traversal requires a maximum depth and frontier bound. Regex, scripting, arbitrary
expressions, and user-defined code are absent from v1.

## 7.4 Search integration

Full-text and semantic search are derived capabilities used for:

- announcements and reports;
- imported manuals and runbooks;
- fortress journal;
- plan/evidence explanations;
- similar historical failures;
- entity labels and descriptions.

The search pipeline is:

```text
normalize query
  → lexical candidates
  → optional semantic candidates
  → deterministic fusion
  → evidence/freshness filtering
  → optional semantic rerank
  → score ledger
  → token-aware result shaping
```

Every result cites the canonical entity/event revision or exact document source span. Embeddings
are cacheable derived artifacts keyed by source digest and model manifest.

## 7.5 Attention model

Attention is a registered family of scores, not an opaque model judgment. Initial score domains:

- survival: food, drink, temperature, air, medical capacity;
- threat: hostile units, siege state, dangerous wildlife, fire, flooding, cave-ins;
- production: blocked critical jobs, missing inputs, idle essential workshops;
- logistics: inaccessible stock, hauling saturation, path failures;
- population: severe stress, unconscious/injured units, labor bottlenecks;
- governance: mandates, demands, justice, diplomacy;
- plan health: missed milestones, stale leases, failed postconditions;
- compatibility: unsupported fields or bridge anomalies.

A score record contains:

```text
score ID and registry version
anchor
subject
normalized score
component contributions
supporting evidence
missing/unknown inputs
freshness
suppression and deduplication decisions
```

The same inputs and registry version must produce the same score.

## 7.6 Explanation

`fortress.explain` accepts a typed subject:

- fact;
- delta;
- query row;
- attention score;
- prepared plan;
- action transition;
- obligation state;
- compatibility decision;
- error;
- checkpoint or restore;
- doctor finding.

Explanation follows evidence edges and registered derivations. It returns:

1. concise conclusion;
2. supporting facts and anchors;
3. decision rule or formula;
4. alternatives rejected;
5. uncertainty or missing data;
6. drill-down resources.

An explanation is not free-form chain-of-thought. It is an externally auditable rationale built
from recorded decision inputs and rules.

## 7.7 Knowledge corpus

Imported knowledge is represented as:

```text
Document
  → exact bytes
  → stable blocks and spans
  → headings, links, code, tables
  → semantic chunks
  → lexical/vector indexes
  → citations
```

FrankenMarkdown is the intended lossless parser. Incremental updates preserve stable IDs where
source spans survive. Each document has trust and taint metadata:

- official DFHack documentation;
- official Dwarf Fortress documentation;
- mod documentation;
- project runbook;
- operator-authored policy;
- agent-authored note;
- in-game text.

Only operator policy files signed or loaded through the policy capability can affect authority.
All other text is informational.

## 7.8 Agent memory

The server may store agent notes, hypotheses, and goals, but they live in a separate epistemic
domain:

- `asserted_by_agent`;
- `inferred`;
- `verified_at_anchor`;
- `falsified`;
- `expired`;
- `superseded`.

Agent memory cannot become canonical world truth without a registered verification step.

---

# Part VIII — Intent, planning, actions, and obligations

## 8.1 Intent contract

An intent contains:

- stable intent ID;
- source state anchor;
- concise objective;
- terminal condition;
- constraints;
- optional requested action skeleton;
- risk ceiling;
- deadline in game ticks;
- optimization preferences;
- explanation depth;
- plan budget.

Intent examples:

- maintain at least 200 drinks and 100 prepared meals through winter;
- establish an iron weapons production line in a specified district;
- evacuate civilians into a safe burrow and lock down access during a siege;
- reduce severe-stress population below five without changing military assignments;
- diagnose why steel production has stopped;
- recover an interrupted construction plan.

The initial static planner accepts explicit semantic action skeletons. Later recipe and search
planners can synthesize action DAGs, but both emit the same prepared-plan contract.

## 8.2 Constraint classes

Constraints are divided into:

1. **safety:** protected entities/areas, no flooding, no magma, no military changes;
2. **resource:** minimum reserves, material exclusions, labor limits;
3. **temporal:** deadline, keep paused, allowed season/window;
4. **authority:** max risk, permitted action domains;
5. **operational:** maximum steps, checkpoint requirement, explanation requirement;
6. **optimization:** minimize travel, material value, disruption, or game ticks.

Hard constraints cannot be traded off. Soft preferences have explicit weights and score ledgers.

## 8.3 Action registry

Each semantic action entry declares:

- stable action kind and schema;
- supported bridge versions;
- minimum risk tier;
- required capability;
- scope extractor;
- precondition generator;
- expected immediate effects;
- semantic postcondition generator;
- temporal/obligation requirement;
- idempotency strategy;
- compensation options;
- lease domains;
- checkpoint policy;
- determinism class;
- recovery/reconciliation method;
- test fixtures.

Initial action families:

| Family | Examples |
|---|---|
| Clock | pause, resume |
| Designation | mine, channel, stairs, ramp, remove construction |
| Construction | place building, construction, furniture, trap |
| Labor | enable/disable registered labor, assignment profile |
| Production | create/update/cancel conditional work order |
| Logistics | stockpile filters/capacity, routes, burrow membership |
| Military | squad membership, alert/schedule assignment under guarded policy |
| Governance | registered standing orders |
| Extension | negotiated typed extension, never arbitrary code |

## 8.4 Risk tiers

### Read-only

No intended game mutation. It may still consume resources or reveal sensitive data.

### Reversible

A registered inverse or compensation generally restores the configuration, and no immediate
irreversible world change is expected. Examples: pause state, labor flag, stockpile setting.

### Guarded

The action may consume resources, create long-running work, expose units to danger, alter terrain,
or be difficult to reverse. Checkpoints and confirmation may be required.

### Irreversible

The action is expected to cause destruction, permanent loss, dangerous release, save replacement,
or other effects for which compensation is not credible. Irreversible actions are absent from
the default initial registry and require explicit policy plus human confirmation.

Context may elevate risk. Digging a normal stone corridor may be guarded; digging a tile adjacent
to pressurized magma may be irreversible or refused.

## 8.5 Plan compilation

Compilation phases:

1. verify intent and anchor;
2. resolve references;
3. load registry and compatibility manifests;
4. refresh required fields to declared freshness;
5. normalize requested actions;
6. infer affected scopes;
7. check hard constraints;
8. expand recipes and dependencies;
9. calculate preconditions and postconditions;
10. attach obligations;
11. calculate risk and capabilities;
12. detect conflicts and protected-scope intersections;
13. calculate checkpoint and lease requirements;
14. predict semantic diff and resource impact;
15. topologically sort with deterministic tie-breaking;
16. generate idempotency keys;
17. seal plan digest;
18. persist plan and explanation.

A plan never contains raw UI coordinates, arbitrary command strings, or unresolved names.

## 8.6 Plan identity and sealing

The canonical plan digest covers:

- intent ID and source anchor;
- registry/schema versions;
- normalized actions;
- step IDs and dependencies;
- scopes and leases;
- preconditions/postconditions;
- obligations;
- risk and capability requirements;
- checkpoint policy;
- predicted semantic diff;
- expiry;
- planner manifest and explicit seed;
- hard constraints and satisfied soft-score ledger.

The plan ID is content-derived or bound to the digest. Prepare receipts name both.

## 8.7 Prepare protocol

`fortress.plan` produces a plan but does not mutate the game.

`fortress.commit` begins by preparing:

1. load the exact plan digest;
2. verify expiry and session ownership;
3. verify capabilities and confirmation seals;
4. acquire or reserve leases;
5. refresh affected state;
6. re-evaluate preconditions and constraints;
7. verify bridge compatibility;
8. create and durably verify checkpoint if required;
9. ask the bridge to prepare typed actions;
10. persist bridge prepare token and revalidated anchor.

Any change invalidating a hard condition returns a structured plan conflict. The server may offer
a deterministic rebase proposal, but it does not silently alter and commit the plan.

## 8.8 Commit protocol

For each ready step:

1. persist dispatch intent and idempotency key;
2. send typed action with prepare token and fencing tokens;
3. receive or time out;
4. persist receipt;
5. refresh affected state;
6. classify immediate effect;
7. if postconditions hold and no obligation exists, verify;
8. otherwise register/advance obligation;
9. unlock dependents only after required predecessor states;
10. emit progress evidence.

Batches are used only where the bridge can guarantee the declared atomicity. A plan-level
transaction does not pretend Dwarf Fortress can roll back arbitrary game simulation.

## 8.9 Idempotency

Idempotency key scope is `(fortress lineage, action registry version, semantic action content,
plan step identity)`. The ledger records:

- key;
- canonical request digest;
- bridge prepare token digest;
- dispatch sequence;
- receipt;
- observed effects;
- terminal state.

Rules:

- same key + same content before dispatch: resume;
- same key + same content after receipt: return prior receipt;
- same key + different content: conflict;
- timeout after possible dispatch: indeterminate and reconcile;
- adapter confirms no dispatch under key: safe retry;
- adapter confirms prior dispatch: attach prior bridge operation and observe;
- adapter lacks idempotency evidence: pause or operator policy before retry.

## 8.10 Postconditions

Postconditions are typed predicates. Examples:

- pause state equals requested state;
- designation exists over exact tile mask with expected mode;
- building entity exists at footprint and references the plan action;
- labor setting for exact unit generations equals requested value;
- work order with normalized conditions exists and is active;
- stockpile filter revision equals expected digest;
- squad membership edge exists.

A postcondition may require several fields and compatibility guarantees. Unknown required facts do
not count as false or true; they block verification.

## 8.11 Bounded obligations

Actions such as mining, construction, hauling, training, and production create obligations.

An obligation includes:

```text
obligation_id
owner region and session
action_id
source anchor
terminal predicate
failure predicate
deadline game tick
minimum poll interval
required stable observations
dependencies
budget
cancellation strategy
evidence chain
state
```

Example: “construct metalsmith’s forge” may require:

- designated/placed building exists;
- construction job was created;
- job is not suspended or cancelled;
- building reaches complete state;
- expected footprint remains;
- stable state observed twice;
- deadline has not passed.

## 8.12 Obligation scheduling

The coordinator does not poll every obligation every frame. It uses:

- event subscriptions;
- field-interest unions;
- next-relevant game-tick estimates;
- exponential backoff under inactivity;
- immediate wakeup on subject changes;
- global bridge/read budgets;
- priority based on risk and blocking dependencies.

Poll schedules are deterministic given the event stream and policy. Starvation prevention is
explicit.

## 8.13 Blocked obligations

A blocked obligation carries a structured blocker set:

- missing material;
- inaccessible path;
- unavailable worker;
- workshop unavailable;
- dangerous condition;
- mandate or policy restriction;
- incompatible state;
- unknown/unsupported fact;
- dependency incomplete.

The coordinator may propose a child plan to resolve blockers, but that plan requires separate
authorization and retains a causal edge to the parent obligation.

## 8.14 Cancellation

Cancellation phases:

### Request

- mark action/obligation `cancel_requested`;
- prevent future dependent dispatch;
- send bridge stop request where supported;
- optionally pause under policy;
- retain leases needed for safe drain.

### Drain

- wait for in-flight bridge requests;
- observe whether the game accepted or progressed the work;
- cancel queued jobs/designations where authorized;
- resolve receipts;
- determine compensation eligibility.

### Compensate

- compile compensation as a new plan;
- authorize and checkpoint as required;
- commit and verify it.

### Finalize

- transition to `cancelled`, `compensated`, `failed`, or `indeterminate`;
- release leases;
- persist final evidence;
- notify dependents and session.

Dropping the MCP request does not bypass this lifecycle.

## 8.15 Reconciliation of indeterminate effects

Reconciliation strategies, in order:

1. query bridge idempotency journal;
2. inspect normalized target state for unique expected markers;
3. compare pre/post checkpoint manifests where appropriate;
4. correlate jobs/events/announcements;
5. run registered action-specific reconciliation;
6. request operator decision with evidence;
7. restore checkpoint only under explicit policy.

Reconciliation may supersede `indeterminate` with a new fact while preserving the original state
and explanation.

## 8.16 Plan rebase

A stale plan can be rebased only by producing a new plan:

- identify changed facts;
- classify irrelevant, compatible, or conflicting;
- rerun compilation against new anchor;
- preserve intent identity but generate new plan ID/digest;
- explain changed steps, risk, scopes, and obligations;
- require fresh prepare/commit.

No plan digest survives a rebase.

---

# Part IX — DFHack bridge

## 9.1 Boundary requirements

The bridge MUST:

- communicate through a versioned bounded protocol;
- report DF, DFHack, bridge, plugin, mod, and schema manifests;
- expose registered semantic read groups;
- expose registered typed action messages;
- support idempotency lookup or clearly declare its absence;
- include game tick/sequence evidence;
- enforce size/depth/count limits;
- reject unknown required fields;
- never accept arbitrary evaluator or command strings on the production service;
- return stable bridge error codes and structured diagnostics;
- maintain a bounded operation journal long enough for reconciliation.

## 9.2 Transport

Initial transport candidates:

1. DFHack remote RPC service over local TCP;
2. local Unix domain socket or named pipe from a bridge sidecar;
3. authenticated loopback stream using length-prefixed protobuf frames.

The selected transport must support:

- connection authentication or local peer verification;
- handshake before requests;
- bounded frame size;
- request IDs;
- cancellation;
- deadlines;
- keepalive/health;
- backpressure;
- no implicit retries of mutating requests.

MCP transport and bridge transport are independent.

## 9.3 Handshake

The bridge handshake returns:

```text
bridge protocol versions
schema versions
DF version/build
DFHack version/commit
platform
loaded mods and fingerprints where available
available read groups
available action kinds
field support matrix
consistency strengths
idempotency support
max frame and batch limits
semantic probe results
bridge process instance ID
game process instance ID
current fortress identity candidate
```

The server compares this with a signed or checked-in compatibility manifest.

## 9.4 Read batches

A read request contains:

- read epoch request;
- field-group masks;
- entity selectors;
- spatial selectors;
- event cursor;
- maximum counts/bytes/time;
- required freshness and consistency;
- continuation token.

A read response contains:

- bridge instance and sequence;
- begin/end game ticks;
- coherence status;
- normalized-or-raw typed records;
- omissions and unsupported fields;
- events;
- continuation;
- checksums and diagnostics.

Early bridge versions may send source-oriented records to the Rust normalizer. Mature versions may
normalize more locally, but canonical semantics remain defined in Rust schemas.

## 9.5 Typed action batches

Bridge action messages are a closed `oneof` or equivalent tagged union. Every message includes:

- protocol/action schema;
- bridge prepare token;
- idempotency key;
- expected game/bridge instance;
- expected tick window or source revision;
- fencing tokens;
- typed action body;
- bounds;
- dry-run/prepare/commit mode.

The bridge validates again immediately before touching game state.

## 9.6 Game-thread execution

DFHack APIs may require game-thread execution. The bridge queues bounded work and returns:

- queued;
- executing;
- applied;
- rejected before effect;
- failed after partial effect;
- unknown/timeout.

The bridge must not call something `applied` merely because it entered the queue.

## 9.7 Bridge operation journal

The bridge keeps a bounded journal keyed by idempotency key:

```text
request digest
prepare token digest
first/last seen time
game instance
dispatch state
native operation identity
receipt
observed immediate markers
```

Journal loss or restart is declared in the handshake. The Rust coordinator then treats unresolved
operations conservatively.

## 9.8 Compatibility probes

Structural version strings are insufficient. Probes validate semantics such as:

- stable fortress/site identity;
- unit ID/generation behavior;
- pause-state read/write;
- entity revision derivation;
- designation creation and readback in a disposable fixture;
- work-order condition round-trip;
- map coordinate conventions;
- event deduplication behavior;
- save path and checkpoint visibility.

Mutation probes run only in certified test fortresses.

## 9.9 Degraded modes

Compatibility levels:

- **exact:** all required schemas/probes match certified manifest;
- **compatible:** known translation with passing probes;
- **degraded read-only:** safe observation subset; mutations disabled;
- **unknown:** diagnostic raw reads only under capability;
- **incompatible:** refuse session beyond doctor.

Compatibility can be per action family. Labor might remain safe while military actions are
disabled.

## 9.10 UI and screenshot fallback

A diagnostic subsystem may capture screenshots, inspect UI state, or drive limited navigation
when structured support is absent. Rules:

- results are `visual_evidence`, not canonical truth;
- no guarded mutation depends solely on vision;
- UI coordinates are tied to exact version, resolution, scaling, and layout manifest;
- every fallback action is visibly marked and separately authorized;
- production certification does not count fallback behavior as semantic bridge coverage.

---

# Part X — Persistence, checkpoints, recovery, and compaction

## 10.1 Ledger responsibilities

The durable ledger stores:

- protocol/schema/compatibility manifests;
- fortress lineage and instance identities;
- sessions and capability grants;
- canonical snapshots and deltas;
- entity generation map;
- events and evidence;
- intents and prepared plans;
- action state machines and idempotency;
- obligations;
- leases and fencing tokens;
- bridge handshakes and operation receipts;
- checkpoint manifests;
- doctor findings and repair plans;
- migrations and compaction proofs.

## 10.2 Transaction boundaries

Key transactions:

### Ingest observation

Atomically persist:

- source bridge frame digest;
- normalized delta/snapshot;
- resulting anchor;
- compatibility notices;
- events;
- evidence;
- active-obligation wakeups.

### Prepare plan

Atomically persist:

- exact plan digest;
- source anchor;
- required capabilities and scopes;
- prepare state;
- leases/reservations;
- checkpoint requirement;
- bridge prepare receipt.

### Dispatch action

Before bridge send, persist dispatch intent and idempotency key. After receipt, persist receipt and
state transition before acknowledging durable progress.

### Verify action

Atomically persist:

- supporting target anchor;
- predicate evaluation;
- evidence links;
- terminal action state;
- obligation discharge;
- dependent-step readiness.

## 10.3 FrankenSQLite integration

The intended adapter uses:

- MVCC snapshots for concurrent readers;
- WAL for durable action/observation transactions;
- checksummed pages/frames;
- deterministic VFS and fault injection in tests;
- explicit busy/backpressure policy;
- online indexes for entity kinds, revisions, events, plans, obligations, and leases;
- migration transactions;
- snapshot/backup hooks coordinated with checkpoints.

No code may assume SQLite C API behavior; integration targets FrankenSQLite’s Rust contracts.

## 10.4 Checkpoint protocol

A checkpoint consists of:

```text
checkpoint ID
fortress lineage
source anchor
DF/DFHack/bridge manifests
save source path capability
file manifest: relative path, size, digest, metadata class
copy/clone method
write ordering record
completion seal
ledger snapshot reference
evidence
```

Protocol:

1. acquire checkpoint and clock/file leases;
2. reach supported save-safe state;
3. request or verify game save;
4. freeze source manifest;
5. clone/copy into staging through scoped filesystem capability;
6. hash and fsync files/directories in defined order;
7. persist ledger/checkpoint association;
8. atomically publish manifest and seal;
9. release leases.

A directory existing is not proof of a complete checkpoint.

## 10.5 FrankenFS integration

FrankenFS is intended to provide:

- injected filesystem/block effects;
- clone-on-write when supported;
- deterministic torn-write and crash campaigns;
- content-addressed manifests;
- doctor bundles;
- repair plan/apply separation;
- bounded event rings;
- path and capability discipline.

Host filesystem support remains behind the same trait for portability.

## 10.6 Restore protocol

Restore is guarded and disruptive:

1. verify checkpoint seal and compatibility;
2. stop new actions;
3. cancel/drain active work;
4. acquire global fortress and clock leases;
5. checkpoint current state if policy requires;
6. stop or coordinate game process;
7. materialize save through staged atomic replacement;
8. restart/reload;
9. re-handshake bridge;
10. create new observation epoch;
11. ingest full canonical snapshot;
12. reconcile lineage and active sessions;
13. persist restore evidence.

Active plans from the prior epoch become stale. They are not silently resumed.

## 10.7 Crash recovery

Recovery scans durable records and reconstructs:

- last valid canonical anchor;
- bridge instance continuity;
- prepared but undispatched actions;
- dispatch intents without receipts;
- receipts without semantic verification;
- active obligations and timers;
- granted leases and expiries;
- incomplete checkpoints;
- pending doctor repair plans.

Classification:

| Durable evidence | Recovery result |
|---|---|
| prepare only, no dispatch intent | safe to expire/reprepare |
| dispatch intent, bridge confirms no send | safe retry |
| dispatch intent, matching receipt | resume verification |
| dispatch intent, no receipt, bridge journal match | attach and reconcile |
| dispatch intent, no receipt, journal lost | indeterminate |
| verified terminal evidence | immutable terminal |
| incomplete checkpoint staging | quarantine and diagnose |
| corrupt ledger frame | stop affected domain and doctor |

## 10.8 Compaction

Compaction may:

- merge deltas into a new canonical snapshot;
- prune superseded non-audit projections;
- rebuild indexes;
- summarize old event ranges;
- archive doctor bundles;
- remove expired continuation state.

It must preserve:

- current anchor proof;
- entity generation history required to prevent ABA;
- active and terminal idempotency records per retention policy;
- unresolved actions/obligations;
- checkpoint lineage;
- required audit evidence;
- migration history.

Each compaction emits a proof record naming input ranges, output digest, and retained roots.

## 10.9 Backup and export

An export bundle contains versioned canonical data and evidence, not implementation-specific
database pages alone. It includes:

- manifest and checksums;
- schema registry;
- snapshots/deltas;
- plans/actions/obligations;
- compatibility and source manifests;
- optional save checkpoints;
- optional indexes clearly marked rebuildable;
- redaction policy.

## 10.10 Doctor and repair

Doctor checks are pure where possible. Repairs follow:

```text
doctor.scan
  → findings
  → repair.plan
  → operator/agent inspection
  → repair seal
  → repair.apply with revalidation
  → verification
```

A repair plan lists exact expected digests and effects. If state changes, apply refuses and a new
plan is required.

---

# Part XI — MCP protocol surface

## 11.1 Protocol principles

The MCP surface is:

- small;
- semantic;
- version-negotiated;
- capability-scoped;
- budgeted;
- cursor-anchored;
- continuation-aware;
- evidence-bearing;
- explicit about partial and indeterminate states.

The server supports the current MCP lifecycle and transport model selected at implementation
time, but `dfmcp` semantics are versioned independently.

## 11.2 Common request envelope

Every tool request logically includes:

```json
{
  "dfmcp_version": "0.x",
  "session_id": "session:…",
  "request_id": "request:…",
  "expected_anchor": {
    "fortress_id": "fortress:…",
    "cursor": {"epoch": 1, "sequence": 42},
    "state_hash": "sha256:…"
  },
  "budget": {
    "wall_millis": 2000,
    "game_ticks": 10000,
    "entities": 2000,
    "bytes": 4194304,
    "output_tokens": 1500,
    "actions": 64
  }
}
```

Read-only tools may allow `expected_anchor: latest` under explicit policy, but return the concrete
anchor used. Mutations never commit against an unspecified anchor.

## 11.3 Common response envelope

```json
{
  "dfmcp_version": "0.x",
  "session_id": "session:…",
  "request_id": "request:…",
  "anchor": {
    "fortress_id": "fortress:…",
    "cursor": {"epoch": 1, "sequence": 43},
    "game_tick": 9120031,
    "state_hash": "sha256:…"
  },
  "result": {},
  "evidence": [],
  "warnings": [],
  "truncated": false,
  "continuation": null
}
```

Errors use stable `ERR-*` codes, retry classification, affected scope, current anchor when known,
and recovery advice.

## 11.4 `fortress.open_session`

Inputs:

- requested protocol versions;
- transport/client metadata;
- fortress selector;
- requested capabilities and scopes;
- desired completeness/observation profile;
- budgets;
- optional authentication/delegation token.

Outputs:

- negotiated versions;
- session ID;
- fortress identity and initial anchor;
- compatibility level and disabled features;
- granted capabilities;
- hard budgets;
- available tools/resources/prompts;
- bridge and schema manifests;
- initial situation summary or continuation.

## 11.5 `fortress.observe`

Inputs:

- since anchor/cursor;
- interest set;
- projection;
- freshness;
- limits;
- continuation.

Outputs:

- snapshot, delta, heartbeat, or reset;
- canonical target anchor;
- evidence and compatibility warnings;
- continuation.

## 11.6 `fortress.query`

Inputs:

- anchor;
- DfQL;
- required freshness;
- score/explanation options;
- limits and continuation.

Outputs:

- deterministic rows;
- matched count;
- score ledger;
- evidence;
- continuation.

## 11.7 `fortress.plan`

Inputs:

- anchor;
- semantic intent;
- constraints;
- requested action skeleton or planner mode;
- optimization preferences;
- risk ceiling;
- plan budget.

Outputs:

- immutable plan ID/digest;
- source anchor and expiry;
- action DAG;
- predicted semantic diff;
- capabilities and leases;
- risk analysis;
- checkpoint policy;
- obligations;
- alternatives and rejected reasons;
- explanation/evidence.

No game effect occurs.

## 11.8 `fortress.commit`

Inputs:

- exact plan ID and digest;
- expected anchor;
- idempotency key for the plan-level request;
- confirmation seal if required;
- commit mode: ready steps, specified steps, or atomic bridge batch where supported.

Outputs:

- prepare receipt;
- checkpoint receipt;
- per-step action receipts;
- current anchor;
- active obligations;
- blocked/conflicted steps;
- evidence.

A timeout may return an action as indeterminate rather than a generic tool error.

## 11.9 `fortress.wait`

Inputs:

- action/obligation/plan IDs;
- stop conditions;
- maximum wall time and game ticks;
- progress threshold;
- output-token budget;
- whether the session holds clock authority.

Outputs:

- only semantically relevant deltas;
- obligation transitions;
- blocker changes;
- terminal evidence;
- continuation.

`wait` is not an unbounded blocking call. It can return progress and a continuation.

## 11.10 `fortress.cancel`

Inputs:

- target plan/action/obligation;
- mode: stop future steps, compensate reversible, emergency pause and drain;
- reason;
- confirmation where required.

Outputs:

- cancellation phase;
- stopped and in-flight effects;
- compensation plan if any;
- final or pending state;
- evidence.

## 11.11 `fortress.checkpoint`

Inputs:

- label;
- source anchor;
- scope/profile;
- durability level;
- reason.

Outputs:

- checkpoint ID;
- manifest/seal digest;
- anchor;
- durability and compatibility;
- evidence.

## 11.12 `fortress.restore`

Inputs:

- checkpoint ID and seal;
- current expected anchor;
- active-work policy;
- confirmation seal.

Outputs:

- prior and restored anchors;
- new epoch;
- invalidated plans/sessions;
- reconciliation findings;
- evidence.

## 11.13 `fortress.explain`

Inputs:

- typed subject;
- depth;
- evidence budget;
- include alternatives;
- include source citations.

Outputs follow the explanation model in Part VII.

## 11.14 `fortress.doctor`

Inputs:

- domains to inspect;
- depth;
- include raw diagnostics;
- bundle policy.

Outputs:

- health summary;
- findings with severity and stable IDs;
- earliest divergence;
- safe actions;
- sealed repair plan candidates;
- doctor bundle resource.

## 11.15 MCP resources

Proposed resources:

```text
df://session/{session}/summary
df://session/{session}/capabilities
df://fortress/{fortress}/anchor
df://fortress/{fortress}/entity/{entity}
df://fortress/{fortress}/map/chunk/{x}/{y}/{z}
df://fortress/{fortress}/events
df://fortress/{fortress}/plans/{plan}
df://fortress/{fortress}/actions/{action}
df://fortress/{fortress}/obligations/{obligation}
df://fortress/{fortress}/checkpoints/{checkpoint}
df://fortress/{fortress}/compatibility
df://knowledge/{document}/{span}
df://doctor/{bundle}
```

Resources are still capability-checked and budgeted.

## 11.16 MCP prompts

Prompts are optional workflow aids, not authority:

- diagnose a production bottleneck;
- propose a safe excavation plan;
- review a prepared plan adversarially;
- summarize fortress changes;
- triage active obligations;
- plan siege lockdown;
- reconcile an indeterminate action.

Prompts produce tool calls or structured intent; they cannot bypass plan/commit.

## 11.17 Progress and notifications

The server emits bounded notifications for:

- anchor advancement;
- high-severity events;
- plan-step transitions;
- obligation blocker/terminal changes;
- lease expiry/revocation;
- compatibility degradation;
- checkpoint completion;
- doctor critical findings.

Clients acknowledge cursors to control retention.

---

# Part XII — Concurrency, leases, delegation, and multi-agent operation

## 12.1 Concurrency model

Many sessions may read at MVCC anchors. Writes are coordinated by semantic scopes rather than a
single global mutex, except for operations that inherently require global exclusivity such as
restore or certain clock transitions.

## 12.2 Lease domains

Lease types:

- entity generation and field domain;
- map cuboid and action class;
- stock/resource reservation lot;
- configuration domain such as labor, standing orders, military schedule;
- workshop/work-order namespace;
- plan/obligation coordinator ownership;
- simulation clock;
- checkpoint/save;
- fortress-global maintenance.

A plan computes required leases. Leases can be acquired in canonical order to prevent deadlock.

## 12.3 Lease record

```text
lease_id
fortress lineage
scope
mode: read-reservation/write/exclusive
owner session/plan
fencing token
granted anchor
expiry wall/monotonic/game tick as appropriate
renewal budget
delegation parent
state
evidence
```

Wall/monotonic expiry protects infrastructure liveness; game-tick expiry protects semantic plans.
Both may be present.

## 12.4 Fencing

Every effect includes relevant fencing tokens. The ledger and bridge reject a token lower than
the current scope fence. Transfer increments the fence before the new owner acts. This prevents a
paused or partitioned old coordinator from mutating after lease expiry.

## 12.5 Conflict detection

Scopes conflict when:

- map regions overlap and effects are not registered commutative;
- entity field domains overlap;
- resource reservations exceed available quantity;
- one plan changes a dependency of another;
- clock/checkpoint/global operations overlap;
- compatibility or schema transition invalidates both.

Conflict responses name the holder, scope, expiry, and safe options: wait, narrow, reorder,
delegate, or replan.

## 12.6 Commutativity registry

Some actions commute:

- labor settings on disjoint unit generations;
- stockpile settings on distinct stockpiles;
- designations on disjoint tile masks;
- read-only observations.

Some appear disjoint but do not safely commute because they compete for dwarves, materials,
paths, or global configuration. The registry must be conservative and evidence-backed. Unknown
pairs conflict.

## 12.7 Delegation

A parent agent may delegate:

- a subset of capabilities;
- narrower entities/areas;
- lower risk;
- shorter expiry;
- smaller budgets;
- limited uses;
- specified plan/obligation ownership.

Delegation tokens are bound to parent, child identity, session, fortress, and policy version.
Revocation propagates. Delegation cannot broaden authority.

## 12.8 Multi-agent roles

A recommended architecture is a coordinating agent with specialists:

- observer/analyst;
- architect/spatial planner;
- production/logistics planner;
- military planner;
- health/welfare monitor;
- diagnostician/recovery agent.

Specialists prepare plans and explanations. The coordinator or policy decides which plans receive
leases and commit authority. This preserves independent analysis without concurrent mutation
chaos.

## 12.9 Clock control

Clock control is special. Agents may:

- observe without controlling time;
- request pause;
- request bounded unpause until stop condition;
- hold an exclusive clock lease;
- participate in a coordinator that aggregates stop conditions.

No agent may unpause indefinitely through `wait`. A bounded run declares maximum game ticks,
wall time, and emergency conditions.

## 12.10 Fairness and starvation

Lease scheduler inputs:

- risk/severity;
- blocker criticality;
- wait age;
- session weight;
- deadline;
- resource cost;
- cancellation state.

Scheduling is deterministic for equal inputs. High-priority emergency operations may preempt
renewable leases but must emit evidence and fencing transitions.

# Part XIII — Franken integration architecture

## 13.1 Integration doctrine

The sibling projects are not mandatory runtime dependencies by slogan. Each integration must
satisfy three rules:

1. `dwarf_fortress_mcp` defines a narrow semantic trait first.
2. A deterministic reference implementation exists for tests.
3. The Franken adapter is admitted only after compatibility, failure, performance, and recovery
   gates pass.

This avoids circular development and semantic leakage while preserving the option to compose the
best mechanisms.

## 13.2 `asupersync`

### Intended use

- root runtime and structured regions;
- `Cx`-style operation contexts;
- deadlines and multidimensional budgets;
- cancellation request/drain/finalize;
- action and obligation ownership;
- virtual time;
- schedule exploration and deterministic replay;
- effect fault injection;
- optional ATP transport for remote workers or replicated observation services.

### Region tree

```text
server
├─ bridge-supervisor
├─ ledger-supervisor
├─ index-supervisor
└─ session
   ├─ observation-stream
   ├─ query-workers
   ├─ plan-compiler
   └─ committed-plan
      ├─ lease-renewer
      ├─ checkpoint-operation
      ├─ action-step
      │  ├─ bridge-request
      │  └─ verification-obligation
      └─ evidence-writer
```

A parent cannot finish while owned children retain unresolved obligations. Shutdown has an
explicit budget and produces a terminal drain report.

### Required acceptance evidence

- no orphan tasks under cancellation at every injected yield point;
- deterministic action FSM under virtual time;
- bridge timeout and delayed-receipt schedules;
- budget propagation and exhaustion;
- cancellation reason preservation;
- no successful session close with active unowned obligation.

## 13.3 `frankensqlite`

### Intended use

Tables/logical domains:

```text
manifests
fortress_lineages
bridge_instances
sessions
capability_grants
anchors
canonical_snapshots
canonical_deltas
entities_current
entity_generations
events
evidence
intents
plans
plan_steps
idempotency
actions
obligations
leases
checkpoints
doctor_findings
migrations
compaction_proofs
```

The canonical append log and current projections should be separate. Rebuildable indexes may be
discarded and regenerated.

### Required acceptance evidence

- WAL crash campaign at every ledger transaction boundary;
- duplicate idempotency requests under process death;
- receipt persistence ordering;
- MVCC read consistency during delta ingestion;
- migration rollback;
- corruption localization;
- deterministic VFS replay;
- sustained write/read benchmark under active fortress change rates.

## 13.4 `frankenfs`

### Intended use

- save and checkpoint filesystem capability;
- staging and atomic publish;
- clone-on-write or copy strategy;
- content manifests and seals;
- evidence/doctor bundles;
- deterministic filesystem fault campaigns;
- repair plan/apply.

### Required acceptance evidence

- path traversal and symlink attack corpus;
- crash at every copy/fsync/rename boundary;
- incomplete checkpoint never advertised complete;
- manifest mismatch localization;
- restore into a new observation epoch;
- sealed repair refusal after state drift;
- large-save performance and disk-amplification measurements.

## 13.5 `frankensearch`

### Intended use

Indexes:

- entities and fields;
- events/announcements/reports;
- action and obligation evidence;
- doctor findings;
- manuals/runbooks/journals;
- attention candidates;
- similar-failure retrieval.

### Required acceptance evidence

- deterministic top-k under fixed model/index manifest;
- exact citation to canonical revision/source span;
- score ledger reconstruction;
- stale-document invalidation;
- token-budget result shaping;
- malicious/oversized text corpus;
- index rebuild equivalence;
- fallback lexical behavior when embeddings unavailable.

## 13.6 `franken_markdown`

### Intended use

- official and community documentation;
- mod docs;
- project policy/runbooks;
- agent playbooks and journals;
- exact source citations;
- incremental corpus updates.

### Required acceptance evidence

- exact byte/span preservation;
- incremental/full parse equivalence;
- stable citation IDs across local edits;
- prompt-injection taint preservation;
- no executable authority from document content;
- malformed/adversarial Markdown corpus.

## 13.7 `frankengraphdb`

### Intended use

The canonical world model is graph-shaped from day one, but the first implementation may use
in-memory ordered maps and FrankenSQLite tables. FrankenGraphDB becomes appropriate for:

- multi-hop relationship queries;
- provenance and causal traversal;
- historical graph snapshots;
- graph/vector hybrid retrieval;
- multi-fortress research corpora;
- agent-operation/world-state joint graphs.

### Required acceptance evidence

- identity and MVCC semantics match the canonical contract;
- exact snapshot-anchor query;
- deterministic traversal order;
- bounded graph expansion;
- provenance retention;
- migration from reference graph without semantic drift;
- performance advantage on representative workloads.

## 13.8 ATP and remote topology

ATP is optional for:

- remote bridge relays;
- distributed read/index workers;
- evidence bundle transfer;
- research cluster replay;
- multi-host observation fanout.

Mutation authority remains single-coordinator per fortress scope. Partial reliability is never
used to send an effect whose loss/duplication semantics are not covered by idempotency and
reconciliation.

## 13.9 Dependency minimization

Core crates should remain standard-library-only where practical. Adapter crates may depend on
specific sibling projects. Feature flags must not create different semantics for the same
protocol version. If an optional adapter cannot meet the contract, the feature is unsupported,
not silently degraded.

---

# Part XIV — Security and threat model

## 14.1 Assets

Protected assets include:

- game/save integrity;
- fortress availability;
- operator filesystem and credentials;
- bridge and server host integrity;
- MCP authentication and capabilities;
- private game or agent data;
- action/idempotency ledger;
- checkpoints and evidence;
- compatibility manifests;
- agent compute and token budgets.

## 14.2 Adversaries and failures

The design considers:

- malicious MCP client;
- compromised or hallucinating agent;
- prompt injection in in-game text or imported docs;
- malicious mod data;
- buggy or compromised bridge;
- malformed protobuf/JSON;
- stale/replayed client;
- concurrent agent races;
- local unprivileged process;
- operator mistake;
- disk corruption;
- process or host crash;
- version drift;
- resource-exhaustion attack.

## 14.3 Capability model

Capabilities are unforgeable authenticated records or server-side grants. The initial registry
includes:

```text
observe
query
plan
designate
construct
configure_labor
configure_production
configure_logistics
configure_military
control_clock
checkpoint
restore
extension
diagnostic_raw
doctor
repair_plan
repair_apply
admin
```

A grant includes:

- subject;
- fortress;
- action class;
- entity IDs/generations;
- map cuboids;
- resource/configuration domains;
- maximum risk;
- expiry;
- use count;
- budgets;
- delegation parent;
- policy version.

Checks occur at MCP intake, plan preparation, lease acquisition, checkpoint, bridge dispatch,
compensation, restore, and repair.

## 14.4 Confirmation seals

High-risk policy may require a confirmation seal bound to:

- exact plan digest;
- source anchor;
- risk explanation digest;
- expiry;
- operator identity;
- allowed commit scope.

A plain “yes” string is insufficient. Any rebase changes the plan digest and invalidates the seal.

## 14.5 Prompt-injection defense

All text from:

- unit names and nicknames;
- announcements;
- books and engravings;
- mod descriptions;
- imported web/docs;
- agent notes;

is tainted data. It may be displayed, searched, summarized, or cited. It cannot:

- alter system prompts or policy;
- grant a capability;
- select an extension namespace;
- cause arbitrary command execution;
- approve a plan;
- expand a map/entity scope;
- suppress security warnings.

Tool arguments are built from typed schemas, not interpolated text commands.

## 14.6 Bridge distrust

The Rust server validates bridge output:

- frame sizes and counts;
- enum/schema versions;
- coordinate bounds;
- string lengths and encoding;
- duplicate IDs;
- revision monotonicity;
- entity/edge integrity;
- digest format;
- game/bridge instance continuity;
- semantic probe expectations.

Bridge facts retain source provenance. A bridge cannot self-declare an unknown version
“compatible” without matching a server manifest and probes.

## 14.7 Filesystem confinement

Save/checkpoint access uses directory capabilities and relative paths. Rules:

- reject absolute paths from clients/bridge;
- resolve and verify every component;
- reject traversal;
- define symlink policy explicitly;
- stage under controlled roots;
- use no ambient current directory;
- bound file count, individual size, and total bytes;
- prevent special-device and socket inclusion;
- redact secrets from bundles.

## 14.8 Network posture

Default:

- MCP stdio or authenticated localhost;
- bridge bound to localhost/private pipe;
- no outbound network;
- no remote mutation.

Remote mode requires explicit authentication, encryption, replay protection, request limits, and
separate policy. Network reachability never implies capability.

## 14.9 Denial of service

Limits apply to:

- sessions;
- in-flight requests;
- frame and message size;
- strings;
- graph depth/frontier;
- spatial area;
- events;
- plans/steps;
- obligations;
- evidence expansion;
- continuations;
- checkpoint bytes;
- search candidates;
- doctor bundle size.

Budget exhaustion returns a stable error and partial evidence where safe. It does not crash or
silently drop owned work.

## 14.10 Secret handling

Secrets are never accepted in ordinary tool arguments or persisted in transcripts. Authentication
tokens use dedicated providers and are redacted by structured logging. Doctor bundles have a
manifested redaction pass.

## 14.11 Audit

Mutating audit records contain:

- authenticated subject and delegation chain;
- session/request IDs;
- source anchor;
- plan/action/idempotency IDs;
- capability and lease evidence;
- bridge receipt;
- postcondition evidence;
- checkpoint/compensation;
- final state.

Audit records are append-only under retention policy.

## 14.12 Security acceptance

Before enabling a mutation family:

- threat-model review;
- capability scope tests;
- injection corpus;
- malformed payload corpus;
- replay/idempotency tests;
- bridge compromise simulations;
- path and size attacks where applicable;
- least-authority review;
- denial-of-service benchmark;
- documented residual risk.

---

# Part XV — Determinism, replay, verification, and testing

## 15.1 Determinism classes

Every operation is classified:

| Class | Meaning |
|---|---|
| `D0 Pure` | same values produce same values; no effects |
| `D1 Injected` | deterministic given explicit effect transcript |
| `D2 Canonicalized` | external nondeterminism normalized into stable ordering/IDs |
| `D3 Observational` | depends on game state but records sufficient source evidence |
| `D4 Stochastic-planned` | uses explicit seed/model manifest; decision replayable |
| `D5 Non-replayable-external` | cannot be replayed; must be isolated and declared |

Core planning and transition logic target D0/D1. Bridge reads are D3. LLM-generated optional
planning is D4 and never bypasses deterministic validation.

## 15.2 Replay record

A replay contains:

- implementation and schema manifests;
- initial canonical snapshot;
- policy/capability manifests;
- requests;
- injected time;
- bridge/storage/filesystem/transport receipts;
- fault decisions;
- planner seeds/model manifests;
- expected transitions, anchors, digests, and outputs.

Sensitive content may be redacted with commitments, but redaction can limit replay claims.

## 15.3 Deterministic laboratory

The lab provides:

- virtual wall/monotonic/game time;
- reference world state;
- scripted game transitions;
- deterministic bridge;
- in-memory ledger;
- checkpoint store;
- controllable yields;
- fault schedules;
- transcript minimization;
- state-machine invariant checks.

The phase-zero `MemoryAdapter` is a seed, not the final lab.

## 15.4 Reference fortress emulator

A minimal semantic emulator models enough behavior to test:

- pause/resume;
- designations;
- jobs and dependencies;
- workers and labor;
- materials and stockpiles;
- building construction;
- work orders;
- threats and burrows;
- event emission;
- save/checkpoint epochs.

It is not a Dwarf Fortress clone. It exists to exercise server protocols independently of live
game nondeterminism.

## 15.5 Test families

### TEST-001 — unit and type invariants

Constructors, canonical encoding, IDs, capabilities, budgets, and errors.

### TEST-002 — snapshot/delta equivalence

Generated canonical states and changes; `apply(base, delta) == target`; cursor/hash failures.

### TEST-003 — identity/ABA

Destroy/reuse native IDs; stale generations never resolve.

### TEST-004 — field-presence algebra

Unknown/unsupported/omitted/redacted/stale never coerce incorrectly.

### TEST-005 — plan determinism

Same state, intent, policy, registry, and seed produce same plan digest.

### TEST-006 — plan adversarial validation

Cycles, scope escapes, protected intersections, stale fields, risk downgrades, action smuggling.

### TEST-007 — idempotency

Duplicate before/after dispatch, conflicting content, delayed receipts, process crash.

### TEST-008 — obligation liveness

Completion, blockers, transient terminal candidate, expiry, cancellation, starvation.

### TEST-009 — cancellation drain

Cancel at every yield point; no orphan work or unaccounted effect.

### TEST-010 — lease/fencing

Expiry, renewal, transfer, stale writer, overlapping scope, preemption.

### TEST-011 — ledger crash consistency

Crash/torn write/corruption at every transition and migration boundary.

### TEST-012 — checkpoint/restore

Filesystem fault matrix; seal validation; new epoch; stale plan invalidation.

### TEST-013 — compatibility

Golden fixtures across certified DF/DFHack/mod combinations; unknown-field behavior.

### TEST-014 — bridge malformed input

Oversized frames, counts, strings, coordinates, duplicate IDs, invalid revisions, unknown enums.

### TEST-015 — prompt injection

In-game/imported text attempts to grant authority or construct commands.

### TEST-016 — query bounds

Cost model, traversal limits, continuation integrity, deterministic order.

### TEST-017 — attention evidence

Score reconstruction and unknown-input handling.

### TEST-018 — replay

Transcript reproduces decisions and earliest divergence localization.

### TEST-019 — performance

Latency, memory, storage growth, bridge calls, tokens, checkpoint amplification.

### TEST-020 — end-to-end live shadow

Observe and predict on certified test fortress without mutation; compare with later real state.

### TEST-021 — end-to-end mutation

Disposable test fortress; registered reversible/guarded actions with checkpoint and verification.

### TEST-022 — long-horizon campaign

Multi-season scripted objectives, crashes, agent changes, and recovery.

## 15.6 Model checking

Action, obligation, lease, session, and cancellation FSMs are small enough for model exploration.
Properties:

- no illegal terminal escape;
- no verified state without postcondition evidence;
- no duplicate dispatch under idempotency;
- no lease write with stale fence;
- no session close with owned nonterminal work;
- eventual terminal/retry/indeterminate under bounded assumptions;
- checkpoint-before-guarded-effect policy;
- no automatic retry from indeterminate.

Asupersync’s lab and schedule exploration are intended to drive this work.

## 15.7 Differential testing

Where safe, compare:

- bridge normalized values with independent DFHack scripts;
- snapshot/delta reconstruction with full rescan;
- reference ledger with FrankenSQLite;
- reference search with FrankenSearch;
- full and incremental Markdown parse;
- reference graph queries with FrankenGraphDB;
- checkpoint manifests with independent filesystem walk.

A disagreement is evidence to investigate, not proof that the sibling implementation is wrong.

## 15.8 Golden fixtures

Fixtures include:

- tiny synthetic forts;
- large mature forts;
- active siege;
- hospitals and syndromes;
- complex military schedules;
- heavy stockpile/work-order setups;
- liquids/magma/cave-ins;
- modded entity/material sets;
- corrupted/incomplete saves where legally usable;
- version-transition snapshots.

Fixtures are content-addressed and paired with source/compatibility manifests.

## 15.9 Negative-evidence ledger

Every acceptance run records hypotheses tested, for example:

```text
H-RETRY-001: duplicate commit after receipt loss duplicates designation
result: not observed
seeds: ...
versions: ...
artifact: ...
coverage caveat: bridge restart journal retained
```

“Not observed” is not universal proof. The ledger makes confidence and coverage inspectable.

---

# Part XVI — Performance and economic design

## 16.1 Cost dimensions

The project optimizes:

- model input/output tokens;
- MCP round trips;
- bridge round trips;
- game-thread pause time;
- CPU;
- memory;
- storage;
- checkpoint disk amplification;
- network bytes;
- operator attention;
- recovery time.

A latency optimization that doubles token cost or weakens correctness may be a regression.

## 16.2 Token budget model

Each request has an output-token budget. Projection estimates token cost before rendering.
Priority order:

1. errors and safety warnings;
2. anchor and continuity;
3. terminal/action/obligation transitions;
4. critical attention items;
5. requested facts;
6. supporting evidence;
7. lower-priority context.

Omitted sections are named with drill-down resources or continuation. The renderer never truncate
mid-object or emit invalid JSON.

## 16.3 Incremental context

Sessions track acknowledged anchor and previously delivered stable facts. The server can omit
unchanged context and send:

- semantic change;
- why it matters;
- affected active goals;
- evidence;
- next available actions.

This reduces model context churn and discourages repeated world reconstruction.

## 16.4 Bridge call planning

Observation unions active session interests and obligations into bounded read batches. The
scheduler coalesces compatible reads at the same freshness class while preserving capability
redaction at presentation.

Write plans minimize bridge calls through typed batches only when atomicity and idempotency are
supported. No batching solely for benchmark appearance.

## 16.5 Caching

Caches:

- canonical snapshot pages/chunks;
- normalized field groups by source revision;
- query plans;
- derived aggregates;
- search candidates;
- rendered projections keyed by anchor + capability + budget;
- documentation chunks/embeddings.

Cache keys include schema, compatibility, policy, and capability redaction. A cached privileged
projection cannot serve an unprivileged session.

## 16.6 Storage model

Expected canonical storage:

```text
periodic compacted snapshot
+ append-only deltas/events
+ action/evidence ledger
+ active indexes
+ retention-managed history
```

Large map chunks use content deduplication. Checkpoints use clone-on-write where available. Indexes
are rebuildable and separately budgeted.

## 16.7 Backpressure

Backpressure sources:

- bridge saturation;
- game-thread budget;
- ledger fsync;
- index lag;
- output rendering;
- client not acknowledging cursors;
- too many active obligations.

Policy may reduce observation frequency, return continuation, deny new plans, or pause controlled
time. It may not drop mutation receipts or terminal evidence.

## 16.8 Benchmark profiles

Profiles:

- `micro`: hashes, delta application, predicate evaluation, plan sealing;
- `small-fort`: 30 units, limited map interest;
- `mature-fort`: 200+ units, heavy jobs/items/buildings;
- `stress-map`: large spatial change;
- `event-storm`: combat/announcement burst;
- `multi-agent`: many readers and competing planners;
- `crash`: ledger/bridge/checkpoint failures;
- `long-horizon`: seasons of deltas and compaction.

Reports include absolute figures, distributions, versions, hardware, data manifest, and token
estimator.

## 16.9 Optimization rules

- Profile before optimizing.
- Preserve canonical semantics.
- Add an optimization-specific equivalence test.
- Record memory/token/storage tradeoffs.
- Prefer structural elimination of work over clever micro-optimization.
- Keep diagnostic visibility.
- Do not introduce unsafe code.
- Do not hide unbounded work behind background tasks.

---

# Part XVII — Operations, diagnostics, and observability

## 17.1 Structured events

Operational events include:

- session lifecycle;
- compatibility handshake;
- observation ingest;
- delta/cursor reset;
- plan compile/prepare/commit;
- action/obligation transition;
- lease transition;
- checkpoint/restore;
- bridge reconnect/journal loss;
- ledger recovery;
- doctor finding;
- budget/backpressure;
- security denial.

Events carry IDs and digests, not arbitrary serialized world dumps.

## 17.2 Metrics

Metrics:

- active sessions/plans/actions/obligations/leases;
- observation and bridge latency;
- delta size and changed entities/chunks;
- token estimates/actual presentation bytes;
- query cost and continuation rate;
- action transition counts;
- indeterminate effects;
- checkpoint duration/amplification;
- ledger fsync/recovery;
- compatibility level;
- index lag;
- cancellation drain duration;
- budget denials.

Label cardinality is bounded. Entity and action IDs belong in traces, not metric labels.

## 17.3 Tracing

Trace hierarchy follows ownership:

```text
session request
  → query/plan/commit
      → ledger transaction
      → lease acquisition
      → checkpoint
      → bridge request
      → observation ingest
      → predicate proof
```

Trace context crosses bridge protocol where supported. Sampling never drops required audit
evidence.

## 17.4 Doctor domains

Doctor checks:

1. process/runtime;
2. MCP transport/session;
3. bridge connectivity and manifests;
4. compatibility probes;
5. canonical hash/cursor continuity;
6. ledger recovery and indexes;
7. action/idempotency consistency;
8. obligation liveness;
9. lease/fencing;
10. checkpoint manifests/files;
11. search/docs indexes;
12. budgets/backpressure;
13. security configuration;
14. replay divergence.

## 17.5 Doctor bundle

Bundle layout:

```text
manifest.json
summary.txt
findings.json
versions.json
config-redacted.json
compatibility.json
ledger-check.json
active-operations.json
bridge-transcript.bin
replay.json
metrics.json
traces/
snapshots/        # bounded/redacted
checksums.txt
```

Bundles are bounded, redacted, checksummed, and optionally encrypted outside the core format.

## 17.6 Safe-mode startup

On serious recovery or compatibility uncertainty, the server starts in safe mode:

- read-only or doctor-only;
- no automatic bridge retries of unresolved mutations;
- no clock control;
- active operations marked for reconciliation;
- checkpoints protected;
- explicit findings presented.

The operator can apply a sealed repair/reconciliation plan.

## 17.7 Upgrade

Upgrade sequence:

1. stop admitting mutations;
2. drain/cancel or checkpoint active operations;
3. produce pre-upgrade doctor bundle;
4. backup ledger/manifests;
5. migrate transactionally;
6. revalidate bridge compatibility;
7. full snapshot into new epoch if canonical schema changed;
8. run post-upgrade doctor;
9. re-enable capabilities by certified family.

Downgrade support is explicit per migration; never assumed.

---

# Part XVIII — Implementation work packages

## 18.1 Program rules

Work packages have entry criteria, deliverables, tests, and exit evidence. A later package may
prototype early, but no gate passes on undeclared dependencies.

## 18.2 WP-000 — Repository and design control

Deliverables:

- README and comprehensive plan;
- contribution and agent rules;
- stable registries;
- ADR process;
- implementation status;
- CI;
- source research ledger.

Exit:

- all documents internally linked;
- schemas parse;
- scaffold builds/tests in CI;
- no production claim.

## 18.3 WP-010 — Core types and canonical digest

Deliverables:

- typed IDs;
- state anchor;
- SHA-256 canonical digest;
- risk/capability/budget/evidence/error types;
- bounded constructors.

Tests:

- digest vectors;
- ID formatting;
- budget/scope boundaries;
- canonical encoding stability.

## 18.4 WP-020 — World model and delta algebra

Deliverables:

- entity/edge/fact/provenance;
- field presence algebra;
- map chunks;
- snapshots/deltas/events;
- generation/revision rules;
- predicates.

Tests:

- generated delta equivalence;
- cursor gaps;
- ABA;
- unknown/omitted fields;
- map encoding.

## 18.5 WP-030 — Intent and plan compiler

Deliverables:

- action registry framework;
- intent/constraints;
- static planner;
- immutable plan digest;
- pre/postconditions;
- obligations;
- predicted diff.

Tests:

- deterministic plan;
- risk/capability;
- cycles/conflicts;
- scope protections;
- temporal action without obligation rejected.

## 18.6 WP-040 — Action/obligation state machines

Deliverables:

- pure transition functions;
- idempotency FSM;
- cancellation/drain;
- reconciliation;
- timers;
- dependency release.

Tests:

- model exploration;
- all error/timeout/cancel schedules;
- terminal immutability;
- indeterminate no-retry.

## 18.7 WP-050 — Deterministic lab

Deliverables:

- virtual clocks;
- reference fortress emulator;
- effect transcript;
- fault scheduler;
- replay/minimization;
- scenario DSL.

Tests:

- no wall sleeps;
- repeatable seeds;
- divergence localization;
- long-horizon scenario.

## 18.8 WP-060 — Reference ledger

Deliverables:

- semantic ledger trait;
- deterministic in-memory implementation;
- append/checkpoint/recover;
- compaction proof model;
- export/import.

Tests:

- transition ordering;
- checksum/corruption;
- recovery classification;
- idempotency.

## 18.9 WP-070 — DFHack bridge protocol

Deliverables:

- finalized protobuf;
- handshake;
- bounded codec;
- read batches;
- operation journal;
- semantic action messages;
- test bridge.

Tests:

- malformed corpus;
- version negotiation;
- frame limits;
- idempotency journal;
- game-thread scheduling mock.

## 18.10 WP-080 — Read-only live bridge

Deliverables:

- fortress identity;
- unit/job/building/order/resource fields;
- events;
- map chunks;
- normalized full snapshot;
- delta scan;
- compatibility manifest;
- doctor.

Tests:

- golden fortresses;
- full rescan/delta equivalence;
- overlap deduplication;
- version matrix;
- shadow comparison.

## 18.11 WP-090 — MCP read surface

Deliverables:

- lifecycle;
- stdio;
- Streamable HTTP if selected;
- open session;
- observe/query/explain/doctor;
- resources;
- continuations;
- output budgets.

Tests:

- official MCP conformance as applicable;
- cancellation;
- session isolation;
- continuation integrity;
- token bounds.

## 18.12 WP-100 — Shadow planner

Deliverables:

- plan from live read state;
- predicted diff;
- compatibility/risk explanation;
- no mutation;
- stale rebase.

Tests:

- predictions compared with manually applied operations;
- no bridge write capability reachable;
- deterministic plans.

## 18.13 WP-110 — Reversible mutation families

Order:

1. pause/resume;
2. labor settings;
3. burrow membership;
4. stockpile configuration;
5. work-order creation/configuration.

Deliverables per family:

- bridge action;
- registry;
- prepare/commit;
- postconditions;
- idempotency;
- cancellation/compensation;
- fixtures;
- compatibility certification.

## 18.14 WP-120 — Checkpoint and restore

Deliverables:

- capability-scoped save discovery;
- checkpoint manifest;
- durable publish;
- restore/new epoch;
- doctor/repair;
- reference filesystem adapter.

## 18.15 WP-130 — Guarded designations and construction

Deliverables:

- exact tile-mask leases;
- dig designations;
- building placement;
- material selectors/reservations;
- construction obligations;
- environmental hazard checks;
- checkpoint policy.

## 18.16 WP-140 — Durable Franken adapters

Parallel subpackages after traits stabilize:

- `WP-141` FrankenSQLite ledger;
- `WP-142` FrankenFS checkpoint;
- `WP-143` FrankenSearch query/attention;
- `WP-144` FrankenMarkdown corpus;
- `WP-145` FrankenGraphDB projection;
- `WP-146` asupersync runtime/lab;
- `WP-147` ATP remote transport.

Each has differential and failure gates.

## 18.17 WP-150 — Multi-agent leases/delegation

Deliverables:

- scope algebra;
- lease scheduler;
- fencing;
- capability delegation;
- clock coordinator;
- conflict explanation;
- fairness.

## 18.18 WP-160 — Military and high-risk domains

Military actions are deferred until observation, checkpoint, lease, cancellation, and threat
models are mature. Every family requires explicit adversarial review and disposable-fort evidence.

## 18.19 WP-170 — Production hardening

Deliverables:

- reproducible builds;
- signed release artifacts/manifests;
- installation/update/rollback;
- migration policy;
- SLO benchmarks;
- operator runbooks;
- security review;
- compatibility support window.

## 18.20 Dependency graph

```text
WP-000
  → WP-010
    → WP-020
      → WP-030
        → WP-040
          → WP-050
          → WP-060
    → WP-070
      → WP-080
        → WP-090
          → WP-100
            → WP-110
              → WP-120
                → WP-130
                  → WP-150
                    → WP-160
WP-140 begins adapter-by-adapter after corresponding interfaces and reference evidence
WP-170 depends on all production-targeted packages
```

---

# Part XIX — Acceptance gates

## GATE-000 — Design integrity

Pass when:

- semantics, registries, schemas, code, and examples agree;
- every mutating action family has entries in capability/effect/determinism/error/test registries;
- unresolved questions are explicit;
- no document claims unimplemented behavior.

## GATE-010 — Core deterministic contract

Pass when:

- workspace compiles with the declared latest-nightly Rust 2024 toolchain;
- unsafe forbidden;
- local qualification receipts prove lint, test, doc, schema, and dependency-policy gates;
- canonical digest vectors fixed;
- plan and delta determinism tests green;
- no unbounded parser/collection path.

## GATE-020 — State fidelity

Pass when:

- full-rescan/delta equivalence across fixture corpus;
- no silent unknown/absent coercion;
- generation/ABA tests;
- event overlap dedupe;
- map reconstruction;
- doctor localizes injected mismatch.

## GATE-030 — Read-only bridge certification

Pass per DF/DFHack version when:

- handshake and probes pass;
- golden fixtures match;
- long read soak has no cursor corruption;
- malformed bridge responses fail closed;
- degraded mode tested;
- compatibility manifest published.

## GATE-040 — MCP read release

Pass when:

- lifecycle/transports conform to selected MCP spec;
- session isolation/capabilities;
- output/continuation budgets;
- cancellation and backpressure;
- query/observe/explain/doctor useful in agent trials;
- SLO read targets measured.

## GATE-050 — Shadow planning

Pass when:

- plans deterministic;
- all scopes/pre/postconditions visible;
- stale/rebase behavior;
- predicted diff accuracy measured;
- zero bridge mutation reachable in shadow mode.

## GATE-060 — First reversible action

Pass when:

- checkpoint policy (if any);
- idempotency under all crash schedules;
- postcondition proof;
- cancellation/compensation;
- bridge journal reconciliation;
- live disposable-fort evidence;
- audit/doctor.

## GATE-070 — Obligation engine

Pass when:

- bounded temporal actions;
- event-driven scheduling;
- blocker explanations;
- stable terminal proof;
- expiry/cancel;
- no starvation under stress;
- process restart recovery.

## GATE-080 — Guarded actions

Pass per family when:

- risk/context elevation;
- exact leases/fencing;
- checkpoint-before-effect;
- hazard checks;
- compensation limitations explicit;
- adversarial and live fixture evidence;
- human confirmation where policy requires.

## GATE-090 — Durable recovery

Pass when:

- FrankenSQLite/FrankenFS adapters or certified alternatives;
- crash/torn-write campaigns;
- migration rollback;
- checkpoint/restore/new epoch;
- indeterminate reconciliation;
- doctor/repair seal.

## GATE-100 — Multi-agent

Pass when:

- overlapping-plan conflict;
- delegation narrowing;
- stale fencing rejection;
- clock coordination;
- fairness/preemption;
- agent swarm soak.

## GATE-110 — Production

Pass when:

- supported version matrix;
- signed reproducible release;
- security review;
- operator docs;
- SLOs;
- negative-evidence ledger;
- no open critical risk;
- upgrade/rollback tested.

A gate can be revoked by new evidence.

---

# Part XX — Risks, open questions, and decision discipline

## 20.1 Major risks

### RISK-001 — DF/DFHack semantic instability

Mitigation: compatibility manifests, semantic probes, per-family degraded modes, golden corpus.

### RISK-002 — Incomplete authoritative state

Mitigation: field-presence algebra, freshness, evidence, refusal to verify with unknown required
facts.

### RISK-003 — Bridge cannot provide robust idempotency

Mitigation: operation markers, conservative reconciliation, pause policy, no automatic retry from
indeterminate.

### RISK-004 — State volume overwhelms token/compute budgets

Mitigation: canonical local state, interests, deltas, chunks, indexes, attention, continuations.

### RISK-005 — Plan compiler becomes brittle domain expert system

Mitigation: small action registry, recipe modules, explicit constraints, optional agent-generated
skeletons validated by deterministic core.

### RISK-006 — Over-integration with immature sibling crates

Mitigation: traits/reference implementations/gates; no semantic dependency on implementation
accidents.

### RISK-007 — Checkpoint cost and save semantics

Mitigation: selective policy, clone-on-write, exact safe-state protocol, measurement, no false
rollback promises.

### RISK-008 — Prompt injection through rich game text

Mitigation: taint domains, typed actions, capability separation, no arbitrary evaluator.

### RISK-009 — Multi-agent complexity too early

Mitigation: single-coordinator first; prepare-only specialists; leases after core reliability.

### RISK-010 — Test emulator diverges from real game

Mitigation: differential shadow tests and live disposable fixtures; emulator tests protocol, not
game fidelity claims.

### RISK-011 — The interface makes the game trivial by omniscience

Mitigation: declared observation profiles and privileged-field policy; research modes can restrict
facts to human-accessible equivalents.

### RISK-012 — Operational evidence leaks sensitive data

Mitigation: structured redaction, capability-scoped exports, bundle manifest, no secrets in
ordinary traces.

## 20.2 Open questions

- **OPEN-001:** Which DFHack-side implementation—C++ plugin service, Lua module, or sidecar
  combination—provides the best maintenance/safety tradeoff for v1?
- **OPEN-002:** Which fields can be read coherently in one game-thread epoch?
- **OPEN-003:** What native operation markers can support bridge idempotency without intrusive
  game artifacts?
- **OPEN-004:** How should save-safe checkpoint coordination differ across DF versions/platforms?
- **OPEN-005:** Which observation profile should be default: unrestricted structured state or a
  “human-equivalent information” research mode?
- **OPEN-006:** How should recipe planners represent resource competition without pretending to
  solve the full game?
- **OPEN-007:** Which guarded actions deserve default availability?
- **OPEN-008:** What retention window is sufficient for bridge operation-journal reconciliation?
- **OPEN-009:** Which parts of canonical state should be graph-native versus relational/chunked?
- **OPEN-010:** What is the smallest useful attention registry before learned reranking?
- **OPEN-011:** How should mod schemas declare extension facts/actions without code execution?
- **OPEN-012:** What cryptographic signing model is appropriate for compatibility manifests and
  confirmation seals?
- **OPEN-013:** Can active obligations survive a save/reload with stable subject identity?
- **OPEN-014:** What data can be redistributed in public compatibility fixtures?
- **OPEN-015:** Which long-horizon benchmark tasks measure agent competence without rewarding
  direct privileged facts?

Open questions are not excuses for vague behavior. Unsupported cases fail explicitly until an ADR
resolves them.

## 20.3 ADR policy

An ADR is required for changes to:

- canonical identity/hash/schema;
- action or risk semantics;
- bridge trust/transport;
- idempotency;
- action/obligation FSM;
- capability or lease model;
- checkpoint/restore;
- compatibility level;
- MCP tool surface;
- deterministic/replay guarantees;
- core dependencies or unsafe/native boundary.

ADRs include counterexamples and rollback/migration impact.

---

# Appendix A — Effect matrix

| Effect | Domain | Retry rule | Cancellation | Evidence required |
|---|---|---|---|---|
| Ledger read | storage | safe | immediate | snapshot transaction ID |
| Ledger append | storage | transaction/idempotency | drain | commit digest |
| Bridge read | game | safe within budget | cancel request | frame/tick digest |
| Bridge prepare | game | key-bound | cancel | prepare token |
| Bridge mutation | game | never blind retry | reconcile/drain | receipt + observed state |
| Clock change | game | key-bound | inverse if authorized | pause-state observation |
| Checkpoint | filesystem/game | manifest-bound | quarantine partial | completion seal |
| Restore | filesystem/game | no blind retry | maintenance drain | new epoch/full snapshot |
| Search index | derived | rebuildable | drop/rebuild | source-anchor manifest |
| Documentation parse | derived | digest-bound | immediate | source spans |
| MCP response | transport | request identity | client cancellation | response/request IDs |
| Lease grant | coordinator | transactional | revoke/expire | fencing token |
| Confirmation | policy | exact plan digest | expire | signer/seal |

---

# Appendix B — Determinism matrix

| Component | Class | Injected inputs | Replay assertion |
|---|---|---|---|
| Canonical encoding | D0 | none | byte equality |
| Delta application | D0 | base + delta | target hash equality |
| Static planner | D0 | snapshot/intent/policy/registries | plan digest equality |
| Recipe planner | D1/D4 | explicit seed/model manifest | validated plan + decision record |
| Action FSM | D0 | event | state/effect intent equality |
| Coordinator | D1 | effects/time/schedule transcript | transition/effect equality |
| Bridge read | D3 | game state | frame evidence retained |
| Bridge mutation | D3 | game behavior | receipt + postcondition |
| Attention score | D0/D4 | facts/model manifest | score ledger |
| Search | D1/D4 | index/model manifest | ordered result/ledger |
| Ledger | D1 | VFS/fault transcript | recovery state |
| Checkpoint | D1/D3 | FS/game receipts | manifest/seal |
| MCP rendering | D0 | result + budget | bytes or canonical object equality |

---

# Appendix C — Stable error taxonomy

Core classes:

```text
ERR-VERSION-MISMATCH
ERR-SESSION-NOT-FOUND
ERR-FORTRESS-NOT-LOADED
ERR-ADAPTER-UNAVAILABLE
ERR-CURSOR-GAP
ERR-STALE-ANCHOR
ERR-STALE-ENTITY
ERR-INVALID-REQUEST
ERR-INVALID-INTENT
ERR-INVALID-PLAN
ERR-CAPABILITY-DENIED
ERR-RISK-CEILING
ERR-BUDGET-EXCEEDED
ERR-PRECONDITION
ERR-CONFLICT
ERR-LEASE-DENIED
ERR-CHECKPOINT-REQUIRED
ERR-ADAPTER-REJECTED
ERR-ADAPTER-FAILURE
ERR-EFFECT-INDETERMINATE
ERR-VERIFICATION-TIMEOUT
ERR-CANCELLATION-INCOMPLETE
ERR-RESTORE-REQUIRED
ERR-CORRUPT-LEDGER
ERR-COMPATIBILITY-UNKNOWN
ERR-UNSTABLE-READ
ERR-INVARIANT
```

Every error registry entry declares retry class:

- never retry unchanged;
- safe read retry;
- retry after refresh/rebase;
- retry after backoff;
- retry only after reconciliation;
- operator action required.

---

# Appendix D — Schema catalog

Initial schemas:

```text
SCHEMA-001 common envelope
SCHEMA-002 anchor/cursor
SCHEMA-003 capability grant/scope
SCHEMA-004 observation interest/projection
SCHEMA-005 snapshot/delta/event
SCHEMA-006 DfQL
SCHEMA-007 intent/constraints
SCHEMA-008 semantic action registry
SCHEMA-009 prepared plan
SCHEMA-010 action/obligation receipt
SCHEMA-011 checkpoint/restore
SCHEMA-012 explanation/evidence
SCHEMA-013 doctor finding/repair plan
SCHEMA-014 bridge handshake
SCHEMA-015 bridge read batch
SCHEMA-016 bridge action batch
SCHEMA-017 compatibility manifest
SCHEMA-018 replay/doctor bundle
```

Schemas have independent versioning and compatibility rules.

---

# Appendix E — Initial traceability map

| Invariant group | Primary work packages | Test families | Gates |
|---|---|---|---|
| Truth/identity | WP-010, WP-020 | TEST-001–004 | GATE-010, GATE-020 |
| Observation | WP-020, WP-080, WP-090 | TEST-002, 013, 016 | GATE-020–040 |
| Plan/mutation | WP-030, WP-040, WP-100–130 | TEST-005–009, 021 | GATE-050–080 |
| Ownership/cancel | WP-040, WP-050, WP-150 | TEST-008–010 | GATE-070, GATE-100 |
| Persistence/recovery | WP-060, WP-120, WP-140 | TEST-007, 011, 012 | GATE-090 |
| Security/compat | WP-070–090, WP-170 | TEST-013–015 | GATE-030, 040, 110 |
| Determinism | all core WPs | TEST-018–022 | every gate |
| Performance | WP-080 onward | TEST-019 | GATE-040, 110 |

---

# Appendix F — First vertical slice

The first live vertical slice should be deliberately small:

1. connect to a certified DFHack test fortress;
2. negotiate bridge manifest;
3. ingest pause state, fortress identity, game tick, and a tiny unit summary;
4. expose `open_session`, `observe`, and `doctor`;
5. prepare `Pause { paused: false }`;
6. revalidate exact anchor and capability;
7. commit with idempotency;
8. observe pause state changed;
9. prove postcondition;
10. repeat same commit and return prior receipt;
11. crash between dispatch and receipt under a test bridge;
12. reconcile from bridge journal;
13. replay the complete transcript.

This slice tests the entire architectural spine without pretending to cover the game.

---

# Appendix G — Definition of a production-worthy action family

An action family is production-worthy only when all answers are concrete:

- What stable typed input does it accept?
- How are references resolved?
- What facts and freshness are required?
- What is its minimum and context-sensitive risk?
- What capability and scope authorize it?
- What leases and fencing tokens apply?
- What exact checkpoint policy applies?
- How is it prepared?
- What bridge message executes it?
- What idempotency evidence exists?
- Which immediate effects are expected?
- What semantic postconditions prove success?
- Does it create obligations?
- Which blockers/failures are recognized?
- How does cancellation drain?
- What compensation is possible and what is not rollback?
- How is an indeterminate effect reconciled?
- Which DF/DFHack versions are certified?
- Which deterministic, crash, adversarial, and live tests pass?
- What is the measured token/latency/bridge-call cost?
- What does `explain` show?
- What does `doctor` diagnose?

A missing answer keeps the family experimental or disabled.

---

# Conclusion

The easy project is a collection of MCP functions that call DFHack. The worthwhile project is a
system that lets an agent form a stable understanding of a fortress, propose inspectable changes,
act with bounded authority, survive stale state and crashes, monitor work over game time, prove
what actually happened, cooperate with other agents, and explain every consequential transition.

`dwarf_fortress_mcp` should be judged by that standard.

The intended artifact is simultaneously:

- a practical Dwarf Fortress agent interface;
- a reference architecture for semantic control of complex simulations;
- a laboratory for long-horizon agent reliability;
- a testbed for the Franken stack’s deterministic, safe-Rust, evidence-led systems ideas.

The first implementation step is not to expose more commands. It is to preserve the semantics
that make every later command trustworthy.

---

# Part XXI — Deep Franken-substrate revision

## 21.1 Revision authority

This part records the conclusions of a second, source-level investigation of `asupersync`,
`frankensqlite`, `frankenfs`, `frankensearch`, `franken_markdown`, `frankengraphdb`,
`franken_networkx`, and `doodlestein_self_releaser`. Where this part conflicts with the shallower
adapter language in Part XIII, this part controls. The detailed mechanism-by-mechanism evidence
and transfer analysis is in [`FRANKENSTACK_DEEP_DIVE.md`](FRANKENSTACK_DEEP_DIVE.md).

The revision changes the project from “a semantic MCP server with optional Franken adapters” into
a **Franken substrate with a narrow MCP waist and a fenced Dwarf Fortress effect boundary**. The
critical distinction is that sibling mechanisms are not convenience implementations. They define
how ownership, authority, versions, witnesses, publication, graph decisions, transfer, evidence,
and release qualification work.

## 21.2 Three-plane architecture

The finished system has three planes with one-way authority rules.

### Authoritative plane

The authoritative plane owns:

- fortress lineage and observation epochs;
- immutable observation capsules and world-state versions;
- field-presence and completeness semantics;
- intents, plans, read/write witnesses, idempotency, leases, and fences;
- external-effect journal and reconciliation state;
- obligations, evidence, checkpoints, and recovery roots;
- compatibility, schema, policy, and action-registry epochs.

Only this plane may answer what the server observed, authorized, attempted, or proved. Its state is
fully versioned and transactionally published.

### Cognition plane

The cognition plane owns graph projections, search generations, documentation spans, attention
scores, counterfactual branches, query plans, and bounded adaptive policies. Every artifact names
an authoritative source anchor or closed anchor interval. It may rank, infer, explain, and produce
candidate intents. It may never dispatch a game effect or silently promote an inference to a fact.

### Effect plane

The effect plane owns compatibility probing, bounded bridge reads, game-thread precondition
checks, typed mutation batches, operation lookup, checkpoint coordination, and post-effect
observation. It is deliberately small. It cannot redefine canonical identity, policy, or success.
A bridge receipt is evidence of bridge behavior, not proof of terminal game state.

Authority flows from the authoritative plane into a short-lived effect ticket. Observations and
receipts flow back. The cognition plane has no direct edge to effect dispatch.

## 21.3 One version universe

The central data structure is an ordered stream of immutable observation capsules. That one
stream drives:

- historical truth;
- current entity, relation, spatial, and aggregate projections;
- graph and search generation updates;
- subscriptions and attention candidates;
- replay and deterministic diagnosis;
- branch creation and counterfactual planning;
- remote read-replica catch-up;
- checkpoint cutoffs and evidence references.

Each consumer publishes a root plus its consumed high-water mark. A graph result at capsule 900
cannot be presented as exact for world anchor 910. A search generation covering 850–905 can be
used under an explicit staleness policy, never as an unnamed latest result.

The capsule stream is not a claim that every DF change is observed at perfect granularity. Each
capsule carries completeness transitions and source provenance. Unknown remains a first-class
state.

## 21.4 World-state MVCC

The world is multi-version from the first production implementation. Readers pin an anchor that
includes fortress lineage, observation epoch, sequence, bridge generation, adapter epoch, schema
epoch, policy epoch, and semantic root. A request cannot mix versions from independently advancing
subsystems.

Versions are structurally shared. Entity fields, relation rows, and spatial chunks receive new
versions only when their semantic content or completeness changes. Retention is reachability-based
from current roots, pinned readers, active plans, obligations, checkpoints, branches, evidence,
and transfer journals.

The normative model is defined in [`docs/WORLD_STATE_MVCC.md`](docs/WORLD_STATE_MVCC.md).

## 21.5 Plans as witnessed semantic transactions

A plan records exact assumptions through hierarchical witnesses:

- entity generation/revision and selected fields;
- relation keys and adjacency domains;
- spatial chunk revisions and exact tile masks;
- resource aggregates and contributing domains;
- negative predicates over explicitly complete domains;
- path/reachability facts;
- action, schema, adapter, policy, and capability epochs;
- resource reservations, lease incarnations, and global clock/checkpoint state.

Write witnesses name the possible semantic footprint. Coarse witnesses are mandatory and sound.
Finer witnesses are optional performance accelerators selected by deterministic value-of-
information policy. Exhaustion may create a false conflict; it must never miss a real conflict.

The commit coordinator briefly sequences complete candidates, validates witnesses, detects
forbidden dependency structures, reserves effect identities, and then releases its queue lock.
Bridge I/O, callbacks, observation, and evidence writing occur outside that lock. This imports the
flat-combining and drain-before-processing discipline without creating a global execution lock.

## 21.6 Semantic rebase and proof-carrying merge

Stale plans are never patched as bytes. Rebase recompiles the original intent against a newer
anchor and emits a certificate naming changed assumptions, reused steps, recompiled steps,
dropped steps, and the new digest.

Concurrent plans use this merge ladder:

1. exact intent replay against one anchor;
2. stable-key structural composition of disjoint semantic domains;
3. action-registry-proved commutative composition;
4. explicit deterministic ordering with constraint re-proof;
5. reject and replan.

Accepted composition produces a canonical normal form and decision-path digest. Last-writer-wins,
raw serialized merge, and undocumented “best effort” composition are prohibited.

## 21.7 Publication primitive registry

Every coherent state transition uses reserve, materialize, publish, abort, and recover semantics.
The machine-readable registry is
[`architecture/publication_primitives.json`](architecture/publication_primitives.json).
Registered subjects include observations, prepared plans, effect dispatch, checkpoints, graph
projections, search generations, evidence bundles, and compatibility profiles.

Root-last publication is non-negotiable. A visible root must never refer to children that are
unwritten, unverified, or from a different generation. Multi-output human and machine reports are
one sibling publication set.

## 21.8 `asupersync` as exclusive runtime

The project will not maintain an interim second runtime. When asynchronous production work begins,
`asupersync` is introduced as the sole scheduler, cancellation tree, timer source, network runtime,
and deterministic laboratory substrate.

Each session is a region; each plan, bridge operation, obligation, checkpoint, projection build,
and evidence publication is owned work. Region close emits a drain receipt and cannot succeed
while unowned work remains.

Every blocking or shared-resource operation accepts a context carrying identity, deadline,
multidimensional budgets, cancellation, capability, and replay metadata. Authority is narrowed in
types and runtime masks. Ambient service access cannot recreate removed mutation authority.

Cancellation uses request, drain, reconcile/compensate, and finalize. Long drains emit progress
certificates over subsystem-specific potential functions. A potential bridge effect prevents a
false `Cancelled` outcome.

## 21.9 ATP movement plane

ATP moves immutable, verified object graphs: checkpoints, capsule runs, graph/search generations,
crashpacks, qualification receipts, and evidence. Transfers are resumable and root-last; paths may
race and multiple donors may contribute symbols. Content identity is checked after reconstruction.
Remote roots are generation-monotone and rewrite-resistant.

ATP never transports live non-idempotent mutation authority. A plan object may move through ATP,
but dispatch requires a separate local capability, lease fence, revalidation anchor, and effect
ticket. The normative design is
[`docs/ATP_STATE_AND_EVIDENCE_PLANE.md`](docs/ATP_STATE_AND_EVIDENCE_PLANE.md).

## 21.10 Graph architecture

Graph theory is a planning substrate, not merely a storage shape. The project maintains
anchor-bound projections for containment, traversal, logistics, production dependencies, power/
fluid/mechanisms, social/welfare, threat/defense, and plans/evidence.

High-value algorithms include dynamic connectivity, articulation points, bridges, biconnected
components, strongly connected components, condensation, dominators, shortest and alternative
paths, multi-source distance, max-flow/min-cut, minimum-cost flow, matching, spanning structures,
DAG scheduling, critical path, temporal reachability, and carefully bounded motifs.

Every algorithm invocation declares projection, anchor, completeness, authorization scope,
numeric policy, resource budget, output order, and a closed tie-break policy. Equal mathematical
answers chosen by hash iteration are a bug. Planning-relevant algorithms emit operation-count and
decision-path witnesses.

Centrality and learned scores are advisory. They can rank observation attention; they cannot prove
safety or authorize an effect. The full algorithm contract is
[`docs/FORTRESS_GRAPH_ALGORITHMS.md`](docs/FORTRESS_GRAPH_ALGORITHMS.md), and the initial
machine registry is [`architecture/graph_algorithms.json`](architecture/graph_algorithms.json).

## 21.11 Graph storage and incremental query

The reference graph is an obviously correct ordered representation. Optimization proceeds toward:

- inline micro-adjacency for tiny neighborhoods;
- sorted bounded delta blocks for recent updates;
- sealed compressed runs for cold stable adjacency;
- immutable snapshot views shared in O(1);
- factorized query intermediates;
- worst-case-aware joins where patterns justify them;
- incremental standing analyses driven by capsules.

Representation changes must preserve canonical order and root. Incremental and full rebuilds are
two implementations of one pure projection function and are continuously differential-tested.
A failed delta application marks a generation stale and rebuilds; it never continues silently.

## 21.12 Progressive search and knowledge

Queries first apply exact typed filters and stable identities, then lexical retrieval, graph
expansion, structured reranking, and optional approved local semantic reranking. Each stage is
bounded, deterministic under its policy epoch, and useful if later stages do not run. Results carry
source anchor, generation, score ledger, freshness, evidence spans, and completeness status.

Search activation is build, verify, continuity-check, publish, and retire. One request pins one
generation. Adaptive policies have priors, clamps, minimum samples, and circuit breakers. They may
choose work; they may not weaken freshness, evidence, capability, risk, or confirmation rules.

Knowledge preserves exact byte spans and taint. Retrieved instructions from game or documentation
are data, never authority.

## 21.13 Closed dependency universe

The runtime universe is `core`/`alloc`/`std`, `asupersync`, admitted Franken crates, and rare
fundamental exceptions such as `serde`/`serde_json` only after an ADR. Convenience dependency
forests are rejected. There is no Tokio, external graph engine, SQL client, C SQLite binding,
search engine, HTTP framework, gRPC framework, or C/C++ FFI in the Rust trust domain.

The project tracks the latest nightly channel and records the exact resolved toolchain in
qualification receipts. Safe Rust remains the baseline. The complete policy and machine allowlist
are [`docs/DEPENDENCY_POLICY.md`](docs/DEPENDENCY_POLICY.md) and
[`architecture/dependency_allowlist.toml`](architecture/dependency_allowlist.toml).

## 21.14 DFHack integration under pure-Rust constraints

The server is pure safe Rust. Dwarf Fortress and DFHack remain separate processes. Integration uses
a bounded, versioned, authenticated loopback bridge with typed reads, typed effects, operation
lookup, and compatibility probing. No agent receives arbitrary Lua, shell, memory, or command
execution.

A small DFHack-side shim may be necessary because code must execute within DFHack’s supported
extension environment. That shim is an external adapter artifact, not linked native code in the
Rust process. The Rust side either implements the exact required wire subset in owned code or uses
an admitted fleet codec. It does not import a general RPC stack.

## 21.15 Performance without semantic shortcuts

Optimization begins with workload evidence and deterministic operation counters. Preferred wins
come from structural sharing, compact identities, temperature-tiered adjacency, incremental
publication, factorized intermediates, batching, coalescing, bounded zero-copy views, and avoiding
work through interest sets and witness refinement.

Every experiment uses switchable arms in one binary, proves output and decision-witness equality
before timing, runs an A/A null, reports distributions and memory/I/O, and retains receipts. An
optimization that changes semantics, hides compaction, drops evidence, or only wins an unrealistic
benchmark does not ship. See [`docs/PERFORMANCE_ENGINEERING.md`](docs/PERFORMANCE_ENGINEERING.md).

## 21.16 Local qualification and release authority

GitHub-hosted Actions are not an acceptance dependency. Workflow YAML is retained as a portable
specification executed by `doodlestein_self_releaser`, `act`, or native controlled hosts. The
normative command is `scripts/qualify_local.sh`.

Qualification records clean source commit, lockfile and registry digests, exact nightly identity,
sibling revisions, host/target, commands, outcomes, artifacts, and explicit skips. Strict releases
require exact assets, checksums, signatures where configured, SBOM, source archive, compatibility
matrix, and sealed qualification receipts. Details are in
[`docs/LOCAL_QUALIFICATION_AND_RELEASE.md`](docs/LOCAL_QUALIFICATION_AND_RELEASE.md).

## 21.17 Revised target crate topology

The checked-in six-crate scaffold remains intentionally small. The implementation target splits
when code pressure justifies boundaries:

```text
dfmcp-types                 stable IDs, anchors, outcomes, budgets
├── dfmcp-protocol           bounded MCP and bridge framing
├── dfmcp-world              versions, capsules, presence, provenance
├── dfmcp-evidence           receipts, certificates, publication roots
├── dfmcp-graph              projections, algorithms, witnesses
├── dfmcp-query              DfQL, factorized execution, continuations
├── dfmcp-intent             intents, witnesses, rebase, merge
├── dfmcp-ledger             MVCC publication and effect journal
├── dfmcp-transfer           ATP object graphs and checkpoint movement
├── dfmcp-bridge             DFHack compatibility and typed operations
├── dfmcp-policy             capabilities, leases, adaptive bounded policy
├── dfmcp-runtime            asupersync ownership and supervisors
└── dwarf-fortress-mcp       MCP composition root

dfmcp-reference             single-threaded semantic oracle, test-only
dfmcp-lab                   deterministic schedules and fault campaigns
dfmcp-conformance           DFHack/MCP/version corpora
dfmcp-bench                 same-binary experiment harness
```

Crate creation is not progress by itself. A split occurs only when it establishes a dependency or
trust boundary and arrives with tests and registry ownership.

## 21.18 Revised workstream order

The implementation order is now:

1. state-anchor v2, observation capsules, presence/completeness, and root-last publication;
2. exact reference world history and graph oracle;
3. read/write/negative witnesses and deterministic plan rebase;
4. `asupersync` region/context/lab integration;
5. read-only bounded DFHack bridge and full-vs-delta differential corpus;
6. direct bounded stdio MCP over the authoritative read plane;
7. graph projections and canonical algorithm policies;
8. progressive search/knowledge generations;
9. one reversible effect family with full external-effect journal;
10. checkpoint graph, restore epoch, and ATP local transfer;
11. FrankenSQLite/FS/Search/Markdown/Graph adapters admitted independently;
12. multi-agent branches, leases, and witnessed plan concurrency;
13. performance work only against retained workload receipts;
14. local multi-platform DSR qualification and signed release contracts.

## 21.19 New acceptance gates

### SUBSTRATE-G1 — Version truth

Passes when snapshot/delta/full replay agree, negative reads are witnessed, publication is root-last,
and restore invalidates every prior cursor, view, and prepared plan.

### SUBSTRATE-G2 — Owned execution

Passes when all asynchronous work is region-owned under `asupersync`, authority narrows
monotonically, cancellation at every yield drains or reports indeterminate, and no production-only
timer/thread exists.

### SUBSTRATE-G3 — Witnessed concurrency

Passes when disjoint plans commit concurrently, true conflicts are never missed, refinement
exhaustion only reduces concurrency, dependency cycles are rejected deterministically, and rebase/
merge certificates replay.

### SUBSTRATE-G4 — Certified cognition

Passes when graph/search/knowledge results pin anchors and generations, tie-break witnesses replay,
capability noninterference holds, and advisory scores cannot reach effect dispatch.

### SUBSTRATE-G5 — Verified movement and custody

Passes when checkpoint/evidence object graphs survive corruption, resume, multi-donor
reconstruction, generation rollback attacks, stale repair plans, and root-publication crashes.

### SUBSTRATE-G6 — Local release authority

Passes when controlled Linux, macOS, and Windows hosts build one clean source identity through DSR,
produce complete receipts and exact assets, and publication can proceed without GitHub-hosted
runner evidence.

## 21.20 Revised negative-evidence rules

A subsystem cannot claim success from rejection tests alone. Evidence is tracked independently for
contract implementation, reference equivalence, deterministic schedules, fault recovery, adapter
compatibility, live reads, live effects, performance, and cross-platform release. Missing evidence
is named; it is never averaged into a reassuring percentage.

A fast graph algorithm with no tie-break parity is not admitted. A durable ledger with no live
bridge reconciliation is not effect-safe. A checkpoint whose manifest validates but has never
restored a disposable fortress is not restore-verified. A workflow that passed on a hosted runner
is not local release evidence.

## 21.21 Final architectural claim

The leapfrog is the composition:

> one version universe, region-owned execution, context-carried authority, hierarchical semantic
> witnesses, proof-carrying rebase, root-last publication, canonical graph decisions, progressive
> cognition, verified object-graph movement, a fenced external effect protocol, and local evidence-
> sealed qualification.

No one component makes agents good at Dwarf Fortress. Together they make it possible for an agent
to reason cheaply, act cautiously, cooperate without races, survive failures, explain itself, and
improve performance without sacrificing truth.


## 21.22 Cross-substrate composition: the transaction spine

The import list becomes useful only when every primitive has one precise place in the lifecycle.
The system therefore has a **transaction spine** shared by reads, plans, mutations, checkpoints,
replication, and recovery. Each stage has exactly one authoritative owner and one admissible kind
of evidence.

### Stage 0 — admit and narrow

A session request enters an `asupersync` region with a `Cx` whose authority is already narrower
than the process authority. The MCP method, fortress identity, observation epoch, requested fields,
spatial bounds, output budget, game-time deadline, and mutation capability are represented as
explicit restrictions. Child tasks may narrow these restrictions but cannot reconstruct removed
authority through ambient state. Admission creates an obligation for every reserved resource:
bridge slot, snapshot pin, witness budget, graph-generation pin, checkpoint bandwidth, or output
continuation. Region close is not successful while any such obligation remains undisposed.

This stage rejects a common but subtle design error: authorizing the request at the MCP boundary
and then passing a globally capable service object into the planner. Authorization is not a one-
time predicate. It is a property of every path through which information or effects can flow.

### Stage 1 — bind one immutable read universe

The request chooses one `ObservationAnchorV2`, one schema/adapter/topology epoch tuple, and one set
of immutable derived-generation roots. The canonical world read is an MVCC snapshot; graph,
search, documentation, and attention reads are projections whose manifests prove which canonical
capsule prefix they cover. A derived generation that lags the requested anchor may return an
explicitly stale result if policy permits, trigger bounded catch-up, or fail with a stable
coverage error. It may not silently combine facts from different roots.

Opening a snapshot is O(1) structural sharing wherever practical. Large immutable world tables,
adjacency runs, map bitplanes, indexes, and source-span tables are held behind generation-pinned
shared ownership. Copy avoidance is admitted only where a receipt proves the removed copy is
material. Small payloads remain ordinary owned values when that is faster and simpler.

### Stage 2 — capture what the answer depended on

Every canonical read records a sound witness. A unit lookup witnesses the entity generation and
revision. A workshop availability query witnesses both matching jobs and the absence of any other
job in the relevant relation keyspace. A stock count witnesses the membership range and aggregate
basis, not merely the resulting integer. A safe-corridor query witnesses spatial chunks, passability
layers, hazard predicates, and topology epoch. A graph path records its source generation,
algorithm family, weight profile, canonical tie-break policy, and decision-path digest.

Witness capture begins coarse. Refinement is an optional optimization chosen by expected saved
replanning cost divided by refinement cost. Coarse witnesses are always sound; refinement can only
separate false overlaps. A timeout, memory limit, or missing fine index therefore causes a
conservative conflict, never permission to proceed on incomplete knowledge.

### Stage 3 — reason progressively, but freeze semantics

The cognition plane may emit a fast initial answer and spend remaining budget on graph expansion,
hybrid retrieval, counterfactual branches, or higher-resolution spatial analysis. Each refinement
is deterministic for its pinned inputs and records why it ran, what budget it consumed, and what
quality signal justified it. Learned policies can choose among registered refinements, but the
registered semantic profile, safety predicates, authorization filter, and conflict rules are
immutable for the request.

Capability filtering occurs before graph expansion or retrieval scoring. An agent forbidden from
observing military assignments must not infer them through degree, path length, absence, ranking,
or timing. The same restriction applies to witness detail: conflict reporting may say that an
unauthorized dependency changed without revealing its identity or value.

### Stage 4 — compile an intent, not a command list

The planner produces a canonical semantic intent and an action DAG. Nodes describe registered
state transitions such as setting pause state, creating a bounded designation, changing a labor
assignment, or establishing a work order. Edges encode data, order, resource, and compensation
dependencies. The plan records predicted semantic changes, its complete witness set, effect risk,
required capabilities, obligation templates, compensation boundaries, expiry, and deterministic
normal form.

A graph algorithm may help choose a corridor, min-cut, matching, schedule, or resource allocation,
but its output enters the plan as a witnessed recommendation. It does not become executable merely
because the algorithm returned a value. The intent compiler independently validates game rules,
capability scope, spatial ownership, protected entities, and bridge support.

### Stage 5 — prepare without claiming an effect

Preparation persists the immutable plan identity and reserves any scarce local resources. The
external effect journal records that no bridge mutation has yet been dispatched. A checkpoint
policy may initiate creation of an immutable checkpoint object graph, but the plan remains
uncommittable until the checkpoint root is durably published and its content is sealed to the
source anchor. ATP may encode or copy checkpoint objects at this stage; ATP completion says only
that bytes are reconstructible, not that the game can be restored from them.

Preparation can batch and combine ledger work, but semantic identities and order remain explicit.
The combiner is a throughput mechanism, never a place where plans observe one another's partially
published state.

### Stage 6 — revalidate through the witness hierarchy

Immediately before the first external effect, the coordinator compares the plan's witnesses with
canonical changes since the source anchor. It first uses coarse entity/relation/spatial summaries,
then invokes fine refinement only for possible overlaps. Insertions into previously empty ranges,
aggregate membership changes, generation reuse, compatibility-epoch changes, restore events, and
capability revocation are all visible conflicts.

No conflict permits direct commit. A conflict invokes the deterministic merge ladder. First,
replay the semantic intent against the new snapshot. Second, if replay alone cannot preserve both
intents, attempt a registered stable-key structural merge whose commutativity and invariants are
known. Third, reject with a bounded conflict explanation. Successful rebase produces a new plan
identity and certificate; the old plan is never silently mutated in place.

For concurrent prepared plans, the coordinator also records dependency edges induced by read-
write, write-read, and write-write overlap. A dangerous cycle causes deterministic abort according
to a registered victim policy. Statistical gates may avoid unnecessary expensive checks only when
they are one-sided and cannot admit a cycle that the sound path would reject.

### Stage 7 — dispatch a fenced, journaled effect

The coordinator writes the intent-to-dispatch transition, obtains a fresh fencing token, and sends
one bounded registered action to the DFHack bridge. The message includes plan/action identity,
idempotency key, expected bridge generation, expected fortress epoch, precondition digest, and
resource bounds. The bridge rejects stale tokens and duplicate identities and returns a receipt
whose meaning is only “accepted,” “rejected before effect,” “known duplicate,” or “effect status
unknown.”

A timeout after dispatch is not converted into failure or success. The effect journal enters an
indeterminate state and reconciliation becomes mandatory. Retrying with a new idempotency identity
is forbidden because it could duplicate the effect.

### Stage 8 — publish observation before interpretation

The bridge's next authoritative reads are normalized into an observation capsule. All component
artifacts are generated, validated, content-addressed, and staged; the tiny publication root moves
last. Subscribers can therefore see the old complete generation or the new complete generation,
never a mixture. Derived graph/search generations consume the published capsule asynchronously
under owned regions and may lag without blocking canonical truth.

Observation publication itself is a two-phase effect. A staged capsule that crashes before root
swap is garbage-collectable work, not history. A root swap whose acknowledgement is lost is
resolved by reading the root and capsule digest, not by publishing a second sequence.

### Stage 9 — prove the right level of success

The verifier separates dispatch, game mutation, and terminal goal. A designation appearing in
canonical state can prove mutation; mined tiles, safe path continuity, and resource constraints may
be required to prove the goal. Predicates name required field presence and completeness so unknown
or unsupported fields do not satisfy a negative claim. Stability predicates can require the
condition across multiple game ticks or observation capsules.

A long-running action becomes an obligation owned by a region. Its progress certificate includes
a bounded potential, regime, last material improvement, blocker evidence, and expected next
observable transition. Cancellation requests stop new work, drain in-flight bridge and publication
work, run separately authorized compensation where valid, and finalize only when terminal state is
known or explicitly indeterminate.

### Stage 10 — seal evidence and release ownership

The final receipt links the plan, rebase/merge certificate, witness roots, effect-journal entries,
bridge receipts, canonical observation capsules, graph decision witnesses, obligation history, and
postcondition proof. Evidence publication uses the same immutable-artifacts/root-last discipline.
Optional ATP custody produces repair symbols and retrievability challenges tied to the evidence
root and monotonically increasing generation.

Only after the receipt is published may leases, snapshot pins, generation pins, bridge slots,
checkpoint reservations, and region obligations be released. Region close then proves the absence
of detached work. This ordering makes “request completed” a meaningful systems statement rather
than a statement about one future resolving.

## 21.23 Why the composition is stronger than the parts

The runtime alone cannot detect a stale negative read. MVCC alone cannot prove that a DFHack call
completed. A graph algorithm alone cannot explain which equal-cost path it selected. Immutable
publication alone cannot prevent an unauthorized observer from learning hidden edges. ATP alone
cannot distinguish a recoverable byte graph from a valid fortress restore point. The bridge alone
cannot know whether a work order achieved the agent's goal.

The architecture is intentionally circular in evidence but acyclic in authority:

- canonical observations authorize what may be believed;
- witnessed plans authorize what may be attempted;
- capabilities and leases authorize who may attempt it;
- the bridge performs only the registered attempt;
- later canonical observations prove what occurred;
- derived cognition explains and accelerates the next decision;
- immutable evidence and ATP custody preserve the proof;
- none of the derived or transported artifacts can write back except through a newly admitted,
  witnessed plan.

This is the central leapfrog. It does not seek reliability by serializing every agent, copying the
whole fortress every turn, or wrapping every mutation in ceremony. It obtains concurrency from
semantic witnesses, speed from immutable sharing and tiered projections, economy from progressive
cognition and deltas, durability from content-addressed generations and repair, and honesty from
explicit indeterminate states and evidence gates.

---

# Part XXII — Owned MCP transport adoption (fastmcp_rust, MCP 2026-07-28)

## 22.1 Revision authority

This 2026-08-29 revision adopts the owned `fastmcp_rust` sibling as the MCP presentation plane
(ADR-013). It amends Part XI (MCP protocol surface) and Part XVIII (work packages) and does not
alter the fifty invariants of Part III: the transport adds no authority, cannot bypass
prepare/revalidate/commit/observe/prove, and cannot redefine canonical identity or success.

## 22.2 Decision and scope

JSON-RPC framing, stdio/HTTP transports, session lifecycle, era negotiation, and cancellation
routing are commodity plumbing with large adversarial surface and near-zero overlap with this
project's thesis. The plane is borrowed from the owned sibling; the semantics stay here.

- `dfmcp-mcp` is the only crate permitted to depend on `fastmcp-rust`.
- `dfmcp-mcp` is presentation-only: no fastmcp type crosses the intent, world, or adapter seams.
- The plane is replaceable: if the sibling violates the ADR-013 counterexamples, the seam reverts
  to a hand-rolled transport without touching any other crate.

## 22.3 The admitted profile

Machine-enforced by `architecture/dependency_allowlist.toml` (`[mcp_transport]`,
`[admitted].lock_exceptions`) and checked by `scripts/check_dependency_policy.py` and
`scripts/validate_repo.py`:

- crate `fastmcp-rust`, exact upstream revision pin;
- `default-features = false` (the `legacy-2024-11-05` graph is never compiled);
- feature `tasks` enabled; auth/JOSE/OAuth/WebSocket/proxy features forbidden;
- `prost` admitted solely as an asupersync-internal transitive (lock exception).

## 22.4 Obligations as MCP Tasks

The bounded-obligation engine (Part VIII) remains the sole authority for temporal work. The
modern Tasks surface (`tasks/get|update|cancel`) is a projection: `ServerBuilder::final_tasks`
receives an application-owned store backed by the obligation engine, so drain progress,
stability requirements, failure predicates, and compensation policy keep their dfmcp semantics.
Cancellation remains request/drain/compensate/finalize and never means record deletion.

## 22.5 Dogfooding and the upstream loop

Adopting the modern-only profile is a deliberate conformance commitment for MCP 2026-07-28:
this project is the hammer that finds fastmcp_rust's spec and feature-gating defects.
Defects are filed upstream per `docs/DOGFOODING_FASTMCP.md`, fixed upstream, and consumed here
as recorded pin bumps with conformance notes. dfmcp-side workarounds for transport defects are
prohibited unless annotated with the upstream issue and listed in the pin-history table.

The first cycle already validated the loop: with the modern-only profile, `fastmcp-server` at the
adoption pin failed to compile because `legacy_adapter_response_async` used `Legacy2024*` types
outside their `#[cfg(any(feature = "legacy-2024-11-05", test))]` gate — invisible to any
default-features build, caught immediately by this project's profile.

## 22.6 Work-package deltas

- WP-13 (MCP server) is built on the pinned fastmcp_rust plane: gate 1 is the stdio laboratory
  slice; gate 2 is session-scoped capability negotiation; gate 3 is the Tasks/obligation binding.
- New WP-21 (MCP conformance and dogfooding): conformance suite on the pinned revision, the
  upstream issue loop, and the pin-bump ledger, evidenced by TEST-023 and TEST-024.

## 22.7 Acceptance

This part is accepted when: the stdio laboratory slice compiles and passes the workspace gates on
the pinned revision; the policy checker rejects any non-conforming profile mechanically; the
first upstream defect cycle (issue, fix, pin bump, conformance note) is complete; and session-
scoped authority plus the Tasks binding land under WP-13 gates 2 and 3.

---

# Part XXIII — Agent campaign memory plane (owned Eidetic Engine sibling)

## 23.1 Revision authority

This 2026-08-30 revision adopts the owned `eidetic_engine_cli` sibling (`ee`) as the recommended
campaign-memory layer for fortress stewardship (docs/EIDETIC_MEMORY.md). It adds no crate
dependency, no schema, and no transport surface: `ee` operates entirely outside the canonical
plane, invoked by agent harnesses.

## 23.2 Doctrine

The canonical model is deliberately distinct from agent memory, which may be stale or speculative
(Part V). `ee` implements that memory layer under one inviolable rule: memory is advisory context;
it is never canonical state, never authority, and loses to any live observation on conflict.
Memories carry provenance pointers INTO canonical anchors and evidence digests; canonical evidence
never cites memory. The doctrine alignment is exact — ee's "evidence before promotion", "indexes
are derived assets", and "no silent memory mutation" mirror this project's derived-cognition and
evidence rules.

## 23.3 Workflow and scope

Per-fortress ee workspace; session start via `ee resume`/`ee pack`; session end via
`ee remember` of decisions, outcomes, and anti-patterns with anchor/evidence provenance;
`ee preflight` as advisory risk lookup only. Multi-agent shared memory (`ee team`) is gated on
WP-15 multi-agent authority. dfmcp artifacts (doctor bundles, obligation outcomes, qualification
receipts) are the intended curation inputs; an export-template work item is tracked as a bead.

## 23.4 Acceptance

Doctrine document present; campaign workflow beads tracked; canonical-plane isolation preserved
(no dependency, no schema, no authority path). Rollback is deletion of the doctrine document and
beads.
