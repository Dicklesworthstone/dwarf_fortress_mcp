# Fortress Graph Architecture and Algorithm Registry

The canonical world is graph-shaped, but “put it in a graph database” is not an architecture. This document defines the projections, algorithms, update strategy, determinism rules, and safety boundaries that make graph theory useful to agents.

## 1. Projection doctrine

Every graph projection is derived from one authoritative anchor. It declares:

- projection identifier and schema version;
- source anchor or closed anchor interval;
- vertex and edge semantics;
- completeness and authorization scope;
- weight units and numeric policy;
- update high-water mark;
- deterministic iteration order;
- root digest.

A projection can be exact, conservative, optimistic, sampled, or hypothetical. Only exact and sufficiently complete projections may support hard preconditions. Advisory projections can rank attention but cannot authorize effects.

## 2. Layered world graph

### 2.1 Identity and containment graph

Vertices represent fortress, map blocks, regions, entities, items, buildings, jobs, squads, stockpiles, zones, and abstract resources. Edges encode containment, ownership, membership, location, and generation-safe references.

### 2.2 Traversal graph

Vertices are walkable or traversable spatial cells/portals. Edge predicates include movement mode, door/bridge state, traffic cost, fluid depth, temperature hazard, burrow policy, unit size, and temporal availability. This is not one graph: it is a family parameterized by traveler class and anchor.

### 2.3 Logistics graph

Edges connect sources, sinks, stockpiles, workshops, hauling jobs, and transformations. Capacities and costs represent quantity, distance, labor, container constraints, reservation, and spoilage.

### 2.4 Production dependency graph

Vertices are requirements, reactions, workshops, materials, jobs, and milestones. Directed edges express prerequisites and outputs. Hyperedge-like n-ary facts are reified as operation vertices so provenance and constraints remain explicit.

### 2.5 Power, fluid, and mechanism graph

This projection represents mechanical power, pressure/flow paths, pumps, axles, gears, mechanisms, gates, and stateful control links. It must preserve direction, capacity, and activation semantics; generic unweighted connectivity is insufficient.

### 2.6 Social and welfare graph

Vertices are units, relationships, roles, needs, locations, and events. Edges are provenance-rich and privacy/capability scoped. Centrality is advisory; it must not turn uncertain social inference into fact.

### 2.7 Threat and defense graph

This graph combines hostile reachability, line-of-approach, chokepoints, refuge reachability, squad placement, doors/bridges, and escape routes. Safety results require explicit traveler and threat models and are invalidated aggressively.

### 2.8 Plan, obligation, and effect graph

Vertices are intents, steps, leases, effects, observations, predicates, evidence, and obligations. Edges encode dependency, ownership, support, contradiction, compensation, and causal attribution. This graph is the primary explanation substrate.

## 3. Stable graph identity

Dense ordinals may be used inside a projection for performance, but external identity is never replaced by an ordinal. Each projection has a bijection table:

```text
StableEntityId + generation ↔ ProjectionOrdinal
```

The table is immutable within a generation. Repacking creates a new projection generation and certificate. Edges are keyed canonically; multiedges retain semantic keys.

## 4. Storage temperatures

Adjacency uses three temperatures:

1. **Inline micro-adjacency** for tiny neighborhoods, avoiding heap-heavy general structures.
2. **Sorted delta blocks** for recent changes, with anchor interval and tombstones.
3. **Sealed compressed runs** for stable cold adjacency, with restart points and checksums.

A view merges the small number of relevant runs in deterministic order. Compaction publishes a new root and retains the old generation for pinned readers. The initial reference implementation may use ordered maps; the temperature design is an optimization target, not an excuse to skip the oracle.

## 5. Canonical Graph Semantics for fortress decisions

Every algorithm invocation selects a closed tie-break policy. Typical policies are:

- stable entity identity ascending;
- insertion order from the canonical observation capsule;
- cost then stable identity;
- cost then insertion sequence;
- risk descending then stable identity;
- plan-step ordinal;
- edge semantic key.

The policy is part of cache identity and the result certificate. Algorithms may not inherit randomized hash order. Randomized algorithms require an explicit seed and declare probabilistic status.

## 6. Algorithm families

### 6.1 Dynamic connectivity

**Questions:** Are required areas connected? Did construction partition the fortress? Can a dwarf, item, cart, fluid, or invader traverse between endpoints?

Use a reference BFS/DFS first. Add incremental connectivity only when update rates justify it. Deletions are the hard case; the design may use bounded recomputation for affected regions before adopting a dynamic forest structure. Connectivity results are traveler-class-specific.

**Invalidation:** any edge state, movement policy, occupancy rule, or relevant tile change.

### 6.2 Bridges, articulation points, and biconnected components

**Questions:** Which corridor, stair, ramp, bridge, door, or power edge is a single point of failure? Which areas lack redundant evacuation or hauling routes?

Results feed risk scoring and plan constraints. An articulation point is not automatically “bad”; it becomes actionable only when the affected components contain protected demand or hazards.

### 6.3 Strongly connected components and condensation

**Questions:** Which dependency, job, reservation, lease, or control relationships form cycles? Can a production chain deadlock? Does a mechanism control loop have unintended feedback?

The condensation DAG supports topological planning. SCC identity is deterministic by minimum stable vertex identity, and component output order follows the condensation order plus tie-break policy.

### 6.4 Dominators

**Questions:** Which node lies on every path from a source to an objective? Which workshop, stockpile, stairwell, bridge, mechanism, or plan step dominates success?

Dominators are often more operationally useful than generic centrality. They identify unavoidable dependencies. The projection must define source and path semantics precisely.

### 6.5 Shortest paths and alternatives

Supported families include unweighted BFS, Dijkstra for nonnegative costs, A* with admissible registered heuristics, Bellman-Ford only where negative weights are semantically legitimate, and k-shortest simple paths for fallback planning.

Path cost is a vector before scalarization:

```text
(distance, danger, congestion, labor, door_state_risk, uncertainty)
```

A policy names the scalarization or Pareto rule. Silent float comparison is forbidden; fixed-point or checked integer units are preferred. Equal-cost frontier ordering is explicit.

### 6.6 Multi-source distance and Voronoi assignment

**Questions:** Which hospital, stockpile, dining hall, barracks, refuge, water source, or workshop best serves each demand point? Where are service deserts?

Multi-source search produces both distance and winning-source witnesses. Ties follow stable source identity or policy priority.

### 6.7 Maximum flow and minimum cut

**Questions:** What is the throughput capacity of a hauling, fluid, power, evacuation, or defensive network? Which smallest cut disconnects a protected region from a threat or resource?

Capacities are semantic and unit-checked. A min-cut result is advisory until mapped back to buildable/controllable game objects and revalidated. Infinite capacities are represented explicitly, not as a magic large integer.

### 6.8 Minimum-cost flow

**Questions:** How can bounded supply satisfy distributed demand while minimizing hauling, labor, danger, or delay?

This is useful for planning allocations, not issuing individual jobs blindly. Costs and capacities come from an anchor; the output becomes a candidate intent set. Integer algorithms and checked arithmetic are preferred for deterministic replay.

### 6.9 Matching and assignment

Bipartite or weighted matching supports dwarf-to-job, bed-to-patient, squad-to-position, workshop-to-order, and vehicle-to-route candidates. Feasibility filters are applied before optimization. The objective is lexicographic unless a versioned scalarization policy says otherwise:

1. satisfy hard eligibility;
2. minimize lethal or severe risk;
3. maximize critical completion;
4. respect skill and preference;
5. minimize travel and disruption;
6. stable tie-break.

Matching never overwrites the game’s autonomous labor system by default. It suggests high-level settings or guarded interventions.

### 6.10 Spanning structures

Minimum spanning forests can propose low-cost connection skeletons for roads, power, or access. They are poor substitutes for resilience because a tree has no redundancy. Plans that need fault tolerance use constrained augmentation or multiple disjoint paths.

### 6.11 DAG scheduling and critical path

The plan graph is a DAG after SCC rejection or explicit cycle handling. Algorithms compute topological order, earliest start, critical path, slack, and parallel fronts. Resource conflicts add edges or cumulative constraints. The scheduler emits a proof of dependency satisfaction and names the obligation controlling terminal completion.

### 6.12 Centrality and community

PageRank, betweenness, harmonic/closeness, HITS, and community methods may rank observation attention, likely bottlenecks, or socially significant units. They are never safety proofs. Approximate results declare seed, sample policy, error evidence, and staleness.

### 6.13 Temporal and causal reachability

The plan/evidence graph is temporal. “Could effect X have caused event Y?” requires edges valid in compatible intervals and a monotone causal order. Queries return supporting paths and missing-link uncertainty. Correlation edges are distinct from causal edges.

### 6.14 Subgraph isomorphism and motifs

Small registered motifs may detect known failure shapes: circular stockpile chains, inaccessible workshops, unsafe airlock patterns, or repeated military traps. General subgraph isomorphism is budgeted and cannot run unbounded on live requests.

## 7. Incremental maintenance

Observation capsules are translated into graph deltas. Each projection declares whether it supports:

- exact incremental update;
- incremental candidate update plus periodic rebuild;
- full rebuild only.

Incremental and full builds are different implementations of the same pure function. Differential tests compare roots. When a delta cannot be applied safely, the generation becomes stale and rebuilds; it does not continue with silent drift.

Standing analyses maintain Z-set-like weighted changes where useful. Counts and joins retract as facts disappear. Recursion is stratified and bounded by a declared policy.

## 8. Query execution

Graph queries use factorized intermediates. A query such as “all blocked workshops reachable through missing-input chains” retains shared prefixes rather than materializing every path. Join planning can choose binary, multiway, or traversal operators, but all must preserve output semantics and canonical order.

Memory budgets are explicit. Operators spill only through a registered deterministic format. A query that cannot complete within budget returns a continuation or bounded partial result with completeness status.

## 9. Zero-copy snapshot views

Immutable projection generations are reference-counted. Cloning a view is O(1). Algorithms receive slices/iterators over stable generation-owned memory. Mutable builders are never shared with readers. Publication swaps the root; old views retain old storage until pins release.

Copy avoidance is accepted only after measurement. Small result materialization may be cheaper than a complex view. The project measures the actual wall before creating specialized storage.

## 10. Complexity and decision witnesses

For planning-relevant algorithms, a witness records:

```text
algorithm and implementation version
projection root and anchor
input n/m and filtered domain
weight and numeric policy
canonical tie-break policy
seed, if any
observed operation counters
budget and cancellation state
result digest
decision-path digest
```

A complexity regression gate compares observed counters before wall-clock performance. This distinguishes algorithmic drift from machine noise.

## 11. Capability noninterference

Authorization filters vertices and edges before expansion. Derived degree, component size, path existence, count, score, and absence are computed only on the authorized projection. Cache keys include authorization-scope digest. A result certificate names the scope.

The system tests paired worlds that differ only in unauthorized data; authorized outputs must be identical.

## 12. Algorithm admission template

A new algorithm is admitted only with:

- semantic projection specification;
- reference implementation;
- tie-break and numeric policy;
- complexity bound;
- cancellation points and budgets;
- adversarial fixtures;
- differential oracle;
- incremental invalidation rules;
- capability noninterference test;
- result certificate schema;
- measured reason it is useful to an agent.
