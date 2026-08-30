# Frankenstack Deep Dive for `dwarf_fortress_mcp`

**Status:** normative design input
**Revision:** 2
**Date:** 2026-08-29


> Transport note (2026-08-29): the MCP presentation plane has since been adopted from the owned
> [`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling (ADR-013,
> `docs/FASTMCP_INTEGRATION.md`). The analysis below remains the design input for the semantic
> substrate; it predates and does not cover the transport decision.

This document records the second-pass repository investigation that followed the initial architecture. It is intentionally narrower and deeper than a feature survey. For every sibling project it asks five questions:

1. Which mechanism is genuinely load-bearing for Dwarf Fortress control?
2. What invariant does that mechanism establish?
3. Where does it belong in the crate and process topology?
4. What failure mode would a superficial imitation introduce?
5. What evidence is required before the mechanism may be called integrated?

The answer is not “depend on every Franken crate.” The answer is a **Franken substrate contract**: every imported primitive has a semantic owner, a replacement prohibition, a deterministic reference model, a failure boundary, and an admission gate. The project must benefit from the fleet’s strongest ideas without inheriting accidental coupling or pretending that unfinished sibling capabilities are already production-ready.

## 1. The resulting constitutional decisions

The deep dives imply twelve decisions that override weaker language in the first draft.

1. **`asupersync` is the only asynchronous runtime.** No Tokio, async-std, smol, Rayon, detached threads, or second cancellation model may enter the process.
2. **The live world is multi-version.** A single mutable “current snapshot” is insufficient for concurrent reads, plan validation, replay, and historical explanation.
3. **Every negative read is witnessed.** “No hostile unit exists in this region” is a predicate over an observed domain, not absence of a row in a cache.
4. **All coherent state publication is reserve → materialize → publish.** Readers never observe a root that refers to incomplete children.
5. **Plans are optimistic semantic transactions over an externally mutating world.** They carry read/write witnesses and must revalidate at commit.
6. **Dwarf Fortress effects are never conflated with ledger commits.** The protocol distinguishes request durability, bridge acceptance, observed game mutation, and verified terminal outcome.
7. **Graph results are projections with certificates.** They improve planning and attention but cannot silently become authoritative state.
8. **Every non-unique graph answer has an explicit tie-break policy.** Decision paths are fingerprinted so replay detects ordering drift.
9. **ATP is the state/evidence movement plane, not an RPC synonym.** Mutation authority never rides an eventually delivered bulk object transfer.
10. **Adaptive policy is bounded above by hard invariants.** Learning may choose how much to inspect; it may not lower safety, freshness, witness, or confirmation requirements.
11. **Dependencies form a closed, owned universe.** Outside crates require an explicit constitutional exception; convenience is not a reason.
12. **Local qualification is the release authority.** GitHub workflow YAML remains a portable executable specification for `dsr`; GitHub-hosted runner success is not required evidence.

## 2. `asupersync`: execution, authority, cancellation, and transfer

### 2.1 What the first pass missed

The first plan correctly mentioned structured concurrency, `Cx`, cancellation, deterministic testing, and ATP. It did not make them structural enough. The deeper reading shows that `asupersync` is not a bag of runtime utilities. It is a programming model in which:

- work has an owner;
- ownership follows a region tree;
- authority and budgets flow through an explicit context;
- cancellation is a protocol with a drain phase;
- effects are separated into reservation and commitment;
- progress can be certified rather than inferred from elapsed time;
- the same code can execute under production or deterministic laboratory time;
- large objects move as verified graphs, not anonymous byte streams.

Those properties should define the fortress server’s control plane.

### 2.2 Region topology

Each open fortress session owns a region. The region owns observation streams, query workers, plan compilers, committed plans, lease renewers, bridge operations, obligations, checkpoint operations, and evidence writers. A session cannot report terminal closure while one of those children is still alive or owns an unresolved effect.

The recommended topology is:

```text
process
├── authority-root
├── bridge-supervisor
│   ├── compatibility-probe
│   ├── read-pump
│   └── effect-lane
├── ledger-supervisor
│   ├── publication-coordinator
│   ├── compaction-worker
│   └── evidence-sealer
├── derived-state-supervisor
│   ├── graph-projector
│   ├── search-generation-builder
│   └── attention-ranker
├── transfer-supervisor
│   ├── checkpoint-export
│   ├── replica-ingest
│   └── retrievability-scrubber
└── session
    ├── observe-stream
    ├── query-scope
    ├── planning-scope
    └── committed-plan
        ├── lease-renewer
        ├── checkpoint-before-effect
        ├── step
        │   ├── bridge-request
        │   ├── reconciliation-watch
        │   └── terminal-obligation
        └── evidence-publication
```

Region closure is itself an obligation. Its output is a drain receipt naming completed children, forcibly abandoned children, outstanding external effects, and the last durable state anchor.

### 2.3 Context-carried authority

Every function that can block, allocate from a shared budget, touch the bridge, persist state, publish a generation, or create child work accepts a context. The context carries:

- session and request identity;
- fortress lineage and observation epoch;
- deadline and game-tick deadline;
- CPU, memory, I/O, bridge-call, output-token, and retry budgets;
- cancellation state and reason chain;
- capability mask and object/region scope;
- policy, schema, adapter, and compatibility epochs;
- trace and replay identity.

Authority is narrowed twice. Rust types remove impossible operations from an interface, while the runtime capability mask prevents ambient context recovery from regaining them. A read-only query worker cannot manufacture a write-capable bridge handle even if it can reach a parent service object.

### 2.4 Two-phase effects beyond game mutation

Reserve/commit applies to more than DF actions. It governs:

- publishing a world snapshot root;
- advancing a search generation;
- publishing graph projections;
- recording an idempotency outcome;
- sealing a checkpoint manifest;
- promoting a compatibility profile;
- emitting a public evidence bundle.

The reservation identifies the exact inputs, budget, generation, and destination. Materialization creates children under an unpublished root. Commit atomically exposes the root only after every child verifies. Cancellation before commit destroys or quarantines the reservation; cancellation after commit follows the registered compensation or reconciliation path.

### 2.5 Cancellation progress certificates

Long-running cancellation must not be an arbitrary timeout followed by process death. Each drainable subsystem defines a nonnegative potential function. Examples:

- number of undispatched plan steps plus in-flight bridge operations;
- number of unacknowledged observation frames;
- bytes of an unpublished checkpoint graph;
- count of unsealed evidence records;
- number of leases whose release has not been observed;
- count of child regions not at a terminal state.

A progress certificate records potential samples, active regime, expected descent, detected rebound, and the reason a safe drain can or cannot be guaranteed. Statistical bounds may support diagnosis, but they never authorize unsafe finalization. When a bridge effect may have happened and cannot be queried, cancellation terminates as `Indeterminate`, not `Cancelled`.

### 2.6 Deterministic laboratory inheritance

Every time source, sleep, timeout, retry schedule, jitter decision, queue handoff, lease expiry, and bridge fault must be driven through the runtime abstraction. The laboratory must explore:

- cancellation at every registered yield point;
- duplicate, delayed, reordered, and lost bridge replies;
- process death between effect reservation, dispatch, receipt persistence, observation, and proof;
- watcher lag and snapshot-generation races;
- budget exhaustion during witness refinement;
- simultaneous disjoint and overlapping plans;
- restore while stale readers and prepared plans remain alive.

A production-only thread or timer is a verification escape hatch and is forbidden.

### 2.7 ATP as a verified object-graph plane

ATP should move immutable state graphs:

- checkpoint manifests and save-file chunks;
- snapshot anchors and delta runs;
- graph/search generations;
- replay traces and crashpacks;
- benchmark corpora and evidence bundles;
- optional read replicas.

An ATP transfer begins from a domain-separated root identity. The root names a manifest; the manifest names child objects; children name chunks or repair symbols. The receiver stages objects, verifies each object identity, verifies graph closure, persists a resumable journal, and publishes the root last. Path candidates may race, but losing paths drain according to protocol so resources and partial state are not leaked.

ATP is explicitly **not** the effect transport for non-idempotent DF mutations. Bulk transfer can be resumed and repaired; game effects require request identity, fencing, lookup, and reconciliation semantics.

### 2.8 Admission gate

`asupersync` integration is admitted only when the same end-to-end scenario passes under real time and lab time; no owned work survives region close; authority narrowing is tested both statically and dynamically; cancellation certificates are emitted for every nontrivial drain; and ATP corruption, truncation, reordering, path failure, and resume campaigns preserve root integrity.

## 3. `frankensqlite`: semantic MVCC and proof-carrying plan concurrency

### 3.1 The core transfer

The most important SQLite-derived idea is not “store records in a database.” It is that concurrent semantic work should be represented through versions, witnesses, deterministic commit ordering, and refinement that is allowed to create false conflicts but never miss a real one.

A fortress is changing independently of the server. Therefore the project cannot provide ACID transactions over the game itself. It can, however, provide serializable semantics over its **observed world history and plan-commit protocol**, then reconcile the external effect boundary honestly.

### 3.2 Multi-version world state

Each observation publication creates an immutable semantic version. A state anchor includes:

```text
fortress_lineage
observation_epoch
snapshot_sequence
bridge_generation
adapter_epoch
schema_epoch
policy_epoch
state_root
```

Readers pin one anchor. A query never mixes entity revisions, map chunks, events, or indexes from different anchors unless it is explicitly a temporal query. Derived graph and search generations declare the anchor interval they cover and cannot be returned as exact for a newer anchor.

### 3.3 Plan read and write witnesses

A prepared plan records what it relied on, not merely the state hash. Witness classes include:

- entity identity, generation, and revision;
- field value or field-presence state;
- edge existence or nonexistence;
- spatial chunk revision and exact tile mask;
- path existence, path cost interval, or reachability frontier;
- aggregate quantity and contributing source set;
- negative-domain predicate, such as no hostile unit in a bounded region;
- action-registry, schema, adapter, capability, and policy epochs;
- resource reservation and lease fences;
- global clock/checkpoint state.

Write witnesses describe intended semantic effects: fields, entity generations, tile masks, relation families, resources, and global domains. They are conservative until refined.

### 3.4 Hierarchical witnesses and value of information

Witnesses form a hierarchy:

```text
fortress
└── domain
    └── region / entity kind / relation family
        └── chunk / entity / adjacency row
            └── tile mask / field / edge key
```

The coarse witness is mandatory and sound. Finer witnesses are optional accelerators. When two plans appear to conflict at a coarse level, the system estimates the value of refinement: probability that refinement proves disjointness multiplied by the value of avoiding a replan, minus compute and latency cost. Exhausting the refinement budget produces a conservative conflict. It can never produce permission to commit.

This asymmetry is essential. Performance failure may reduce concurrency; it may not create an undetected race.

### 3.5 SSI-style dangerous structures

Prepared and recently committed plans form a dependency graph from read/write conflicts. The coordinator rejects or replans transactions that form a dangerous structure under the project’s serializability profile. Negative reads participate: a plan whose validity depends on “no unit occupies these tiles” conflicts with an observation that introduces such a unit before commit.

Because Dwarf Fortress may change between the final read and effect execution, passing the SSI gate does not prove the game effect is safe forever. It proves the plan was valid at the revalidation anchor. The bridge still receives explicit preconditions and must perform a final bounded check on the game thread.

### 3.6 Deterministic commit combining

Plan commits enter a single deterministic sequencing point only for the brief publication/fence decision, not for planning or observation. Producers enqueue complete commit candidates. The combiner:

1. orders candidates by stable policy;
2. validates lease and epoch fences;
3. checks witnesses against the chosen anchor;
4. performs bounded witness refinement;
5. detects dangerous structures;
6. reserves idempotency and effect records;
7. emits either a commit ticket or a precise replan/conflict result.

Callbacks and expensive work run after releasing the queue lock. Drain-then-drop-then-process prevents reentrant deadlocks.

### 3.7 Semantic merge ladder

When current state has advanced, the system never merges raw serialized bytes. It attempts:

1. **Exact intent replay.** Recompile the original semantic intent against the new anchor.
2. **Stable-key structural merge.** Merge disjoint entity fields, tile masks, relation keys, or resource reservations using canonical ordering.
3. **Registered commutative composition.** Apply only if the action-family registry proves the operations commute under current constraints.
4. **Compensate and replan.** For a partially observed external effect, reconcile first, then generate a new plan.
5. **Reject.** Unknown or ambiguous semantics do not merge.

An accepted merge emits a certificate containing basis anchors, intent identities, conflict domains, canonical normal form, decision path, and post-plan digest. “Last writer wins” is forbidden for semantic game state.

### 3.8 Time travel and speculative branches

The append-only observation history supports `AS OF` explanation and counterfactual branches. An agent may fork a planning branch from an anchor, apply hypothetical semantic deltas, run graph algorithms, and compare outcomes. Branches never merge fabricated state into the live world. They emit candidate intents, which are compiled and validated afresh against live state.

### 3.9 E-processes and regime monitors

Sequential statistical processes may detect changes in conflict rate, bridge latency, observation churn, or witness-refinement payoff. Their role is advisory: choose batching, refinement depth, or backoff. They are not correctness oracles. If a monitor is absent, reset, or uncertain, the safe baseline remains valid.

### 3.10 Admission gate

Admission requires differential execution against a single-threaded reference ledger, schedule exploration, crash injection at every publication boundary, negative-read phantom tests, deterministic rebase equivalence, proof-certificate replay, and a demonstrated rule that refinement exhaustion increases false positives only.

## 4. `frankenfs`: coherent publication, checkpoint custody, and evidence discipline

### 4.1 Filesystem state is an effect domain

Save files, checkpoint bundles, compatibility manifests, evidence, and exported generations are not “just files.” Every filesystem operation occurs under a rooted capability that defines allowed roots, path classes, symlink policy, size budget, durability policy, and publication rules.

The safe-Rust server must not claim race-free path confinement that the underlying interface cannot establish. Until the appropriate FrankenFS capability is admitted on a platform, high-risk filesystem operations remain disabled or require a dedicated helper with explicit evidence.

### 4.2 Coherent multi-artifact publication

Checkpoint publication follows:

1. enumerate all intended outputs;
2. preflight destinations and identities;
3. stage every child under an unpublished generation;
4. fsync/verify according to policy;
5. compute the manifest and root seal;
6. atomically expose the root pointer;
7. retire or retain the prior root;
8. emit a publication receipt.

A manifest is never published before its children. A partial directory is not a checkpoint. Human-readable reports, machine JSON, signatures, and repair symbols are siblings in one publication transaction.

### 4.3 Ownership, generations, and fences

Checkpoint and repair work carries an ownership lease with incarnation and generation. A restarted worker cannot publish an old staged result after a newer worker acquires the lease. Remote destinations enforce monotonic generation and reject rewrite of an already sealed object identity.

### 4.4 Evidence bundles and readiness dimensions

The project reports readiness separately:

- contract implemented;
- deterministic reference verified;
- filesystem fault verified;
- adapter verified;
- live-game read verified;
- live-game effect verified;
- cross-version compatibility verified;
- performance verified;
- recovery verified.

A test that only proves rejection of malformed input is **negative evidence**. It is valuable but does not count as proof that the success path works. Release claims must cite the exact dimensions earned.

### 4.5 Same-binary experiments

Performance experiments use one binary with runtime-selected arms. The request, input root, semantic output digest, and receipt schema are identical. Timing begins only after semantic equivalence is established. This applies to:

- snapshot representations;
- delta codecs;
- graph projection strategies;
- query indexes;
- bridge batching;
- checkpoint compression;
- cache replacement;
- ATP chunking and repair overhead.

An optimization without an A/A null, correctness digest, distributional results, and workload manifest remains a hypothesis.

### 4.6 Cache and I/O policy

The server distinguishes staged from committed data. Cache entries are keyed by generation and cannot make unpublished data visible. Request coalescing joins identical reads without coalescing distinct authority or freshness requirements. Compression, alignment, direct I/O, and batch sizes are workload policies, not universal truths.

### 4.7 Repair and retrievability

Doctor produces findings; repair planning transforms findings into an immutable plan sealed to the current root; apply revalidates that root before mutation. Retained checkpoint/evidence graphs may carry repair symbols and support probabilistic proof-of-retrievability sampling. A failed audit schedules repair or re-replication; it does not silently bless the object.

### 4.8 Admission gate

Admission requires a complete crash matrix, path-attack corpus, generation-fence tests, same-binary A/B receipts, checkpoint restore into a new observation epoch, repair-plan stale-root refusal, and independent reconstruction of every advertised root from its manifest.

## 5. `frankensearch`: progressive cognition with immutable generations

### 5.1 Retrieval is a bounded decision process

Agents should not receive a raw world dump. Query execution first returns a cheap, deterministic candidate set, then refines when the expected decision benefit justifies cost. Every stage is useful on its own, carries the same pinned anchor, and may terminate on budget exhaustion without lying about completeness.

### 5.2 Immutable generation per request

A query pins one search generation. Activation proceeds build → verify → continuity-check → publish root → retire old. A request cannot observe half of two generations. The generation declares source anchor range, schema epoch, analyzer policy, model identity if any, and exact-document count.

Mutable ingestion uses lease-bounded append segments. Segments have sequence order and memory limits. Freezing creates an immutable generation; overflow creates backpressure or spills through a registered path rather than growing without bound.

### 5.3 Progressive query ladder

The default ladder is:

1. exact typed filters and stable identifiers;
2. lexical retrieval over canonical text and event fields;
3. graph expansion from high-confidence seeds;
4. structured reranking from urgency, causal proximity, confidence, novelty, and actionability;
5. optional semantic/vector reranking when an approved local model exists;
6. explanation and evidence shaping to the token budget.

Each stage records candidate count, score components, pruning, freshness, and stop reason. A later stage may reorder but cannot erase provenance.

### 5.4 Decision plane and adaptation

Adaptive policies are keyed by workload class. They have explicit priors, clamps, minimum sample counts, circuit breakers, and evidence events. They may tune candidate budgets, observation frequency, cache admission, or witness-refinement effort. They may not:

- accept stale state beyond the caller’s limit;
- drop required evidence;
- weaken a capability or risk check;
- convert unknown completeness into complete;
- change deterministic tie-break policy without a policy epoch;
- authorize an effect.

### 5.5 Recall and absence claims

Top-k quality and absence claims are distinct. “No relevant result exists” requires a coverage certificate for the authorized domain and pinned generation. When the engine cannot certify recall, it reports `uncertified` rather than manufacturing confidence. Security scopes apply before retrieval so counts and absence cannot leak unauthorized state.

### 5.6 Admission gate

Admission requires exact generation activation/rollback, deterministic top-k, source-span reconstruction, stale invalidation, bounded malformed-input handling, fallback behavior without embeddings, score-ledger replay, and certificates that fail closed when coverage cannot be established.

## 6. `franken_markdown`: exact knowledge, bounded protocol, and publication

### 6.1 Span-preserving knowledge

Manuals, DFHack docs, mod documentation, runbooks, action-family notes, and agent journals are parsed into a typed arena with exact byte spans. Derived chunks preserve source identity and transformation lineage. A citation names corpus generation, document digest, span, parser policy, and any normalization.

Text can influence planning but cannot grant capability. Prompt-like text from the game or corpus retains taint and provenance through retrieval and summarization.

### 6.2 Nonrecursive bounded structures

DfQL predicates, MCP JSON, and knowledge ASTs should use arenas and explicit stacks rather than recursive descent over attacker-controlled depth. Limits cover bytes, nesting, members, strings, arrays, nodes, diagnostics, and output. Strict mode rejects malformed input. Any hardened recovery is bounded and emits a decision record.

### 6.3 Direct MCP implementation

The project does not need a web framework to speak local stdio MCP. A narrow protocol crate can implement:

- bounded JSON parsing through the approved serialization layer;
- JSON-RPC request/response identity;
- MCP initialization and capability negotiation;
- tool/resource/prompt enumeration;
- progress notifications;
- cancellation mapping into `Cx`;
- deterministic error serialization;
- strict content-length or newline framing, depending on selected transport.

Network transports remain outside the initial trust boundary. This avoids importing a second runtime and large HTTP dependency graph.

### 6.4 Multi-output publication

Knowledge generation, search index, citation map, diagnostics, and human report publish as one sibling set. Output identity is preflighted before any destination changes. Publication failure restores or retains the prior root.

### 6.5 Admission gate

Admission requires byte/span round trips, incremental/full equivalence, stable citation behavior, parser depth and size campaigns, malicious text taint preservation, all-or-nothing sibling publication, and direct MCP conformance without hidden background work.

## 7. `frankengraphdb`: one version universe without putting a database in the effect path

### 7.1 Most valuable ideas

The deep value is the composition:

- one append-only version universe for history, replication, subscriptions, and branches;
- temperature-tiered graph storage;
- factorized and worst-case-optimal query execution;
- incremental Z-set maintenance;
- snapshot-pinned graph views;
- deterministic plan certificates;
- branch-per-agent speculation;
- capabilities compiled into the planner before expansion;
- reference semantics and deterministic simulation.

`dwarf_fortress_mcp` should import the semantic architecture while keeping the live effect path simpler.

### 7.2 One version universe

Observation capsules are the common delta stream for:

- canonical history;
- current projections;
- graph updates;
- search updates;
- subscriptions;
- replay;
- branch creation;
- remote read replicas.

Every derived system consumes the same ordered capsules and publishes a root that names its consumed high-water mark. There is no separate “eventually updated” world with untraceable provenance.

### 7.3 Temperature-tiered world graph

World graph adjacency should migrate by workload:

- tiny hot adjacency inline with the entity projection;
- recent mutable deltas in sorted bounded blocks;
- cold stable adjacency in sealed compressed runs;
- historical anchors retained according to policy.

This is especially relevant to Dwarf Fortress’s power-law-like relations: most entities have small local neighborhoods, while map connectivity, jobs, stocks, and social relations contain hot hubs. One representation for all of them wastes memory or update cost.

### 7.4 Factorization and incremental maintenance

Queries such as “which workshops are blocked through any chain of missing inputs and inaccessible stockpiles?” can explode if intermediate paths are materialized. The query layer should preserve factorized sets and use worst-case-aware joins. Standing predicates and attention candidates should update incrementally from observation capsules, not rerun full scans every tick.

The initial implementation can be simple ordered maps plus reference operators. Optimized graph storage is admitted only behind equivalent semantics and measured workload gates.

### 7.5 Branch-per-agent planning

Each agent may own a cheap logical branch rooted at a pinned anchor. Hypothetical intents create semantic deltas on the branch. Graph and resource analysis runs over the branch. Merge means “produce a candidate live intent and conflict report,” not “write branch bytes into Dwarf Fortress.” This preserves isolation and makes multi-agent comparison cheap without pretending the game supports transactions.

### 7.6 Planner-enforced capability scope

Authorization is applied before graph expansion. A capability restricted to a burrow, entity set, or relation family cannot infer hidden neighbors through degree, reachability, counts, or absence witnesses. Query certificates name the authorized projection, not the global graph.

### 7.7 Admission gate

Admission requires a reference graph oracle, snapshot-pinned zero-copy views, incremental/full equivalence, deterministic factorized outputs, bounded expansion, capability noninterference, branch isolation, and a demonstrated benefit before replacing simpler projections.

## 8. `franken_networkx`: graph algorithms, canonical choices, and complexity witnesses

### 8.1 Why this changes the design

Dwarf Fortress is not merely graph-shaped storage. It contains operational graphs whose algorithms directly improve safety and efficiency. FrankenNetworkX also treats iteration order, tie-breaks, failure modes, and observed complexity as contracts. Those ideas belong in the planning kernel.

### 8.2 Canonical graph semantics

Every algorithm family declares:

- graph projection and anchor;
- directedness, multiedge, and weight semantics;
- deterministic tie-break policy;
- numeric policy and overflow behavior;
- dominant complexity bound;
- resource budget;
- output ordering;
- decision-path digest;
- stale-result policy.

Equivalent mathematical answers are not operationally equivalent when agents replay plans. A shortest path with equal cost must choose by a declared policy such as insertion order or stable entity identity. Hash iteration is never a policy.

### 8.3 Algorithm families most accretive to fortress control

The following are not decorative analytics; each maps to a decision:

- **Dynamic connectivity:** can a dwarf, item, fluid, invader, or cart reach a target after a designation or construction?
- **Articulation points and bridges:** which doors, stairs, ramps, corridors, bridges, or power links are single points of failure?
- **Strongly connected components:** where do job, hauling, production, lease, or dependency cycles create deadlock?
- **Condensation DAG and topological order:** what is the executable order of a production or construction plan?
- **Dominators:** which workshop, stockpile, stairwell, power component, or resource dominates all routes to an objective?
- **Shortest and k-shortest paths:** what route is cheapest, and what fallback remains if the primary path closes?
- **Multi-source distance:** which hospital, stockpile, meeting area, barracks, or refuge best serves current demand?
- **Max flow / min cut:** what is corridor, hauling, fluid, or defensive capacity; which cut is the smallest failure or defense set?
- **Min-cost flow:** how should bounded items or transport capacity satisfy demands at minimal cost?
- **Bipartite and weighted matching:** which dwarves, workshops, beds, squads, or hauling vehicles should be assigned to which jobs?
- **Spanning forests:** what low-cost infrastructure connects required components without redundant construction?
- **Centrality and community:** which entities deserve observation attention; these are advisory, never effect authorization.
- **Temporal reachability:** did a causal path exist at the anchor when a plan was prepared?
- **Plan-DAG critical path:** which obligation controls completion time and where would parallelism help?

### 8.4 Zero-copy snapshots and invalidation

Algorithms operate over immutable snapshot views shared by reference. Cloning a view must not deep-copy the graph. A view carries anchor and projection generation. Mutation publishes a new generation; old readers remain coherent. Iterators that promise live fail-fast behavior check revision and return the registered error on change.

### 8.5 Complexity witnesses

Every planning-relevant graph execution emits a compact witness:

```text
algorithm_id
projection_id
anchor
n, m
policy_id
observed_operation_counts
budget_consumed
decision_path_digest
output_digest
```

The witness detects accidental complexity regressions and nondeterministic tie-break drift. Instrumentation is not itself a correctness requirement; the selected result and policy are. Failure to record optional performance evidence cannot mutate the answer.

### 8.6 Admission gate

Algorithms are ported or consumed only after differential tests against the reference implementation, adversarial graph families, tie-break fixtures, budget cancellation, snapshot invalidation tests, and complexity-witness regression locks. The core must not pull Python or PyO3 into the server.

## 9. `doodlestein_self_releaser`: local qualification is part of the architecture

The release path is not clerical machinery outside the system. A project whose correctness depends
on exact nightly behavior, owned sibling revisions, deterministic artifacts, native operating-
system behavior, and live DFHack compatibility cannot treat a green hosted workflow badge as its
trust root.

### 9.1 One specification, local execution

Workflow YAML remains useful as a portable job graph. `doodlestein_self_releaser`, `act`, and
controlled native hosts can execute that graph without queue availability or hosted-runner state.
The repository's direct local qualifier is the semantic source of truth; workflows invoke it
rather than duplicating a second set of checks.

The imported rule is:

```text
one clean source identity
+ one locked owned-dependency closure
+ one qualification contract
+ platform-specific native execution
= comparable release evidence
```

A GitHub-hosted run may be informative, but it cannot be required for build, test, qualification,
or publication and it cannot override a failing local receipt.

### 9.2 Clean snapshots and sibling closure

A release begins from a clean commit, not the developer's mutable checkout. Every live
`asupersync` or Franken-suite path dependency must be copied into the release snapshot at an exact
clean revision and named in the build manifest. Cargo resolution is locked and checked offline.
This prevents a binary from being attributed to the main repository commit while silently
incorporating uncommitted or later sibling code.

The source manifest covers toolchain identity, target, profile, feature set, environment inputs,
Cargo lock digest, architecture-registry roots, sibling revisions, and qualification receipt. A
rebuild claim is invalid unless these identities match.

### 9.3 Partial builds are retained but never blessed

Multi-platform work is naturally failure-prone. Completed target artifacts may be retained across
a resumed run, but the authoritative release manifest is withheld until every required target and
every cross-target invariant passes. Resume retries only incomplete or invalid targets and verifies
previous outputs before reuse.

This mirrors root-last publication elsewhere in the architecture: staged files can exist without
becoming a release. Publication of the release root is the semantic commit point.

### 9.4 Exact asset and custody contract

Each target maps to one exact primary asset name and required checksum/signature siblings. The
release also carries an SBOM, source snapshot, qualification manifest, and public verification
instructions. Upload is followed by download-and-verify, so success means the bytes users can
retrieve match the locally qualified bytes.

The DSR admission gate requires:

1. controlled Linux, macOS, and Windows hosts;
2. a clean source and sibling revision closure;
3. latest-nightly identity recorded in every receipt;
4. locked/offline dependency resolution;
5. complete checksums, signatures, SBOM, and qualification receipts;
6. exact release-asset enumeration with no discovery-based extras;
7. resume and interruption tests that never bless a partial matrix;
8. publication and download verification without GitHub-hosted runner dependence.

## 10. Cross-project composition

The imports compose into three planes.

### 9.1 Authoritative plane

Owned by the runtime, ledger, world MVCC, publication coordinator, checkpoint custody, and evidence system. It contains canonical facts, versions, intents, plans, effects, obligations, and receipts. It is the only plane allowed to answer “what did the system observe or do?”

### 9.2 Cognition plane

Contains graph projections, search generations, knowledge spans, attention scores, counterfactual branches, and adaptive policies. Everything is derived from named authoritative anchors. It may produce candidate intents and explanations. It cannot directly dispatch effects.

### 9.3 Effect plane

Contains compatibility probing, game-thread precondition checks, typed mutation batches, operation lookup, observation of effects, and checkpoint/restore coordination. It is narrow, fenced, and distrustful. It cannot redefine canonical semantics.

## 11. Explicit rejections

The following tempting designs are rejected:

- one giant MCP tool per DFHack command;
- arbitrary shell or Lua execution exposed to an agent;
- screenshot-only state;
- a mutable global world cache;
- raw-byte merge or last-writer-wins plans;
- using a graph database as the sole system of record before reference semantics exist;
- sending mutation commands over ATP;
- treating an accepted bridge call as completed game work;
- detached observers or retry loops;
- opaque third-party runtimes, graph engines, SQL clients, search engines, or RPC stacks;
- adaptive safety thresholds;
- “latest state” reads that silently mix generations;
- release claims based only on GitHub-hosted CI badges;
- benchmark wins without same-binary semantic receipts.

## 12. Integration sequence

The correct order is:

1. freeze identities, anchors, effect outcomes, and publication contracts;
2. build a single-threaded reference world history and graph oracle;
3. execute the existing deterministic in-memory adapter through the new contracts;
4. introduce `asupersync` ownership and lab execution;
5. add multi-version world publication and witnesses;
6. add a read-only DFHack bridge and differential snapshots;
7. add derived graph/search generations;
8. add one reversible effect family with full prepare/observe/prove semantics;
9. add checkpoint custody and restore epochs;
10. admit persistent Franken adapters one at a time through evidence gates;
11. add multi-agent branches and leases;
12. optimize only after workload receipts identify the wall.

This order deliberately builds the semantic oracle before the optimized substrate. Otherwise every performance result risks accelerating the wrong behavior.
