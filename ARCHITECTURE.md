# Architecture

This is the compact engineering map. The comprehensive plan and machine registries are normative when details differ.

## Thesis

`dwarf_fortress_mcp` is a safe-Rust, latest-nightly semantic operating substrate for agents stewarding a Dwarf Fortress simulation. MCP is the narrow presentation waist. The authoritative state is a multi-version, provenance-carrying world history. Graph/search/knowledge systems are certified derived cognition. DFHack is a fenced out-of-process effect boundary.

## Three planes

```text
┌──────────────────────────────────────────────────────────────────────┐
│ MCP · sessions · capabilities · budgets · continuations             │
└──────────────────────────────────────────────────────────────────────┘
                 │ reads / intents                   │ evidence
                 ▼                                   ▲
┌──────────────────────────────────────────────────────────────────────┐
│ AUTHORITATIVE PLANE                                                  │
│ observation capsules · MVCC world · witnesses · intents/plans       │
│ idempotency · leases/fences · effects · obligations · checkpoints   │
│ evidence · compatibility/schema/policy epochs                        │
└──────────────────────────────────────────────────────────────────────┘
        │ pinned immutable generations            │ short-lived ticket
        ▼                                         ▼
┌──────────────────────────────────┐  ┌────────────────────────────────┐
│ COGNITION PLANE                  │  │ EFFECT PLANE                   │
│ graph projections and algorithms│  │ compatibility probes           │
│ search/knowledge generations     │  │ bounded reads                  │
│ attention and counterfactuals    │  │ game-thread preconditions      │
│ candidate intents only           │  │ typed effects and lookup       │
└──────────────────────────────────┘  │ observe and reconcile          │
                                      └────────────────────────────────┘
```

The cognition plane has no path to effect dispatch. The effect plane cannot redefine canonical identity or success.

## One version universe

The append unit is an immutable observation capsule. The ordered capsule stream drives current state, history, graph/search updates, subscriptions, branches, replay, checkpoint cutoffs, replicas, and evidence references. Every derived generation publishes its source high-water mark.

A version-2 anchor is conceptually:

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

A request reads one complete anchor. Restore, incompatible reload, or ambiguous bridge restart creates a new observation epoch.

## Observation publication

```text
reserve successor
→ normalize and validate capsule
→ materialize immutable children
→ verify semantic root
→ publish root/high-water mark atomically
→ notify derived consumers
```

Readers see the old or new root, never a partially assembled generation. Deltas require exact basis identity and reconstruct the successor root.

## Witnessed mutation

```text
semantic intent
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
```

No transport acknowledgment is semantic success. Unknown dispatch outcome remains indeterminate until operation lookup and observation resolve it.

## Semantic concurrency

Witnesses cover entity generations and fields, relation domains, tile masks, aggregates and their contributors, absence predicates over complete domains, graph facts, resource reservations, leases, and epochs. Coarse witnesses are sound. Optional fine witnesses may prove disjointness. Exhaustion may create a false conflict but can never authorize an unsafe overlap.

Rebase recompiles intent. Concurrent merge uses exact replay, stable-key structural composition, registered commutativity, explicit ordering with re-proof, or rejection. Raw-byte and last-writer-wins merge are forbidden.

## Owned execution

`asupersync` is the exclusive async runtime. Sessions, plans, bridge operations, obligations, checkpoint work, graph/search builders, ATP transfers, and evidence writers are region-owned. Every effectful function receives context-carried authority, deadlines, multidimensional budgets, cancellation, and replay identity.

Cancellation is request, drain, reconcile/compensate, finalize. Long drains produce progress certificates. Session close cannot silently abandon owned work.

## Graph and cognition

Anchor-bound graph projections represent traversal, logistics, production, power/fluid/mechanisms, social/welfare, threat/defense, and plan/evidence relations. Planning algorithms have explicit numeric and tie-break policy, bounded resources, canonical output order, output digest, and decision-path/complexity witness.

Search and knowledge use immutable generations activated only after verification. Progressive refinement can stop on budget while preserving completeness status. Adaptive policies can choose effort but cannot weaken safety or evidence.

## State and evidence movement

ATP moves immutable object graphs such as checkpoints, capsule runs, derived generations, crashpacks, and qualification receipts. Manifests and children verify before the root publishes. Transfers can resume, race paths, and use repair symbols. Mutation authority never rides ATP.

## Trust boundaries

### Safe-Rust trust domain

The MCP server, semantic core, runtime adapters, world MVCC, graph/query engines, policy, evidence, transfer, and bridge client use `unsafe_code = "forbid"`.

### Game/native domain

Dwarf Fortress and DFHack are external. The Rust process uses no C/C++ FFI or direct memory scraping. A bounded versioned loopback bridge exposes only typed reads, typed effects, operation lookup, and compatibility probes.

### Untrusted content domain

MCP arguments, bridge frames, in-game text, mod text, documentation, and agent notes are untrusted. Text can provide evidence; it cannot grant authority or become executable code.

## Reference before optimization

The first implementation for every semantic subsystem is deliberately simple and deterministic. Optimized Franken adapters are admitted only after differential, crash, cancellation, compatibility, and performance gates. Derived state can always be discarded and rebuilt from authoritative capsules.

## Dependency policy

The universe is `core`/`alloc`/`std`, `asupersync`, admitted Franken crates, and rare fundamental exceptions named in the machine allowlist. There is no second async runtime, external graph engine, SQL client, C SQLite, web framework, RPC framework, or search engine in the production trust domain.

## Current and target crate graphs

The six-crate phase-zero scaffold remains executable and intentionally small:

```text
dfmcp-core → dfmcp-world → dfmcp-intent → dfmcp-adapter → dfmcp-lab
      └──────────────────────────────────────────────→ dwarf-fortress-mcp
```

The target decomposition is specified in Part XXI of the comprehensive plan. A crate is split only to establish a dependency, trust, or verification boundary.

## Non-bypassability

An implementation is invalid if any path can mutate without plan/idempotency/capability/fence checks; verify from a receipt alone; read mixed generations; apply a delta across a gap; ignore a negative-read phantom; retry an indeterminate effect blindly; execute arbitrary text; publish a root before its children; restore without a new epoch; use adaptive policy to weaken safety; or close a region while owned work is silently abandoned.
