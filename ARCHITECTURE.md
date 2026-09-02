# Architecture

`IMPLEMENTATION_STATUS.md` defines what is currently established. This document is the compact
system map. The comprehensive plan and machine contracts remain normative for detailed target
semantics.

## Thesis

`dwarf_fortress_mcp` is a safe-Rust, latest-nightly semantic operating substrate for agents
stewarding a Dwarf Fortress simulation. MCP is a narrow presentation waist. Canonical state is a
versioned, provenance-carrying world history. Graphs, search, attention, recommendations, and memory
are derived cognition. DFHack is an external, fenced native boundary. Compatibility, bridge
protocol identity, executable qualification, and process admission are explicit evidence systems,
not deployment folklore.

The system has one agent-facing center: the control loop in `docs/AGENT_OPERATING_MODEL.md`. Every
subsystem exists to make that loop more truthful, economical, safe, replayable, and accretive.

## Current source slice

```text
modern-only MCP 2026-07-28
+ frozen eleven-tool waist
+ deterministic laboratory effects
+ authenticated protocol-1.0 citizen reads
+ implemented protocol-1.1 retained-announcement reads
+ canonical live capsules and graph projections
+ protocol-1.1 transactional publication and single-read bootstrap
+ read-only live observe/query/wait/explain/doctor
+ exact compatibility registry and resolver
+ owner-private monotonic registry floor
+ authority-free admission doctor
+ source-bound server receipt
+ protocol-bound V2 launch and ticket
+ descriptor-only production execution
```

The checked-in compatibility registry has zero entries. Consequently no current live tuple is
admitted. Protocol 1.1 has a separately named, exact-opt-in development MCP runtime but is absent
from the production runner map. No live mutation method exists.

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

After admitted startup, the packet also carries the exact bridge protocol, ticket, registry entry,
registry generation, decision, monotonic floor, server receipt, launch, and executable identities.
This is explanatory provenance, never new authority.

## Four authority-separated boundaries

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
        │ pinned immutable generations            │ effect tickets
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
│ R1-R5/A1-A6 evidence · registry · monotonic floor · exact protocol   │
│ server receipt · executable identity · loader hygiene · V2 ticket    │
└──────────────────────────────────────────────────────────────────────┘
```

The cognition plane cannot dispatch effects. The bridge cannot redefine canonical success. The
deployment boundary may authorize one exact process/protocol posture but cannot grant a game
capability absent from the compatibility entry and bridge method set.

## Protocol generations

### Protocol 1.0

Protocol 1.0 exposes exactly:

```text
Handshake
ReadObservation
```

It authenticates over numeric loopback, reads fortress summary and complete bounded citizen roster,
and contains no mutation route. A complete read becomes one immutable citizen observation capsule.
Equivalent transport pagination produces identical canonical identity.

### Protocol 1.1

Protocol 1.1 retains the same method names but is a different generation identified by protocol
version, bridge version, plugin digest, source commits, and platform. Method names alone never imply
compatibility.

It adds bounded retained-announcement fields inside `ReadObservation`. The canonical batch records:

- requested and next report cursor;
- oldest and latest retained report ID;
- strict report-ID order;
- explicit gap before the retained window;
- whether the batch reaches the current retained high-water;
- bounded text and record counts;
- historical coverage remaining partial even when the retained suffix is complete.

Citizen pagination and announcement continuation are one transactional publication problem. The
publisher rejects drift and publishes no capsule until the entire configured combined observation is
complete.

Bootstrap acquires one complete combined capsule and wraps the source in a primed replay layer. The
adapter consumes the same capsule without a second underlying bridge read. The replay preserves the
two-dimensional request surface and fails on citizen-offset, announcement-cursor, limit, source
manifest, projection, or final snapshot drift.

The protocol-1.1 MCP runtime is development-only. It requires exact operator opt-in, uses a distinct
session namespace, refuses production admission environment state, and cannot consume a production
ticket.

## One version universe

The append unit is an immutable observation capsule. Its ordered stream drives current state,
history, graph/search updates, subscriptions, branches, replay, checkpoint cutoffs, evidence,
briefings, attention, affordance validity, and handoff anchors. Every derived generation publishes
its source high-water mark.

A target anchor is conceptually:

```text
(fortress_lineage,
 observation_epoch,
 snapshot_sequence,
 game_tick?,
 bridge_generation,
 bridge_protocol,
 adapter_epoch,
 schema_epoch,
 policy_epoch,
 semantic_world_root)
```

Restore, incompatible world switch, game-clock regression, or ambiguous bridge restart creates or
requires a new observation epoch. Heartbeats preserve semantic identity while recording liveness.

## Observation publication

```text
reserve successor
→ acquire all bounded transport pages
→ normalize and validate canonical capsule
→ materialize immutable children
→ verify semantic root
→ publish root/high-water mark atomically
→ notify derived consumers
```

Readers see the old or new root, never a partially assembled generation. Deltas require exact basis
identity and must reconstruct the successor root. `continuous`, `heartbeat`, `partial`, `gap`,
`reset`, `stale`, and `indeterminate` are protocol facts, not presentation guesses.

## Exact compatibility admission

One live tuple requires:

```text
exact clean dfmcp source
+ exact DFHack source
+ exact plugin SHA-256
+ exact bridge protocol and version
+ R1 native build and inventory
+ R2 authentication/non-disclosure
+ R3 deterministic complete reads
+ R4 restart/drift/gap fencing
+ R5 cold-agent orientation
+ generation-specific evidence such as protocol-1.1 A1-A6
→ one content-addressed experimental registry entry
```

The resolver binds the complete canonical registry digest, exact manifest, and explicitly required
entry ID. A match under another entry ID fails closed. The current registry is empty.

## Monotonic local custody

A deployment host accepts registry generations through an owner-private monotonic floor. The floor
binds exact registry file bytes, canonical registry digest, ordered entry IDs, monotonic sequence,
and previous floor digest.

```text
absolute path
+ real exact-mode 0700 parent
+ regular non-symlink exact-mode 0600 file
+ root/effective-user ownership
+ exclusive initialization
+ lock and expected-file-digest CAS
+ atomic fsynced replacement
+ prior entry-ID preservation
```

The floor prevents an older valid registry from silently replacing a newer accepted generation. It
is not compatibility evidence, distributed consensus, revocation, or hostile-root protection.

## Authority-free diagnosis

The admission doctor evaluates, in deterministic order:

```text
registry
→ compatibility floor
→ exact tuple resolution
→ optional source-bound server artifact
```

It may report `compatibility_ready` or `artifact_preflight_ready`. It does not read a bridge token,
connect to DFHack, execute a server, alter custody, or grant capabilities.

## Protocol-bound V2 process admission

The process boundary is `architecture/live_admission_ticket_v2.json`. Protocol is an authority
identity, not a late runtime option.

```text
deployment manifest protocol
→ compatibility decision protocol
→ launch record bridge_protocol
→ ticket bridge_protocol
→ DFMCP_ADMITTED_BRIDGE_PROTOCOL
→ Rust admission context/provenance
→ reviewed private runner lookup
```

Launch and ticket digests both cover the protocol. The current production map is deliberately:

```text
1.0 → dwarf-fortress-mcp serve-live → crate::live_server::run_live_stdio
```

Protocol 1.1 is `implemented_unadmitted_development_only`. Unknown protocols, 1.1 production
attempts, mismatches, and legacy V1 tickets fail before server startup. Future map widening is a
reviewed evidence-generation change, not an environment toggle.

The launcher verifies registry/floor equality, exact decision and entry, source-bound server receipt,
loader hygiene, and an already-open no-follow executable. It re-reads registry/floor and re-hashes
the descriptor before ticket issue and before descriptor-only `execve`.

The exact-mode `0600` single-use ticket lives under a real exact-mode `0700` directory and binds
process, expiry, protocol, compatibility, floor, receipt, launch, executable identity, read-only
capabilities, and empty mutation authority. The Rust process repeats semantic and executable checks,
deletes the ticket, proves absence, retains provenance, and invokes the selected runner.

## Witnessed mutation target

Future mutation remains:

```text
semantic objective / intent
→ compile against pinned anchor
→ positive, negative, range, aggregate, spatial, and epoch witnesses
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
```

No transport acknowledgement is semantic success. Unknown dispatch outcome remains indeterminate
until operation lookup and authoritative observation resolve it. No current live protocol exposes
this path.

## Semantic concurrency

Witnesses cover entity generations and fields, relation domains, tile masks, aggregates and their
contributors, absence predicates over complete domains, graph facts, resources, leases, and epochs.
Coarse witnesses are sound. Fine witnesses may prove disjointness but may never introduce a false
negative. Budget exhaustion causes conservative replan.

Rebase recompiles intent. Concurrent merge uses exact replay, stable-key structural composition,
registered commutativity, explicit ordering with re-proof, or rejection. Raw-byte and
last-writer-wins merge are forbidden.

## Owned execution

`asupersync` is the exclusive async runtime. Sessions, plans, bridge operations, obligations,
checkpoint work, graph/search builders, ATP transfers, evidence writers, briefings, standing watches,
and memory exporters are region-owned. Every effectful function carries context, authority,
deadline, multidimensional budget, cancellation, and replay identity.

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
is orthogonal to class. Empty results prove absence only with a complete-domain witness.

## Graph and cognition

Anchor-bound graph projections represent traversal, logistics, production, power/fluid/mechanisms,
social/welfare, threat/defense, plan/evidence relations, objectives, and active-work dependencies.
Algorithms declare numeric policy, tie-breaks, budgets, output order, digest, and complexity/
decision witnesses.

Search and knowledge use immutable generations activated only after verification. Progressive
refinement may stop on budget while preserving completeness status. Adaptive policy may choose
effort but cannot weaken safety, compatibility, custody, or evidence.

Cognition produces bounded attention, legal affordances, recommendations, and counterfactuals. It
never issues effect tickets.

## Source and evidence movement

ATP moves immutable object graphs: checkpoints, capsule runs, derived generations, crashpacks,
qualification receipts, handoff packets, and evidence-linked memory exports. Children verify before
the root publishes. Mutation authority never rides ATP.

Release source bundles are independently canonicalized from exact Git objects and verified without
extraction. Source custody does not imply compilation, compatibility, or runtime authority.

## Trust boundaries

### Safe-Rust domain

MCP server, semantic core, adapters, world state, graph/query engines, policy, evidence, transfer,
bridge client, and admission consumer use `unsafe_code = "forbid"`.

### Game/native domain

Dwarf Fortress and DFHack are external. Rust uses no C/C++ FFI or direct memory scraping. Current
bridge generations are bounded, authenticated, versioned, loopback-only, and read-only.

### Deployment custody domain

Registry, monotonic floor, source receipt, bridge protocol, executable descriptor, loader
environment, and ticket are separate inputs. Every transition revalidates its dependencies.

### Untrusted content domain

MCP arguments, bridge frames, in-game text, mod text, documentation, agent notes, imported memories,
and model rationales are untrusted. Text may be evidence; it cannot grant authority or become code.

## Dependency policy

The universe is `core`/`alloc`/`std`, `asupersync`, admitted Franken crates, the owned pinned
`fastmcp_rust` plane, and rare fundamental exceptions in the allowlist. There is no second async
runtime, external graph engine, C SQLite, broad web/RPC framework, non-owned MCP framework, or search
engine in the production trust domain.

## Current crate graph

```text
dfmcp-core → dfmcp-world → dfmcp-intent → dfmcp-adapter → dfmcp-lab
      └────────────────────────────────────────────→ dwarf-fortress-mcp
dfmcp-world → dfmcp-mcp → dwarf-fortress-mcp
dfmcp-mcp → fastmcp-rust (owned sibling, modern-only MCP 2026-07-28, pinned revision)
```

`dfmcp-mcp` is presentation-only and replaceable. A crate split must establish a dependency, trust,
or verification boundary.

## Non-bypassability

An implementation is invalid if any path can:

- mutate without plan, capability, idempotency, fence, and evidence checks;
- start production live mode without exact registry, floor, protocol, receipt, descriptor, and
  ticket proof;
- select a runner from an environment value not covered by launch and ticket digests;
- let an unadmitted development runtime consume production admission state;
- fall back to path execution after qualifying another inode;
- verify only metadata while executable bytes can change;
- silently roll back the accepted registry generation;
- publish mixed observation generations or bridge a cursor gap;
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
