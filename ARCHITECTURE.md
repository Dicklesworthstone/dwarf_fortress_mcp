# Architecture

This is the compact engineering map. `IMPLEMENTATION_STATUS.md` defines what is currently
established. The comprehensive plan and machine registries are normative when detailed target
semantics differ.

## Thesis

`dwarf_fortress_mcp` is a safe-Rust, latest-nightly semantic operating substrate for agents
stewarding a Dwarf Fortress simulation. MCP is the narrow presentation waist. Canonical state is a
versioned, provenance-carrying world history. Graph, search, attention, recommendations, and memory
are derived cognition. DFHack is an external, fenced native boundary. Compatibility and executable
admission are explicit evidence systems, not deployment folklore.

The system has one agent-facing center: the control loop in
`docs/AGENT_OPERATING_MODEL.md`. Every plane exists to make that loop more truthful, economical,
safe, replayable, and accretive. No subsystem may create a competing lifecycle or force an agent to
reconstruct protocol state from unrelated outputs.

## Current executable slice versus target

### Current source slice

```text
modern-only MCP 2026-07-28
+ frozen eleven-tool waist
+ deterministic laboratory effects
+ authenticated read-only DFHack bridge
+ complete citizen-roster observation
+ canonical live capsule and graph projection
+ read-only live observe/query/wait/explain/doctor
+ exact compatibility registry and resolver
+ owner-private monotonic registry floor
+ authority-free admission doctor
+ source-bound server receipt
+ descriptor-bound launcher
+ single-use Rust admission ticket
```

The checked-in registry contains no entries. Consequently no current live tuple is admitted and the
launcher cannot authorize a live process from the repository as checked in. No live mutation method
exists.

### Target system

The target extends the same boundaries rather than bypassing them:

- wider canonical observation domains;
- witnessed semantic mutation;
- durable MVCC and effect custody;
- admitted Franken graph/search/filesystem/transfer substrates;
- objective decomposition and counterfactual comparison;
- evidence-gated campaign memory;
- distributed transfer and exact release qualification.

## Agent control loop

```text
bootstrap → orient → inspect → formulate → propose → compare
          → commit → verify → learn → handoff/resume
                         ↘ reconcile ↗
```

Every success and error converges on a canonical Agent Turn Packet:

```text
identity + exact anchor + continuity
briefing + semantic changes + ranked attention
active work + legal affordances + ranked next protocol steps
uncertainty + coverage + budget + typed references
```

The packet is an orientation spine, never a second source of truth. After admitted live startup, it
also carries the exact entry, registry, decision, monotonic-floor, server-receipt, launch, ticket,
and executable digests that explain the process’s read-only admission.

## Four cooperating boundaries

The target retains three semantic planes plus an explicit deployment-admission boundary.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ PRESENTATION / SESSION                                               │
│ MCP · sessions · capabilities · budgets · Agent Turn Packet          │
│ profiles · affordances · recommendations · typed recovery            │
└──────────────────────────────────────────────────────────────────────┘
                 │ reads / intents                   │ evidence
                 ▼                                   ▲
┌──────────────────────────────────────────────────────────────────────┐
│ AUTHORITATIVE PLANE                                                  │
│ observation capsules · versioned world · witnesses · plans           │
│ idempotency · leases/fences · effects · obligations · checkpoints   │
│ evidence · compatibility/schema/policy epochs                        │
└──────────────────────────────────────────────────────────────────────┘
        │ pinned immutable generations            │ short-lived ticket
        ▼                                         ▼
┌──────────────────────────────────┐  ┌────────────────────────────────┐
│ COGNITION PLANE                  │  │ EFFECT / BRIDGE PLANE          │
│ graph and spatial projections    │  │ typed DFHack reads/effects     │
│ search/knowledge generations     │  │ game-thread preconditions      │
│ attention and counterfactuals    │  │ operation lookup/reconcile     │
│ candidate intents only           │  │ canonical post-state reads     │
└──────────────────────────────────┘  └────────────────────────────────┘
                 ▲                                   ▲
                 └──────── evidence identities ──────┘
                                      │
┌──────────────────────────────────────────────────────────────────────┐
│ DEPLOYMENT ADMISSION                                                 │
│ R1-R5 registry · monotonic floor · exact decision · server receipt   │
│ executable identity · loader hygiene · single-use process ticket     │
└──────────────────────────────────────────────────────────────────────┘
```

The cognition plane has no path to effect dispatch. The bridge cannot redefine canonical success.
The deployment-admission boundary can authorize one exact process posture but cannot grant game
mutation capability absent from the registry and bridge protocol.

Outside the semantic planes sits advisory campaign memory through the owned
`eidetic_engine_cli` sibling. Memory cites anchors and evidence digests; canonical state and
authority never cite memory as proof.

## Current read-only bridge

Protocol V1 exposes exactly:

```text
Handshake
ReadObservation
```

The plugin uses DFHack’s supported native protobuf RPC service on loopback, with no remote flag and
no arbitrary command, Lua, keyboard, path, or mutation route. The Rust trust domain uses no C/C++
FFI and no direct memory scraping.

The Rust client validates bounds, canonical protobuf encoding, duplicate fields, UTF-8, nonce,
versions, method manifest, bridge generation, ordering, offsets, completeness, projection, and text
notification budgets. A failed transport is permanently fenced for the session.

A complete read becomes one immutable `LiveObservationCapsule`. Pagination is transport, not
semantics: one-page and contiguous multi-page representations of the same paused fortress produce
identical canonical bytes and the same digest. Multi-page publication while the game is running is
rejected.

The current projection contains:

- fortress identity, version manifest, clock, pause state, and citizen count;
- one canonical unit entity per fully covered citizen;
- deterministic membership edges;
- citizen identity, profession, position, and basic status;
- conditional citizen names;
- explicit omitted coverage for items, jobs, map, economy, detailed welfare, military, and history.

## One version universe

The append unit is an immutable observation capsule. The ordered capsule stream drives current
state, history, graph/search updates, subscriptions, branches, replay, checkpoint cutoffs, evidence,
briefings, attention, affordance validity, and handoff anchors. Every derived generation publishes
its source high-water mark.

A target version-2 anchor is conceptually:

```text
(fortress_lineage,
 observation_epoch,
 snapshot_sequence,
 game_tick?,
 bridge_generation,
 adapter_epoch,
 schema_epoch,
 policy_epoch,
 semantic_world_root)
```

A request reads one complete anchor. Restore, incompatible world switch, game-clock regression, or
ambiguous bridge restart creates or requires a new observation epoch. The current live adapter
implements heartbeats, ordinary sequence advancement, bridge-restart reset, clock-regression reset,
and world/version switch refusal over the V1 read slice.

## Observation publication

```text
reserve successor
→ normalize and validate capsule
→ materialize immutable children
→ verify semantic root
→ publish root/high-water mark atomically
→ notify derived consumers
```

Readers see the old or new root, never a partially assembled generation. Deltas require exact basis
identity and reconstruct the successor root. `continuous`, `heartbeat`, `partial`, `gap`, `reset`,
`stale`, and `indeterminate` are protocol facts, not presentation guesses.

## Exact compatibility admission

Source presence is not compatibility evidence. One live tuple requires:

```text
exact clean dfmcp source
+ exact DFHack source
+ exact plugin SHA-256
+ R1 native build and inventory
+ R2 authentication/non-disclosure
+ R3 deterministic complete reads
+ R4 restart/drift/gap fencing
+ R5 cold-agent orientation
→ one content-addressed experimental registry entry
```

The resolver binds the complete canonical registry digest, exact manifest, and explicitly required
entry ID. A match under another entry ID fails closed.

The current checked-in registry is empty. That is an operational fact, not a placeholder success.

## Monotonic local custody

A deployment host accepts registry generations through an owner-private monotonic floor. The floor
binds exact registry file bytes, canonical registry digest, ordered entry IDs, monotonic sequence,
and previous floor digest.

```text
absolute path
+ real 0700 parent
+ regular non-symlink 0600 file
+ root/effective-user ownership
+ exclusive initialization
+ lock and expected-file-digest CAS
+ atomic fsynced replacement
+ prior entry-ID preservation
```

This prevents an older but valid registry from silently replacing a newer trusted generation for
callers that require the floor. It is not distributed consensus, compatibility evidence, or a
defense against compromise of the owning account/root.

## Authority-free diagnosis

The admission doctor evaluates, in fixed deterministic order:

```text
registry
→ compatibility floor
→ exact tuple resolution
→ optional source-bound server artifact
```

It can report `compatibility_ready` or `artifact_preflight_ready`. It does not read a bridge token,
connect to DFHack, execute the server, modify registry/floor custody, or grant capabilities. Its
canonical report digest covers every field except itself.

## Server artifact and process admission

A release-server receipt binds:

- exact clean source commit;
- complete passing local qualification gate order;
- exact source-file digest map;
- platform and toolchain;
- executable `contract`, `doctor`, and `demo` checks;
- executable size and SHA-256;
- empty mutation capability.

The Python launcher verifies the trusted floor and registry, exact decision, source receipt, loader
environment, and an already-open no-follow executable. It re-reads the floor/registry after artifact
verification and immediately before execution, and re-hashes the opened executable before ticket
issue and before descriptor-only `execve`.

The owner-only ticket binds process ID, expiry, entry, registry, decision, floor file/content/
sequence, receipt, launch digest, executable metadata/SHA, read-only capabilities, and empty
mutation capability. The Rust process revalidates and hashes the current executable, consumes and
deletes the ticket, then starts the private live MCP server. Direct `serve-live` bypass fails.

## Witnessed mutation target

The future effect chain remains:

```text
semantic objective / intent
→ compile against pinned anchor
→ read/write/negative witnesses
→ immutable plan digest
→ capability/risk/budget validation
→ lease incarnation and fencing
→ checkpoint where required
→ commit-time anchor selection
→ hierarchical conflict refinement
→ dependency-cycle/SSI gate
→ short-lived effect ticket
→ bridge game-thread precondition check
→ durable dispatch-attempt record
→ typed operation dispatch/lookup
→ canonical observation
→ terminal predicate proof
→ obligation discharge or reconciliation
→ prediction-versus-observation surprise record
```

No transport acknowledgement is semantic success. Unknown dispatch outcome remains indeterminate
until operation lookup and authoritative observation resolve it. This target must not be smuggled
into read-only bridge V1.

## Semantic concurrency

Witnesses cover entity generations and fields, relation domains, tile masks, aggregates and their
contributors, absence predicates over complete domains, graph facts, resource reservations,
leases, and epochs. Coarse witnesses are sound. Optional fine witnesses may prove disjointness.
Exhaustion may create a false conflict but can never authorize unsafe overlap.

Rebase recompiles intent. Concurrent merge uses exact replay, stable-key structural composition,
registered commutativity, explicit ordering with re-proof, or rejection. Raw-byte and
last-writer-wins merge are forbidden.

## Owned execution

`asupersync` is the exclusive async runtime. Sessions, plans, bridge operations, obligations,
checkpoint work, graph/search builders, ATP transfers, evidence writers, briefing builders,
standing watches, and memory exporters are region-owned. Every effectful function receives
context-carried authority, deadlines, multidimensional budgets, cancellation, and replay identity.

Cancellation is request, drain, reconcile or compensate, finalize. Session close cannot silently
abandon owned work.

## Epistemic separation

Agent-visible claims are classified as:

```text
observed
certified_derived
inferred
predicted
assumed
stale
unknown
contradicted
indeterminate
```

Only observed and eligible certified-derived facts may satisfy mutation preconditions. Confidence
is orthogonal to class. Empty results prove absence only with complete-domain coverage.

## Graph and cognition

Anchor-bound graph projections represent traversal, logistics, production, power/fluid/mechanisms,
social/welfare, threat/defense, plan/evidence relations, objectives, and active-work dependencies.
Planning algorithms declare numeric policy, tie-breaks, budgets, output order, digest, and
complexity/decision witnesses.

Search and knowledge use immutable generations activated only after verification. Progressive
refinement can stop on budget while preserving completeness status. Adaptive policies may choose
effort but cannot weaken safety, compatibility, custody, or evidence.

The cognition plane produces bounded:

- **attention:** what matters now, with score ledger, consequence, expiry, and evidence;
- **affordances:** what actions are expressible under state, capability, risk, and compatibility;
- **recommendations:** informational or control protocol steps with utility, information value,
  cost, risk, reversibility, invalidators, and confirmation requirements.

These projections share one anchor and deterministic policy. They never issue effect tickets.

## Objectives, counterfactuals, and accretion

An objective is distinct from an intent, plan, action, obligation, or memory item. It declares
desired predicates, hard constraints, soft utility, horizon, priority, ownership, and evidence
requirements. Decomposition is deterministic and inspectable.

Counterfactual branches are immutable structurally shared derived worlds. They compare candidates
but cannot become canonical state or authority.

Execution records predicted and observed deltas. Material divergence emits a surprise record. Raw
episodes become semantic, procedural, policy, or negative memory only through evidence-gated
promotion. Memory cites canonical anchors; canonical state never cites memory as authority.

## State and evidence movement

ATP moves immutable object graphs: checkpoints, capsule runs, derived generations, crashpacks,
qualification receipts, handoff packets, and evidence-linked memory exports. Children verify before
the root publishes. Transfers can resume, race paths, and repair symbols. Mutation authority never
rides ATP.

## Trust boundaries

### Safe-Rust domain

MCP server, semantic core, runtime adapters, world state, graph/query engines, policy, evidence,
transfer, and bridge client use `unsafe_code = "forbid"`.

### Game/native domain

Dwarf Fortress and DFHack are external. The Rust process uses no C/C++ FFI or direct memory
scraping. The current bridge is bounded, authenticated, versioned, loopback-only, and read-only.

### Deployment custody domain

Registry, monotonic floor, source receipt, executable descriptor, loader environment, and ticket are
separate inputs. Every transition revalidates the identities it depends on. Source-text corruption,
permissive custody, symbolic links, stale generations, executable byte drift, and path fallback are
failures.

### Untrusted content domain

MCP arguments, bridge frames, in-game text, mod text, documentation, agent notes, imported memories,
and model rationales are untrusted. Text may provide evidence; it cannot grant authority or become
executable code.

## Reference before optimization

The first implementation for every semantic subsystem is simple and deterministic. Optimized
Franken adapters are admitted only after differential, crash, cancellation, compatibility,
Agent Turn, and performance gates. Derived state can always be rebuilt from authoritative capsules.

## Dependency policy

The universe is `core`/`alloc`/`std`, `asupersync`, admitted Franken crates, the owned pinned
`fastmcp_rust` plane, and rare fundamental exceptions in the allowlist. There is no second async
runtime, external graph engine, C SQLite, broad web/RPC framework, non-owned MCP framework, or
search engine in the production trust domain.

## Current crate graph

```text
dfmcp-core → dfmcp-world → dfmcp-intent → dfmcp-adapter → dfmcp-lab
      └────────────────────────────────────────────→ dwarf-fortress-mcp
dfmcp-world → dfmcp-mcp → dwarf-fortress-mcp
dfmcp-mcp → fastmcp-rust (owned sibling, modern-only MCP 2026-07-28, pinned revision)
```

`dfmcp-mcp` is presentation-only and replaceable. A crate split must establish a dependency, trust,
or verification boundary. Agent Turn presentation belongs at the MCP seam; source facts and
decisions remain typed below it.

## Non-bypassability

An implementation is invalid if any path can:

- mutate without plan, capability, idempotency, fence, and evidence checks;
- start live mode without exact registry, trusted floor, receipt, descriptor, and ticket proof;
- fall back to path execution after qualifying another inode;
- verify only metadata while executable bytes can change;
- silently roll back the accepted registry generation;
- read mixed generations or apply a delta across a gap;
- ignore a negative-read phantom;
- retry an indeterminate effect blindly;
- execute arbitrary text;
- publish a root before its children;
- restore without a new epoch;
- use adaptive policy, memory, attention, or recommendations as authority;
- satisfy a precondition from inference or prediction;
- prove absence from incomplete coverage;
- hide active work across handoff;
- close a region while owned work remains.
