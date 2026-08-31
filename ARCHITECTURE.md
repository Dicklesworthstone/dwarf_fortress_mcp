# Architecture

This is the compact engineering map. The comprehensive plan and machine registries are normative when details differ.

## Thesis

`dwarf_fortress_mcp` is a safe-Rust, latest-nightly semantic operating substrate for agents stewarding a Dwarf Fortress simulation. MCP is the narrow presentation waist. The authoritative state is a multi-version, provenance-carrying world history. Graph/search/knowledge systems are certified derived cognition. DFHack is a fenced out-of-process effect boundary.

The system has one agent-facing center: the control loop defined in
`docs/AGENT_OPERATING_MODEL.md`. Every plane and crate exists to make one or more stages of that
loop more truthful, economical, safe, replayable, or accretive. No subsystem may expose a competing
lifecycle or force the agent to reconstruct protocol state from unrelated outputs.

## Agent control loop

```text
bootstrap → orient → inspect → formulate → propose → compare
          → commit → verify → learn → handoff/resume
                         ↘ reconcile ↗
```

Every success and error response converges on a canonical Agent Turn Packet with:

```text
identity + exact anchor + continuity
briefing + semantic changes + ranked attention
active work + legal affordances + ranked next protocol steps
uncertainty + coverage + budget + typed references
```

The packet is an orientation spine, not a second source of truth. Briefing, attention,
affordances, recommendations, counterfactuals, and memory are derived views. They can propose an
intent but cannot authorize or dispatch an effect. The machine contract is
`architecture/agent_turn_contract.json`.

## Three planes

```text
┌──────────────────────────────────────────────────────────────────────┐
│ MCP · sessions · capabilities · budgets · continuations             │
│ Agent Turn Packet · profiles · affordances · recommendations        │
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
│ affordance/recommendation views  │  │ typed effects and lookup       │
│ candidate intents only           │  │ observe and reconcile          │
└──────────────────────────────────┘  └────────────────────────────────┘
```

The cognition plane has no path to effect dispatch. The effect plane cannot redefine canonical identity or success.

Outside the three planes sits **agent campaign memory** (the owned `eidetic_engine_cli` sibling,
`docs/EIDETIC_MEMORY.md`): advisory, speculative context for the humans/agents operating the
fortress. It has no path into canonical state or authority — memory cites anchors and evidence
digests; it is never cited by them.

## One version universe

The append unit is an immutable observation capsule. The ordered capsule stream drives current state, history, graph/search updates, subscriptions, branches, replay, checkpoint cutoffs, replicas, evidence references, agent briefings, attention projections, affordance validity, and handoff anchors. Every derived generation publishes its source high-water mark.

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

Agent continuity is computed from the same publication chain. `continuous`, `heartbeat`, `partial`,
`gap`, `reset`, `stale`, and `indeterminate` are protocol facts, not presentation guesses.

## Witnessed mutation

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

No transport acknowledgment is semantic success. Unknown dispatch outcome remains indeterminate until operation lookup and observation resolve it.

## Semantic concurrency

Witnesses cover entity generations and fields, relation domains, tile masks, aggregates and their contributors, absence predicates over complete domains, graph facts, resource reservations, leases, and epochs. Coarse witnesses are sound. Optional fine witnesses may prove disjointness. Exhaustion may create a false conflict but can never authorize an unsafe overlap.

Rebase recompiles intent. Concurrent merge uses exact replay, stable-key structural composition, registered commutativity, explicit ordering with re-proof, or rejection. Raw-byte and last-writer-wins merge are forbidden.

## Owned execution

`asupersync` is the exclusive async runtime. Sessions, plans, bridge operations, obligations, checkpoint work, graph/search builders, ATP transfers, evidence writers, briefing builders, standing watches, and memory exporters are region-owned. Every effectful function receives context-carried authority, deadlines, multidimensional budgets, cancellation, and replay identity.

Cancellation is request, drain, reconcile/compensate, finalize. Long drains produce progress certificates. Session close cannot silently abandon owned work.

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
is orthogonal to class: a high-confidence prediction is still not an observation. Empty query
results prove absence only when accompanied by a complete-domain coverage witness.

## Graph and cognition

Anchor-bound graph projections represent traversal, logistics, production, power/fluid/mechanisms, social/welfare, threat/defense, plan/evidence relations, objectives, and active-work dependencies. Planning algorithms have explicit numeric and tie-break policy, bounded resources, canonical output order, output digest, and decision-path/complexity witness.

Search and knowledge use immutable generations activated only after verification. Progressive refinement can stop on budget while preserving completeness status. Adaptive policies can choose effort but cannot weaken safety or evidence.

The cognition plane additionally produces three bounded projections:

- **attention:** what matters now, with score ledger, consequence, expiry, and evidence;
- **affordances:** what semantic actions are currently expressible under state, capability, risk,
  and compatibility constraints;
- **recommendations:** ranked informational or control protocol steps with utility,
  value-of-information, cost, risk, reversibility, invalidators, and confirmation requirements.

These projections share one anchor and deterministic tie-break policy. They never issue effect
tickets.

## Objectives, counterfactuals, and accretion

An objective is distinct from an intent, plan, action, obligation, or memory item. It declares
desired predicates, hard constraints, soft utility, horizon, priority, ownership, and evidence
requirements. Decomposition is a deterministic, inspectable artifact.

Counterfactual branches are immutable structurally shared derived worlds. They can compare plan
candidates but cannot become canonical state or mutation authority.

Execution records predicted and observed deltas. Material divergence emits a surprise record. Raw
episodes may be curated into semantic, procedural, policy, or negative memory only through the
evidence-gated promotion ladder. Memory cites canonical anchors; canonical state never cites memory
as authority.

## State and evidence movement

ATP moves immutable object graphs such as checkpoints, capsule runs, derived generations, crashpacks, qualification receipts, handoff packets, and evidence-linked memory exports. Manifests and children verify before the root publishes. Transfers can resume, race paths, and use repair symbols. Mutation authority never rides ATP.

## Trust boundaries

### Safe-Rust trust domain

The MCP server, semantic core, runtime adapters, world MVCC, graph/query engines, policy, evidence, transfer, and bridge client use `unsafe_code = "forbid"`.

### Game/native domain

Dwarf Fortress and DFHack are external. The Rust process uses no C/C++ FFI or direct memory scraping. A bounded versioned loopback bridge exposes only typed reads, typed effects, operation lookup, and compatibility probes.

### Untrusted content domain

MCP arguments, bridge frames, in-game text, mod text, documentation, agent notes, imported memories,
and model-generated rationales are untrusted. Text can provide evidence; it cannot grant authority or become executable code.

## Reference before optimization

The first implementation for every semantic subsystem is deliberately simple and deterministic. Optimized Franken adapters are admitted only after differential, crash, cancellation, compatibility, agent-turn, and performance gates. Derived state can always be discarded and rebuilt from authoritative capsules.

## Dependency policy

The universe is `core`/`alloc`/`std`, `asupersync`, admitted Franken crates, the owned
`fastmcp_rust` MCP plane (modern-only MCP 2026-07-28, pinned; ADR-013), and rare fundamental
exceptions named in the machine allowlist. There is no second async runtime, external graph
engine, SQL client, C SQLite, web framework, RPC framework, non-owned MCP framework, or search
engine in the production trust domain.

## Current and target crate graphs

The seven-crate scaffold plus the pinned transport:

```text
dfmcp-core → dfmcp-world → dfmcp-intent → dfmcp-adapter → dfmcp-lab
      └────────────────────────────────────────────→ dwarf-fortress-mcp
dfmcp-world → dfmcp-mcp → dwarf-fortress-mcp
dfmcp-mcp → fastmcp-rust (owned sibling, modern-only MCP 2026-07-28, pinned rev)
```

`dfmcp-mcp` is presentation-only and replaceable; the target decomposition is specified in
Part XXI of the comprehensive plan, and the transport contract in
`docs/FASTMCP_INTEGRATION.md`. A crate is split only to establish a dependency, trust, or
verification boundary. The Agent Turn Packet belongs at the presentation seam, while its source
facts and decisions remain typed in the lower semantic crates.

## Non-bypassability

An implementation is invalid if any path can mutate without plan/idempotency/capability/fence checks; verify from a receipt alone; read mixed generations; apply a delta across a gap; ignore a negative-read phantom; retry an indeterminate effect blindly; execute arbitrary text; publish a root before its children; restore without a new epoch; use adaptive policy to weaken safety; use memory or recommendations as authority; satisfy a precondition from inference or prediction; prove absence from incomplete coverage; hide active work across an agent handoff; or close a region while owned work is silently abandoned.
